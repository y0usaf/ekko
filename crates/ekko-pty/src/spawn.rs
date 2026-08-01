//! PTY spawning, ported from zellij's `handle_terminal`/`handle_openpty`
//! (`zellij-server/src/os_input_output_unix.rs`).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

pub type Pid = libc::pid_t;

use crate::reaper::reap_child;

/// Environment variables that shouldn't leak from the ekko server process (or
/// an outer terminal multiplexer) into a freshly spawned pane, since they'd
/// otherwise confuse programs into thinking they're still inside that outer
/// session.
const DROP_ENV_VARS: &[&str] = &["EKKO_SESSION_NAME", "STY", "TMUX", "TMUX_PANE"];

static NEXT_TERMINAL_ID: AtomicU32 = AtomicU32::new(0);

/// Errors that can occur while spawning or controlling a PTY.
#[derive(Debug)]
pub enum PtyError {
    OpenPty(io::Error),
    Spawn(io::Error),
    Resize(io::Error),
    Io(io::Error),
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenPty(error) => write!(f, "failed to open pty: {error}"),
            Self::Spawn(error) => write!(f, "failed to spawn child process: {error}"),
            Self::Resize(error) => write!(f, "failed to resize pty: {error}"),
            Self::Io(error) => write!(f, "io error: {error}"),
        }
    }
}

impl std::error::Error for PtyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OpenPty(error) => Some(error),
            Self::Spawn(error) | Self::Resize(error) | Self::Io(error) => Some(error),
        }
    }
}

impl From<io::Error> for PtyError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Requested initial size of a PTY, in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinSize {
    pub cols: u16,
    pub rows: u16,
}

/// A command to run inside a freshly allocated PTY.
#[derive(Debug, Clone, Default)]
pub struct PtyCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Extra environment variables to set, applied after the default
    /// TERM/COLORTERM handling (see [`apply_environment`]).
    pub env: Vec<(String, String)>,
}

impl PtyCommand {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
}

/// A live PTY and the process running inside it.
pub struct PtyHandle {
    /// The PTY master file descriptor. Left in blocking mode; use
    /// [`set_nonblocking`](crate::set_nonblocking) if the caller (e.g. an
    /// async server) needs non-blocking semantics.
    pub master_fd: OwnedFd,
    pub child_pid: Pid,
    pub terminal_id: u32,
}

/// Apply the standard ekko PTY environment to `command`: drop variables that
/// would leak outer-session state, force a sane `TERM`, pass `COLORTERM`
/// through if the server itself has one, then apply any caller-supplied
/// overrides last.
fn apply_environment(command: &mut Command, extra: &[(String, String)]) {
    for key in DROP_ENV_VARS {
        command.env_remove(key);
    }
    command.env("TERM", "xterm-256color");
    if let Ok(colorterm) = std::env::var("COLORTERM") {
        command.env("COLORTERM", colorterm);
    }
    for (key, value) in extra {
        command.env(key, value);
    }
}

