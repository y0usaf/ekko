//! PTY spawning, ported from zellij's `handle_terminal`/`handle_openpty`
//! (`zellij-server/src/os_input_output_unix.rs`).

use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;

use nix::pty::{OpenptyResult, Winsize, openpty};
use nix::sys::signal::{Signal, kill as nix_kill};
use nix::sys::termios::Termios;
use nix::unistd::Pid;

use crate::reaper::reap_child;

/// Environment variables that shouldn't leak from the ekko server process (or
/// an outer terminal multiplexer) into a freshly spawned pane, since they'd
/// otherwise confuse programs into thinking they're still inside that outer
/// session.
const DROP_ENV_VARS: &[&str] = &["EKKO_SESSION_NAME", "STY", "TMUX", "TMUX_PANE"];

static NEXT_TERMINAL_ID: AtomicU32 = AtomicU32::new(0);

/// Errors that can occur while spawning or controlling a PTY.
#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("failed to open pty: {0}")]
    OpenPty(#[source] nix::Error),
    #[error("failed to spawn child process: {0}")]
    Spawn(#[source] io::Error),
    #[error("failed to resize pty: {0}")]
    Resize(#[source] io::Error),
    #[error("nix error: {0}")]
    Nix(#[from] nix::Error),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
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

    let winsize = Winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let OpenptyResult { master, slave } =
        openpty(Some(&winsize), None::<&Termios>).map_err(PtyError::OpenPty)?;

    let slave_raw: RawFd = slave.as_raw_fd();

    let mut command = Command::new(&cmd.program);
    command.args(&cmd.args);
    if let Some(cwd) = &cmd.cwd {
        command.current_dir(cwd);
    }
    apply_environment(&mut command, &cmd.env);

    // SAFETY: `pre_exec` runs in the forked child, after `fork` but before
    // `exec`, so only async-signal-safe operations are allowed here.
    // `login_tty` and `close_open_fds` both qualify.
    unsafe {
        command.pre_exec(move || {
            if libc::login_tty(slave_raw) != 0 {
                return Err(io::Error::last_os_error());
            }
            // Close everything except stdin/stdout/stderr (which
            // `login_tty` just dup'd onto the slave) so the child doesn't
            // inherit unrelated fds from the server process.
            close_fds::close_open_fds(3, &[]);
            Ok(())
        });
    }

    let child = command.spawn().map_err(PtyError::Spawn)?;
    let child_pid = Pid::from_raw(child.id() as i32);

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
    let winsize = Winsize {
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
    nix_kill(pid, Some(Signal::SIGTERM)).map_err(PtyError::from)
}

/// Forcibly kill a process (`SIGKILL`).
pub fn force_kill(pid: Pid) -> Result<(), PtyError> {
    nix_kill(pid, Some(Signal::SIGKILL)).map_err(PtyError::from)
}
