//! The ekko session daemon: one process per session, zellij-style.
//!
//! [`run`] binds the session's socket, spawns its shell PTY (on first
//! attach), and hosts a `vt100::Parser` that survives detach/reattach. See
//! `hub.rs` for the single-threaded state machine everything else reports
//! into.

mod broadcast;
mod client_io;
mod grid;
mod hub;
mod input_compat;
mod logging;
mod pty_io;
mod pty_writer;
mod terminal_pane;
mod topology;
mod vt_compat;

use std::os::fd::AsRawFd;
use std::thread;

use anyhow::Context;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::sync::mpsc::Sender;

use hub::{Hub, HubInstruction};

/// Build the daemon's extension runtime: builtins first (so user extensions
/// reusing a name fail loudly), filtered by `[extensions] disabled`. Only
/// scripts declaring `host = "server"` or `"both"` load here; a `"both"`
/// script gets its own Lua state per process.
fn build_runtime(config: &ekko_config::Config) -> anyhow::Result<ekko_ext::AppRuntime> {
    let builder = ekko_ext::RuntimeBuilder::new().with_disabled(&config.extensions.disabled);
    #[cfg(feature = "builtins")]
    let builder = builder.register_boxed_extensions(ekko_builtins::server_extensions());
    #[cfg(feature = "lua")]
    let builder = builder.register_boxed_extensions(ekko_lua::load_extensions(
        &ekko_config::config_dir().join("extensions"),
        ekko_lua::HostKind::Server,
        config,
    ));
    builder.build()
}

/// Run the session daemon for `session_name` until the session exits (shell
/// exit, explicit kill, a crash in a PTY thread, or a shutdown signal).
///
/// When `daemonize` is true, this forks into the background (stdout/stderr
/// redirected to `~/.cache/ekko/logs/<session_name>.log`) and only the child
/// process's call returns; the parent exits from inside daemonization.
pub fn run(session_name: &str, daemonize: bool) -> anyhow::Result<()> {
    // The config cascade (`init.lua` or defaults)
    // lives in ekko-config; the lua feature just injects the evaluator.
    // A broken `init.lua` refuses to start rather than silently running
    // on defaults.
    #[cfg(feature = "lua")]
    let config = ekko_lua::load_config_cascade()?;
    #[cfg(not(feature = "lua"))]
    let config = ekko_config::Config::load_cascade(None)?;
    let runtime = build_runtime(&config).context("building extension runtime")?;
    run_with_runtime(session_name, daemonize, config, runtime)
}

/// [`run`] with an explicit extension runtime — the seam integration tests
/// use to inject recording/misbehaving extensions.
pub fn run_with_runtime(
    session_name: &str,
    daemonize: bool,
    config: ekko_config::Config,
    runtime: ekko_ext::AppRuntime,
) -> anyhow::Result<()> {
    logging::init(session_name).context("initializing logging")?;

    if daemonize {
        let log_file =
            logging::open_redirect_file(session_name).context("opening log file for stdout")?;
        daemonize_process(log_file).context("daemonizing")?;
    }

    let socket_path = ekko_proto::socket_path(session_name);
    let listener = ekko_proto::ipc_bind(&socket_path).context("binding session socket")?;
    // Liveness metadata for `ekko kill --force`: written at bind time so
    // even a never-attached (no manifest) or wedged daemon has a verified
    // escalation target.
    let pid_path = ekko_proto::pid_path(session_name);
    if let Err(e) = std::fs::write(&pid_path, std::process::id().to_string()) {
        log::warn!(
            "daemon: failed to write pid file {}: {e}",
            pid_path.display()
        );
    }
    // RAII: the hub's `shutdown_cleanup` removes the socket on a clean exit,
    // but a hub-thread panic unwinds straight past it. A dead daemon must
    // never leave a connectable-looking ghost socket behind.
    let _socket_guard = SocketGuard(socket_path.clone(), pid_path);

    let (hub_tx, hub_rx) = std::sync::mpsc::channel::<HubInstruction>();

    install_panic_hook(hub_tx.clone());
    spawn_listener_thread(listener, hub_tx.clone())?;
    spawn_signal_thread(hub_tx.clone())?;

    let hub = Hub::new(session_name.to_string(), config, hub_tx, runtime);
    hub.run(hub_rx);
    Ok(())
}