/// Open a new PTY and spawn `cmd` attached to its slave side.
///
/// `on_exit` is invoked exactly once, on a dedicated reaper thread, after
/// the child has fully exited and been waited on (no zombie survives).
pub fn spawn_pty(
    cmd: PtyCommand,
    size: WinSize,
    on_exit: Box<dyn FnOnce(Option<i32>) + Send>,
) -> Result<PtyHandle, PtyError> {
    let terminal_id = NEXT_TERMINAL_ID.fetch_add(1, Ordering::Relaxed);

    let winsize = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let mut master_raw = -1;
    let mut slave_raw = -1;
    // SAFETY: all pointers are valid and openpty initializes both outputs.
    let result = unsafe {
        libc::openpty(
            &mut master_raw,
            &mut slave_raw,
            std::ptr::null_mut(),
            std::ptr::null(),
            &winsize,
        )
    };
    if result != 0 {
        return Err(PtyError::OpenPty(io::Error::last_os_error()));
    }
    // SAFETY: openpty returned ownership of these newly opened descriptors.
    let master = unsafe { OwnedFd::from_raw_fd(master_raw) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave_raw) };

    let slave_raw: RawFd = slave.as_raw_fd();
    let fd_limit = unsafe {
        let mut limits = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) == 0 {
            limits.rlim_cur.min(65_536) as RawFd
        } else {
            65_536
        }
    };

    let mut command = Command::new(&cmd.program);
    command.args(&cmd.args);
    if let Some(cwd) = &cmd.cwd {
        command.current_dir(cwd);
    }
    apply_environment(&mut command, &cmd.env);

    // SAFETY: `pre_exec` runs in the forked child, after `fork` but before
    // `exec`, so only async-signal-safe operations are allowed here.
    // `login_tty` and the raw fd operations below qualify.
    unsafe {
        command.pre_exec(move || {
            if libc::login_tty(slave_raw) != 0 {
                return Err(io::Error::last_os_error());
            }
            // Close everything except stdin/stdout/stderr (which
            // `login_tty` just dup'd onto the slave) so the child doesn't
            // inherit unrelated fds from the server process.
            // This closure allocates no memory and takes no locks: login_tty,
            // syscall, close, and the integer loop are async-signal-safe.
            // Fall back on every close_range failure: seccomp and invalid or
            // unsupported arguments must not leak the server's open fds.
            let result = libc::syscall(libc::SYS_close_range, 3, u32::MAX as libc::c_ulong, 0);
            if result == -1 {
                for fd in 3..fd_limit {
                    libc::close(fd);
                }
            }
            Ok(())
        });
    }

    let child = command.spawn().map_err(PtyError::Spawn)?;
    let child_pid = child.id() as libc::pid_t;

    // The child now owns its own copy of the slave side (dup'd onto fds
    // 0/1/2 by login_tty); the parent's copy is no longer needed.
    drop(slave);

    thread::spawn(move || {
        let exit_code = reap_child(child);
        on_exit(exit_code);
    });

    Ok(PtyHandle {
        master_fd: master,
        child_pid,
        terminal_id,
    })
}

/// Resize a live PTY via `TIOCSWINSZ`.
pub fn resize(fd: RawFd, cols: u16, rows: u16) -> Result<(), PtyError> {
    let winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // TIOCSWINSZ is a u32 on Linux but the ioctl request parameter is u64 on
    // some platforms; `.into()` bridges the two (and is a no-op where they
    // already match).
    #[allow(clippy::useless_conversion)]
    let ret = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ.into(), &winsize) };
    if ret != 0 {
        return Err(PtyError::Resize(io::Error::last_os_error()));
    }
    Ok(())
}

/// Politely ask a process to exit (`SIGTERM`).
pub fn kill(pid: Pid) -> Result<(), PtyError> {
    signal(pid, libc::SIGTERM)
}

/// Forcibly kill a process (`SIGKILL`).
pub fn force_kill(pid: Pid) -> Result<(), PtyError> {
    signal(pid, libc::SIGKILL)
}

/// Terminal-hangup semantics (`SIGHUP`), matching zellij's pane teardown: a
/// job-control shell forwards the HUP to its jobs, so the whole foreground
/// tree dies rather than just the shell.
pub fn hangup(pid: Pid) -> Result<(), PtyError> {
    signal(pid, libc::SIGHUP)
}

/// Guarantee a pane child is dead: `SIGHUP`, wait up to `grace` for it to
/// exit, then escalate to `SIGKILL` and wait up to `settle` more. Never
/// blocks longer than `grace + settle` plus one poll interval; anything
/// still alive after that (uninterruptible sleep) is logged, not chased —
/// the reaper thread still owns the final `waitpid`.
pub fn terminate(pid: Pid, grace: Duration, settle: Duration) {
    let _ = hangup(pid);
    if wait_gone(pid, grace) {
        return;
    }
    log::warn!("pty: pid {pid} ignored SIGHUP for {grace:?}; escalating to SIGKILL");
    let _ = force_kill(pid);
    if !wait_gone(pid, settle) {
        log::warn!(
            "pty: pid {pid} still present {settle:?} after SIGKILL (uninterruptible sleep?)"
        );
    }
}

/// Poll until `pid` is gone (`ESRCH`) or the budget elapses. A zombie still
/// reads as alive here, but its reaper thread collects it within one poll
/// interval, so the loop absorbs that race instead of mis-escalating.
fn signal(pid: Pid, signal: libc::c_int) -> Result<(), PtyError> {
    if unsafe { libc::kill(pid, signal) } == -1 {
        Err(PtyError::Io(io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn wait_gone(pid: Pid, budget: Duration) -> bool {
    const POLL: Duration = Duration::from_millis(10);
    let deadline = std::time::Instant::now() + budget;
    loop {
        if unsafe { libc::kill(pid, 0) } == -1
            && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}
