//! ekko-client: the attach client of the ekko terminal multiplexer. Connects to
//! a per-session daemon over a Unix socket, hosts the client-side extension
//! runtime (surfaces, modes, commands, keybindings, overlays — all policy
//! lives in extensions, stock behavior in `ekko-builtins`), and forwards
//! keyboard/mouse input.

mod clipboard;
mod drawctx;
mod event_loop;
mod gridblit;
mod input;
mod scene;
mod sessions;
mod spawn;
mod state;

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use interprocess::local_socket::traits::Stream as StreamTrait;
use ekko_config::Config;
use ekko_proto::{AttachRejectReason, ClientToServer, ServerToClient, WIRE_VERSION, socket_path};

/// Options controlling how [`run`] attaches to a session.
#[derive(Clone, Debug)]
pub struct ClientOptions {
    pub session_name: String,
    pub create_if_missing: bool,
    pub force: bool,
}

/// What the client should do once its event loop exits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientOutcome {
    /// The whole `ekko` process should exit.
    Exited,
    /// Reconnect in-place to a different session (e.g. after ctrl+n or
    /// `:switch`), re-entering `mode` if the switch was made from one (a
    /// sticky leader map keeps navigating with its panel open).
    SwitchTo { name: String, mode: Option<String> },
}

/// A fresh throwaway session name, used when `ekko new` / ctrl+n get no
/// explicit name.
pub fn generate_session_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("session-{nanos:08x}")
}

/// Build the client's extension runtime: builtins first (so user extensions
/// reusing a name fail loudly), filtered by `[extensions] disabled`.
fn build_runtime(config: &Config) -> Result<ekko_ext::AppRuntime> {
    let builder = ekko_ext::RuntimeBuilder::new().with_disabled(&config.extensions.disabled);
    #[cfg(feature = "builtins")]
    let builder = ekko_builtins::client_extensions(config)
        .into_iter()
        .fold(builder, ekko_ext::RuntimeBuilder::register_boxed_extension);
    #[cfg(feature = "keycast")]
    let builder = builder.register_extension(ekko_keycast::KeycastExtension);
    #[cfg(feature = "lua")]
    let builder = ekko_lua::load_extensions(&ekko_config::config_dir().join("extensions"))
        .into_iter()
        .fold(builder, ekko_ext::RuntimeBuilder::register_boxed_extension);
    builder.build()
}

/// Attach to (optionally creating) a session and run the client until it
/// detaches, is kicked, or the session exits. Session switches reconnect
/// in-place: the extension runtime, raw-mode guard, and the stdin/resize
/// reader threads are built once and live for the whole client process —
/// rebuilding them per switch reset the active mode (killing sticky leader
/// navigation) and left an orphaned stdin reader that ate the next
/// keystroke.
pub fn run(options: ClientOptions) -> Result<()> {
    let config = Config::load_default().unwrap_or_default();
    let runtime = build_runtime(&config).context("building extension runtime")?;

    // Restore the terminal from anywhere a panic unwinds through, since the
    // `RawModeGuard`'s `Drop` won't run during an abort/unhandled unwind out
    // of this thread's stack in every case.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ekko_tui::emergency_restore();
        previous_hook(info);
    }));
    let _raw_guard = ekko_tui::RawModeGuard::new()?;

    let (tx, rx) = std::sync::mpsc::channel::<event_loop::Event>();
    event_loop::spawn_stdin_reader(tx.clone());
    event_loop::spawn_resize_watcher(tx.clone());

    let mut session_name = options.session_name.clone();
    let mut resume_mode: Option<String> = None;
    let mut generation: u64 = 0;
    loop {
        let (send, recv, attached_name) =
            connect_and_attach(&session_name, options.create_if_missing, options.force)?;
        generation += 1;
        event_loop::spawn_socket_reader(tx.clone(), recv, generation);
        match event_loop::run_event_loop(
            send,
            &rx,
            attached_name,
            &runtime,
            resume_mode.take(),
            generation,
        )? {
            ClientOutcome::Exited => return Ok(()),
            ClientOutcome::SwitchTo { name, mode } => {
                session_name = name;
                resume_mode = mode;
            }
        }
    }
}

