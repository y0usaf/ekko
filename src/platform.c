#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <poll.h>
#include <pty.h>
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>
#include <utmp.h>
#include <zlib.h>

static int result(int n) { return n < 0 ? -errno : n; }
int ek_close(int fd) { return result(close(fd)); }
int ek_nonblock(int fd) {
    int flags = fcntl(fd, F_GETFL);
    if (flags < 0) return -errno;
    return result(fcntl(fd, F_SETFL, flags | O_NONBLOCK));
}
int ek_read(int fd, void *data, int size) { return result(read(fd, data, size)); }
int ek_write(int fd, const void *data, int size) { return result(write(fd, data, size)); }
int ek_poll(int *fds, int *events, int count, int timeout) {
    if (count < 0 || count > 128) return -EINVAL;
    struct pollfd items[128];
    for (int i = 0; i < count; i++) items[i] = (struct pollfd){fds[i], events[i], 0};
    int n = poll(items, count, timeout);
    if (n < 0) return -errno;
    for (int i = 0; i < count; i++) events[i] = items[i].revents;
    return n;
}
int ek_size(int fd, int *values) {
    struct winsize ws;
    if (ioctl(fd, TIOCGWINSZ, &ws) < 0) return -errno;
    values[0] = ws.ws_col; values[1] = ws.ws_row;
    values[2] = ws.ws_xpixel; values[3] = ws.ws_ypixel;
    return 0;
}
int ek_resize(int fd, int cols, int rows, int x, int y) {
    struct winsize ws = {rows, cols, x, y};
    return result(ioctl(fd, TIOCSWINSZ, &ws));
}
static struct termios saved;
static int saved_fd = -1, saved_flags, saved_output_flags;
int ek_raw(int fd) {
    if (saved_fd >= 0) return -EBUSY;
    if (tcgetattr(fd, &saved) < 0) return -errno;
    saved_flags = fcntl(fd, F_GETFL);
    saved_output_flags = fcntl(STDOUT_FILENO, F_GETFL);
    struct termios raw = saved;
    cfmakeraw(&raw);
    if (tcsetattr(fd, TCSANOW, &raw) < 0) return -errno;
    saved_fd = fd;
    ek_nonblock(STDOUT_FILENO);
    return ek_nonblock(fd);
}
int ek_restore(void) {
    if (saved_fd < 0) return 0;
    int n = tcsetattr(saved_fd, TCSANOW, &saved);
    fcntl(saved_fd, F_SETFL, saved_flags);
    fcntl(STDOUT_FILENO, F_SETFL, saved_output_flags);
    saved_fd = -1;
    return result(n);
}
int ek_spawn(char **argv, int cols, int rows, int x, int y, int *pid_out) {
    int master, slave, errors[2];
    struct winsize ws = {rows, cols, x, y};
    if (openpty(&master, &slave, NULL, NULL, &ws) < 0) return -errno;
    if (pipe2(errors, O_CLOEXEC) < 0) {
        int e = errno; close(master); close(slave); return -e;
    }
    pid_t pid = fork();
    if (pid == 0) {
        close(errors[0]); close(master);
        if (login_tty(slave) < 0) goto failed;
        if (errors[1] != 3) {
            if (dup3(errors[1], 3, O_CLOEXEC) < 0) goto failed;
            close(errors[1]); errors[1] = 3;
        }
        close_range(4, UINT_MAX, 0);
        sigset_t mask;
        sigemptyset(&mask);
        sigprocmask(SIG_SETMASK, &mask, NULL);
        for (int s = 1; s < NSIG; s++) signal(s, SIG_DFL);
        execvp(argv[0], argv);
failed:;
        int e = errno;
        (void)!write(errors[1], &e, sizeof(e));
        _exit(127);
    }
    int e = errno;
    close(slave); close(errors[1]);
    if (pid < 0) { close(master); close(errors[0]); return -e; }
    int child_error = 0;
    ssize_t n;
    do { n = read(errors[0], &child_error, sizeof(child_error)); } while (n < 0 && errno == EINTR);
    close(errors[0]);
    if (n != 0) {
        close(master);
        while (waitpid(pid, NULL, 0) < 0 && errno == EINTR) {}
        return -(child_error ? child_error : EIO);
    }
    fcntl(master, F_SETFD, FD_CLOEXEC);
    ek_nonblock(master);
    *pid_out = pid;
    return master;
}
int ek_reap(int pid) {
    int status;
    int n = waitpid(pid, &status, WNOHANG);
    if (n < 0) return -errno;
    if (!n) return -EAGAIN;
    return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}
