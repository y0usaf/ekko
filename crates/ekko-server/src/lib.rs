//! The ekko session daemon: one process per session, zellij-style.
//!
//! [`run`] binds the session's socket, spawns its shell PTY (on first
//! attach), and hosts a `vt100::Parser` that survives detach/reattach. See
//! `hub.rs` for the single-threaded state machine everything else reports
//! into.

mod client_io;
mod grid;
mod hub;
mod input_compat;
mod logging;
mod pty_io;
mod pty_writer;
mod vt_compat;

use std::thread;

use anyhow::Context;
use crossbeam_channel::Sender;
use daemonize::{Daemonize, Stdio};
use interprocess::local_socket::Listener;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

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
    ));
    builder.build()
}

/// Run the session daemon for `session_name` until the session exits (shell
/// exit, explicit kill, a crash in a PTY thread, or a shutdown signal).
///
/// When `daemonize` is true, this forks into the background (stdout/stderr
/// redirected to `~/.cache/ekko/logs/<session_name>.log`) and only the child
/// process's call returns; the parent exits from inside `daemonize::start`.
pub fn run(session_name: &str, daemonize: bool) -> anyhow::Result<()> {
    let config = ekko_config::Config::load_default().unwrap_or_default();
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
        let stdout =
            logging::open_redirect_file(session_name).context("opening log file for stdout")?;
        let stderr =
            logging::open_redirect_file(session_name).context("opening log file for stderr")?;
        Daemonize::new()
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .start()
            .context("daemonizing")?;
    }

    let socket_path = ekko_proto::socket_path(session_name);
    let listener = ekko_proto::ipc_bind(&socket_path).context("binding session socket")?;

    let (hub_tx, hub_rx) = crossbeam_channel::unbounded::<HubInstruction>();

    install_panic_hook(hub_tx.clone());
    spawn_listener_thread(listener, hub_tx.clone())?;
    spawn_signal_thread(hub_tx.clone())?;

    let hub = Hub::new(session_name.to_string(), config, hub_tx, runtime);
    hub.run(hub_rx);
    Ok(())
}

/// Scan for known sessions: live ones (a socket is currently bound) and
/// resurrectable ones (a manifest exists but the daemon has exited).
pub fn list_sessions() -> anyhow::Result<Vec<ekko_proto::SessionSummary>> {
    ekko_resurrection::list_sessions()
}

fn spawn_listener_thread(listener: Listener, hub_tx: Sender<HubInstruction>) -> anyhow::Result<()> {
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
        let _ = hub_tx.send(HubInstruction::ThreadPanicked {
            thread_name,
            message,
        });
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