type ConnectedHalves = (Box<dyn std::io::Write + Send>, Box<dyn Read + Send>, String);

/// Connect to a session's daemon (spawning it if allowed) and complete the
/// attach handshake. Returns the send/recv halves plus the canonical session
/// name from the server.
fn connect_and_attach(
    session_name: &str,
    create_if_missing: bool,
    force: bool,
) -> Result<ConnectedHalves> {
    let path = socket_path(session_name);

    if !path.exists() {
        if !create_if_missing {
            bail!("session '{session_name}' not found");
        }
        spawn::spawn_daemon(session_name)?;
        spawn::wait_for_socket(&path)?;
    }

    let stream = match ekko_proto::ipc_connect(&path) {
        Ok(stream) => stream,
        // A socket file whose daemon is gone (e.g. killed -9) refuses
        // connections. Treat it like a missing session: clean up the stale
        // socket and respawn so the resurrection manifest can do its job.
        Err(_) if create_if_missing => {
            let _ = std::fs::remove_file(&path);
            spawn::spawn_daemon(session_name)?;
            spawn::wait_for_socket(&path)?;
            ekko_proto::ipc_connect(&path).with_context(|| {
                format!("connecting to session '{session_name}' after respawning its daemon")
            })?
        }
        Err(e) => {
            return Err(e).with_context(|| format!("connecting to session '{session_name}'"));
        }
    };
    let (recv_half, send_half) = stream.split();
    let mut recv: Box<dyn Read + Send> = Box::new(recv_half);
    let mut send: Box<dyn std::io::Write + Send> = Box::new(send_half);

    let (cols, rows) = ekko_tui::terminal_size();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    ekko_proto::write_msg(
        &mut send,
        &ClientToServer::Attach {
            wire_version: WIRE_VERSION,
            session_name: session_name.to_string(),
            create_if_missing,
            cols,
            rows,
            cwd,
            shell: None,
            force,
        },
    )
    .context("sending attach request")?;

    match ekko_proto::read_msg::<_, ServerToClient>(&mut recv).context("waiting for attach reply")? {
        Some(ServerToClient::Attached { session_name, .. }) => Ok((send, recv, session_name)),
        Some(ServerToClient::AttachRejected(reason)) => Err(attach_rejected_error(reason)),
        Some(other) => bail!("unexpected reply while attaching: {other:?}"),
        None => bail!("connection closed before attach completed"),
    }
}

fn attach_rejected_error(reason: AttachRejectReason) -> anyhow::Error {
    match reason {
        AttachRejectReason::WrongWireVersion => {
            anyhow::anyhow!("wire protocol version mismatch; rebuild ekko and try again")
        }
        AttachRejectReason::SessionNotFound => anyhow::anyhow!("session not found"),
        AttachRejectReason::SpawnFailed(message) => {
            anyhow::anyhow!("session daemon failed to start: {message}")
        }
    }
}

/// Kill a named session: if its socket is live, ask its daemon to shut down;
/// otherwise just remove any leftover manifest.
pub fn kill_session(name: &str) -> Result<()> {
    let path = socket_path(name);
    if path.exists() {
        match ekko_proto::ipc_connect(&path) {
            Ok(stream) => {
                let (mut recv, mut send): (Box<dyn Read + Send>, Box<dyn std::io::Write + Send>) = {
                    let (r, s) = stream.split();
                    (Box::new(r), Box::new(s))
                };
                ekko_proto::write_msg(&mut send, &ClientToServer::KillSession(name.to_string()))
                    .context("sending kill request")?;
                // Best-effort: wait briefly for the server to confirm or hang up.
                let _ = ekko_proto::read_msg::<_, ServerToClient>(&mut recv);
                println!("killed session '{name}'");
                return Ok(());
            }
            Err(_) => {
                println!("session '{name}' has a stale socket; cleaning up");
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    let manifest_dir = sessions::session_info_dir().join(name);
    if manifest_dir.exists() {
        std::fs::remove_dir_all(&manifest_dir)
            .with_context(|| format!("removing manifest for '{name}'"))?;
        println!("removed manifest for session '{name}'");
    } else {
        println!("no such session: '{name}'");
    }
    Ok(())
}