int ek_signal_group(int pid, int sig) { return result(kill(-pid, sig)); }
void ek_server_signals(void) { signal(SIGCHLD, SIG_DFL); signal(SIGPIPE, SIG_IGN); }
void ek_ignore_pipe(void) { signal(SIGPIPE, SIG_IGN); }
int ek_lock(const char *path) {
    int fd = open(path, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (fd < 0) return -errno;
    if (flock(fd, LOCK_EX | LOCK_NB) < 0) { int e = errno; close(fd); return -e; }
    return fd;
}
static int address(struct sockaddr_un *addr, const char *path) {
    if (strlen(path) >= sizeof(addr->sun_path)) return -ENAMETOOLONG;
    memset(addr, 0, sizeof(*addr)); addr->sun_family = AF_UNIX;
    strcpy(addr->sun_path, path);
    return 0;
}
int ek_listen(const char *path) {
    struct sockaddr_un addr;
    int n = address(&addr, path);
    if (n < 0) return n;
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0);
    if (fd < 0) return -errno;
    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0 || listen(fd, 8) < 0) {
        int e = errno; close(fd); return -e;
    }
    chmod(path, 0600);
    return fd;
}
int ek_connect(const char *path) {
    struct sockaddr_un addr;
    int n = address(&addr, path);
    if (n < 0) return n;
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -errno;
    if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        int e = errno; close(fd); return -e;
    }
    ek_nonblock(fd);
    return fd;
}
int ek_accept(int listener) {
    int fd = accept4(listener, NULL, NULL, SOCK_NONBLOCK | SOCK_CLOEXEC);
    if (fd < 0) return -errno;
    struct ucred peer;
    socklen_t size = sizeof(peer);
    if (getsockopt(fd, SOL_SOCKET, SO_PEERCRED, &peer, &size) < 0 || peer.uid != getuid()) {
        close(fd); return -EACCES;
    }
    return fd;
}
int ek_inflate(const void *input, int size, void *output, int capacity) {
    uLongf length = capacity;
    uLong consumed = size;
    int n = uncompress2(output, &length, input, &consumed);
    return n == Z_OK && consumed == (uLong)size ? (int)length : -EINVAL;
}
int ek_deflate(const void *input, int size, void *output, int capacity) {
    uLongf length = capacity;
    return compress2(output, &length, input, size, 1) == Z_OK ? (int)length : -EINVAL;
}

/* Take ownership of a POSIX graphics transfer, never keep a producer's
 * reusable buffer as retained scene state. Copy through a bounded stack buffer
 * into an exclusively created, daemon-owned regular file. */
int ek_snapshot_shm(const char *name, const char *path, int size) {
    if (name[0] != '/' || !name[1] || strchr(name + 1, '/') ||
        strlen(name) > 255 || size <= 0 || size > 32 * 1024 * 1024) return -EINVAL;
    int in = shm_open(name, O_RDONLY | O_CLOEXEC | O_NONBLOCK | O_NOFOLLOW, 0);
    if (in < 0) return -errno;
    struct stat st;
    int err = 0, out = -1;
    if (fstat(in, &st) < 0) err = errno;
    else if (!S_ISREG(st.st_mode) || st.st_uid != getuid() || st.st_size != size) err = EINVAL;
    if (err) { close(in); return -err; }
    /* Unlink before copying: a producer can immediately reuse the name without
     * changing this open object's identity. Never unlink a failed validation. */
    if (shm_unlink(name) < 0) { err = errno; close(in); return -err; }
    out = open(path, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (out < 0) { err = errno; close(in); return -err; }
    char buffer[65536];
    int remaining = size;
    while (remaining > 0 && !err) {
        ssize_t n = read(in, buffer, remaining < (int)sizeof(buffer) ? remaining : (int)sizeof(buffer));
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) { err = n < 0 ? errno : EIO; break; }
        ssize_t done = 0;
        while (done < n) {
            ssize_t written = write(out, buffer + done, n - done);
            if (written < 0 && errno == EINTR) continue;
            if (written <= 0) { err = written < 0 ? errno : EIO; break; }
            done += written;
        }
        remaining -= n;
    }
    close(in);
    if (close(out) < 0 && !err) err = errno;
    if (err) unlink(path);
    return -err;
}