/// Removes the session socket and PID file when dropped, however the daemon
/// exits (clean hub shutdown, panic unwind). Best-effort: removal races with
/// a freshly respawned daemon are lost loudly by the respawn's own bind.
struct SocketGuard(std::path::PathBuf, std::path::PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(&self.1);
    }
}

/// Scan for known sessions: live ones (a socket is currently bound) and
/// resurrectable ones (a manifest exists but the daemon has exited).
pub fn list_sessions() -> anyhow::Result<Vec<ekko_proto::SessionSummary>> {
    ekko_resurrection::list_sessions()
}

/// Reproduce daemonize 0.5's daemon process setup: fork, wait-and-exit in
/// the original parent, then setsid and fork again so the surviving process
/// cannot reacquire a controlling terminal.
fn daemonize_process(log_file: std::fs::File) -> anyhow::Result<()> {
    // SAFETY: fork is called before this process creates any threads.
    let first_pid = unsafe { libc::fork() };
    if first_pid < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if first_pid > 0 {
        let mut status = 0;
        // SAFETY: first_pid is the child returned by fork and status is valid.
        let result = unsafe { libc::waitpid(first_pid, &mut status, 0) };
        if result < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        std::process::exit(status);
    }

    // SAFETY: this is the single-threaded child of the first fork.
    if unsafe { libc::setsid() } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // daemonize defaults to / and umask 027.
    std::env::set_current_dir("/")?;
    // SAFETY: umask has no memory-safety preconditions.
    unsafe { libc::umask(0o027) };

    // The first child exits after the second fork; daemonize uses the normal
    // process exit path here, so match it rather than using _exit.
    // SAFETY: fork remains before any threads are spawned.
    let second_pid = unsafe { libc::fork() };
    if second_pid < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if second_pid > 0 {
        std::process::exit(0);
    }

    let log_fd = log_file.as_raw_fd();
    // SAFETY: descriptors are valid and dup2/open/close are async-signal-safe.
    let null_fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDWR) };
    if null_fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    for (target, source) in [
        (libc::STDIN_FILENO, null_fd),
        (libc::STDOUT_FILENO, log_fd),
        (libc::STDERR_FILENO, log_fd),
    ] {
        if unsafe { libc::dup2(source, target) } < 0 {
            let error = std::io::Error::last_os_error();
            unsafe { libc::close(null_fd) };
            return Err(error.into());
        }
    }
    unsafe { libc::close(null_fd) };
    Ok(())
}

fn spawn_listener_thread(
    listener: ekko_proto::IpcListener,
    hub_tx: Sender<HubInstruction>,
) -> anyhow::Result<()> {
    thread::Builder::new()
        .name("listener".to_string())
        .spawn(move || {
            for conn in listener {
                match conn {
                    Ok(stream) => {
                        if hub_tx.send(HubInstruction::NewClient(stream)).is_err() {
                            return;
                        }
                    }
                    Err(e) => log::warn!("listener: accept error: {e}"),
                }
            }
        })
        .context("spawning listener thread")?;
    Ok(())
}

fn spawn_signal_thread(hub_tx: Sender<HubInstruction>) -> anyhow::Result<()> {
    let mut signals = Signals::new([SIGTERM, SIGINT]).context("installing signal handlers")?;
    thread::Builder::new()
        .name("signal-handler".to_string())
        .spawn(move || {
            if signals.forever().next().is_some() {
                let _ = hub_tx.send(HubInstruction::Shutdown);
            }
        })
        .context("spawning signal-handler thread")?;
    Ok(())
}

/// Installs a panic hook that keeps the default (still prints to stderr /
/// the log file) but also reports non-main-thread panics to the hub so a
/// crashed PTY thread can be turned into a clean session shutdown instead of
/// silently wedging the daemon.
///
/// A main-thread panic (the hub loop itself) just unwinds and exits the
/// process normally; there's no hub left to notify.
fn install_panic_hook(hub_tx: Sender<HubInstruction>) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let thread_name = thread::current().name().unwrap_or("<unnamed>").to_string();
        if thread_name == "main" {
            return;
        }
        let message = panic_message(info);
        let instruction =
            if let Some(pane) = terminal_pane::pane_key_from_pty_thread_name(&thread_name) {
                HubInstruction::PtyThreadPanicked {
                    pane,
                    thread_name,
                    message,
                }
            } else {
                HubInstruction::ThreadPanicked {
                    thread_name,
                    message,
                }
            };
        let _ = hub_tx.send(instruction);
    }));
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}
