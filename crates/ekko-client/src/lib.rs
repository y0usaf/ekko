//! ekko-client: the attach client of the ekko terminal multiplexer. Connects to
//! a per-session daemon over a Unix socket, hosts the client-side extension
//! runtime (surfaces, modes, commands, keybindings, overlays — all policy
//! lives in extensions, stock behavior in `ekko-builtins`), and forwards
//! keyboard/mouse input.

mod actions;
mod borders;
mod clipboard;
mod ctl;
mod drawctx;
mod event_loop;
mod gridblit;
mod input;
mod scene;
mod sessions;
mod spawn;
mod state;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ekko_config::Config;
use ekko_ext::SessionState;
use ekko_proto::{
    AttachRejectReason, ClientToServer, ExitReason, ServerToClient, WIRE_VERSION, socket_path,
};
use interprocess::local_socket::traits::{RecvHalf as _, Stream as StreamTrait};

pub use ctl::{CtlError, dump, is_no_such_session, send};

/// Options controlling how [`run`] attaches to a session.
#[derive(Clone, Debug)]
pub struct ClientOptions {
    /// `None` asks the registered session namer for a fresh name (bare
    /// `ekko`, unnamed `ekko new`); naming is extension policy, so it can
    /// only be resolved once the runtime is built inside [`run`].
    pub session_name: Option<String>,
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

/// A fresh session name for bare `ekko`, unnamed `ekko new`, and alt+n:
/// naming is policy, so the registered session namer produces the candidate;
/// the invariants — printable, non-empty, unique among known sessions —
/// are enforced here regardless of what the namer returns. With no namer
/// registered (bare harness) or a garbage answer, falls back to a hex name.
pub(crate) fn next_session_name(runtime: &ekko_ext::AppRuntime) -> String {
    let taken: Vec<String> = sessions::scan_sessions()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    let named = runtime.session_namer().and_then(|namer| {
        let input = ekko_ext::NamerInput {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            taken: taken.clone(),
        };
        sanitize_session_name((namer.generate)(&input))
    });
    uniquify(named.unwrap_or_else(fallback_session_name), &taken)
}

/// Strip control characters and outer whitespace; `None` if nothing usable
/// survives. The cap is on the *encoded* filename (`/` and `%` expand 3x,
/// see `ekko_proto::encode_session_name`) because socket paths must fit in
/// `sun_path` (~108 bytes) alongside the socket directory prefix.
fn sanitize_session_name(raw: String) -> Option<String> {
    const MAX_ENCODED_LEN: usize = 60;
    let mut cleaned: String = raw.trim().chars().filter(|c| !c.is_control()).collect();
    while ekko_proto::encode_session_name(&cleaned).len() > MAX_ENCODED_LEN {
        cleaned.pop();
    }
    let cleaned = cleaned.trim_end();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

/// Suffix `-2`, `-3`, ... until the name collides with nothing known.
fn uniquify(base: String, taken: &[String]) -> String {
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken.contains(candidate))
        .expect("some numeric suffix is always free")
}

/// The no-policy fallback: a throwaway hex name.
fn fallback_session_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("session-{nanos:08x}")
}

/// Build the client's extension runtime: builtins first (so user extensions
/// reusing a name fail loudly), filtered by `[extensions] disabled`.
fn build_runtime(
    config: &Config,
    #[cfg_attr(not(feature = "builtins"), allow(unused_variables))] terminal_colors: Option<
        ekko_tui::TerminalColors,
    >,
) -> Result<ekko_ext::AppRuntime> {
    let builder = ekko_ext::RuntimeBuilder::new().with_disabled(&config.extensions.disabled);
    #[cfg(feature = "builtins")]
    let builder = builder
        .register_boxed_extensions(ekko_builtins::client_extensions(config, terminal_colors));
    #[cfg(feature = "keycast")]
    let builder = builder.register_extension(ekko_keycast::KeycastExtension);
    #[cfg(feature = "lua")]
    let builder = builder.register_boxed_extensions(ekko_lua::load_extensions(
        &ekko_config::config_dir().join("extensions"),
        ekko_lua::HostKind::Client,
        config,
    ));
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
    // The cascade (`init.lua` supersedes `config.toml` supersedes defaults)
    // lives in ekko-config; the lua feature just injects the evaluator.
    // A broken `init.lua` refuses to start rather than silently running on
    // defaults. Loaded before raw mode, so the error prints normally.
    #[cfg(feature = "lua")]
    let config = ekko_lua::load_config_cascade()?;
    #[cfg(not(feature = "lua"))]
    let config = Config::load_cascade(None)?;

    // Restore the terminal from anywhere a panic unwinds through, since the
    // `RawModeGuard`'s `Drop` won't run during an abort/unhandled unwind out
    // of this thread's stack in every case.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ekko_tui::emergency_restore();
        previous_hook(info);
    }));
    let _raw_guard = ekko_tui::RawModeGuard::new()?;

    // Probe the host terminal's colors (OSC 10/11/4) so the builtin theme can
    // derive its palette from them. Must run inside raw mode (the reply is
    // read byte-wise from stdin) and before the stdin reader thread spawns,
    // or the reader would eat the response.
    let terminal_colors = ekko_tui::detect_terminal_colors(std::time::Duration::from_millis(250));
    // Also forwarded to the server on attach, so the hub can answer the
    // child's OSC 10/11/4 color queries on the host terminal's behalf
    // (nested ekko, phi, neovim background detection).
    let wire_colors = terminal_colors.as_ref().map(wire_terminal_colors);
    let runtime = build_runtime(&config, terminal_colors).context("building extension runtime")?;

    let (tx, rx) = std::sync::mpsc::channel::<event_loop::Event>();
    event_loop::spawn_stdin_reader(tx.clone());
    event_loop::spawn_resize_watcher(tx.clone());

    let mut session_name = options
        .session_name
        .clone()
        .unwrap_or_else(|| next_session_name(&runtime));
    let mut resume_mode: Option<String> = None;
    let mut generation: u64 = 0;
    loop {
        let (send, recv, attached_name) = connect_and_attach(
            &session_name,
            options.create_if_missing,
            options.force,
            wire_colors.clone(),
        )?;
        generation += 1;
        event_loop::spawn_socket_reader(tx.clone(), recv, generation);
        match event_loop::run_event_loop(
            send,
            &rx,
            attached_name,
            &runtime,
            resume_mode.take(),
            generation,
            config.animation_interval_ms(),
            &_raw_guard,
        )? {
            ClientOutcome::Exited => return Ok(()),
            ClientOutcome::SwitchTo { name, mode } => {
                session_name = name;
                resume_mode = mode;
            }
        }
    }
}

/// `ekko activate`: ask one already-attached client to request focus/attention
/// from its host terminal (e.g. BEL → XDG activation urgency in foot).
pub fn activate() -> Result<()> {
    for entry in sessions::scan_sessions() {
        if entry.state == SessionState::Alive && request_activate(&entry.name).unwrap_or(false) {
            return Ok(());
        }
    }
    bail!("no attached ekko client to activate")
}

fn request_activate(session_name: &str) -> Result<bool> {
    let stream = ekko_proto::ipc_connect(&socket_path(session_name))?;
    let (recv_half, send_half) = stream.split();
    let mut recv: Box<dyn Read + Send> = Box::new(recv_half);
    let mut send: Box<dyn std::io::Write + Send> = Box::new(send_half);
    ekko_proto::write_msg(&mut send, &ClientToServer::Activate)
        .context("sending activate request")?;
    match ekko_proto::read_msg::<_, ServerToClient>(&mut recv)
        .context("waiting for activate reply")?
    {
        Some(ServerToClient::ActivateResult { delivered }) => Ok(delivered),
        Some(other) => bail!("unexpected reply to activate request: {other:?}"),
        None => bail!("connection closed before activate reply arrived"),
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
    terminal_colors: Option<ekko_proto::TerminalColors>,
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
            cols,
            rows,
            cwd,
            shell: None,
            force,
            terminal_colors,
        },
    )
    .context("sending attach request")?;

    match ekko_proto::read_msg::<_, ServerToClient>(&mut recv)
        .context("waiting for attach reply")?
    {
        Some(ServerToClient::Attached { session_name, .. }) => Ok((send, recv, session_name)),
        Some(ServerToClient::AttachRejected(reason)) => Err(attach_rejected_error(reason)),
        Some(other) => bail!("unexpected reply while attaching: {other:?}"),
        None => bail!("connection closed before attach completed"),
    }
}

/// Convert the TUI probe result into the wire representation sent on attach.
fn wire_terminal_colors(colors: &ekko_tui::TerminalColors) -> ekko_proto::TerminalColors {
    let rgb = |c: ekko_tui::Rgb| (c.0, c.1, c.2);
    ekko_proto::TerminalColors {
        background: rgb(colors.background),
        foreground: rgb(colors.foreground),
        palette: colors.palette.map(|slot| slot.map(rgb)),
    }
}

fn attach_rejected_error(reason: AttachRejectReason) -> anyhow::Error {
    match reason {
        AttachRejectReason::WrongWireVersion => {
            anyhow::anyhow!("wire protocol version mismatch; rebuild ekko and try again")
        }
        AttachRejectReason::SpawnFailed(message) => {
            anyhow::anyhow!("session daemon failed to start: {message}")
        }
    }
}

/// Kill a named session: if its socket is live, ask its daemon to shut down
/// and verify the teardown actually happened; otherwise just remove any
/// leftover manifest.
///
/// Closing is *confirmed*, never assumed: the daemon broadcasts `Exit` to
/// every client on kill (zellij semantics), and this function waits for that
/// acknowledgement plus the socket's removal before reporting success. With
/// `force`, a daemon that never confirms — wedged hub, flooded accept queue
/// — is escalated to an out-of-band `SIGKILL` using the PID recorded in its
/// resurrection manifest, after verifying the PID still belongs to this
/// session's daemon.
pub fn kill_session(name: &str, force: bool) -> Result<(), CtlError> {
    let path = socket_path(name);
    let path_was_present = path.exists();
    if path_was_present {
        match ekko_proto::ipc_connect(&path) {
            Ok(stream) => {
                let (recv, mut send) = stream.split();
                // Bound the wait: without a timeout a wedged daemon hangs
                // this command forever, the user interrupts, and the kill
                // silently never happened.
                recv.set_timeout(Some(KILL_REPLY_TIMEOUT))
                    .context("arming kill-confirmation timeout")
                    .map_err(CtlError::Other)?;
                ekko_proto::write_msg(&mut send, &ClientToServer::KillSession(name.to_string()))
                    .context("sending kill request")
                    .map_err(CtlError::Other)?;
                return confirm_kill(name, force, recv, &path).map_err(CtlError::Other);
            }
            Err(_) => {
                // Connect failed: stale socket from a dead daemon — or a
                // live daemon whose accept queue is flooded. With --force,
                // resolve that ambiguity via the daemon's PID file.
                if force && daemon_pid(name).is_some() {
                    return force_kill_daemon(name, &path).map_err(CtlError::Other);
                }
                eprintln!("session '{name}' has a stale socket; cleaning up");
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_file(ekko_proto::pid_path(name));
            }
        }
    }

    if ekko_resurrection::read(name).is_some() {
        ekko_resurrection::delete(name);
    } else if !path_was_present {
        return Err(CtlError::NoSuchSession(name.to_string()));
    }
    Ok(())
}

/// How long `ekko kill` waits for the daemon's `Exit` acknowledgement before
/// concluding the daemon is wedged.
const KILL_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to poll for the socket to vanish after a confirmed kill.
const KILL_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

/// Interpret the daemon's reply to a `KillSession` request and report
/// honestly; escalate to `--force` when the daemon can't or won't confirm.
fn confirm_kill<R: Read>(name: &str, force: bool, mut recv: R, path: &Path) -> Result<()> {
    let refusal = loop {
        match ekko_proto::read_msg::<_, ServerToClient>(&mut recv) {
            Ok(Some(ServerToClient::Exit(ExitReason::ServerError(message)))) => {
                break Some(message);
            }
            Ok(Some(ServerToClient::Exit(_))) | Ok(None) => break None,
            // Not the reply we're after (e.g. a stray Notice); keep waiting
            // until the timeout fires.
            Ok(Some(_)) => continue,
            Err(ekko_proto::FrameError::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if force {
                    return force_kill_daemon(name, path);
                }
                bail!(
                    "daemon for session '{name}' did not confirm within {KILL_REPLY_TIMEOUT:?}; \
                     it may be wedged — retry or use --force"
                );
            }
            // Torn connection: possibly the daemon dying mid-teardown. The
            // socket check below decides what really happened.
            Err(_) => break None,
        }
    };
    if let Some(message) = refusal {
        bail!("daemon refused to kill session '{name}': {message}");
    }
    if wait_socket_gone(path, KILL_CLEANUP_TIMEOUT) {
        return Ok(());
    }
    if force {
        return force_kill_daemon(name, path);
    }
    bail!(
        "daemon for session '{name}' stopped replying but the session is still up; \
         retry or use --force"
    );
}

/// Poll until the session socket disappears or the budget elapses.
fn wait_socket_gone(path: &Path, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while path.exists() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    true
}

/// Read the daemon's PID file for a session: the out-of-band liveness
/// record written at bind time. Present even for never-attached sessions
/// (no manifest yet) and when the resurrection extension is disabled.
fn daemon_pid(name: &str) -> Option<u32> {
    std::fs::read_to_string(ekko_proto::pid_path(name))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Out-of-band escalation for a wedged daemon: `SIGKILL` the daemon PID from
/// its PID file — after verifying it still looks like this session's daemon
/// — then clear the socket, PID file, and manifest by hand, exactly what
/// the daemon's own cleanup would have done.
fn force_kill_daemon(name: &str, path: &Path) -> Result<()> {
    let pid = daemon_pid(name).ok_or_else(|| {
        anyhow::anyhow!("no daemon PID file for session '{name}'; cannot --force safely")
    })?;
    match verify_daemon_pid(name, pid) {
        Ok(true) => {
            // SAFETY: plain libc kill(2); SIGKILL has no error cases worth
            // handling here — a dead pid just means teardown already won.
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            eprintln!("force-killed wedged daemon for session '{name}' (pid {pid})");
        }
        Ok(false) => {
            eprintln!("daemon for session '{name}' (pid {pid}) is already gone; cleaning up");
        }
        Err(e) => return Err(e),
    }
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(ekko_proto::pid_path(name));
    ekko_resurrection::delete(name);
    Ok(())
}

/// Check that `pid` still belongs to this session's daemon before sending it
/// `SIGKILL`: PIDs get recycled, and killing a recycled PID would nuke an
/// innocent process. `Ok(false)` means the pid is dead. Linux-only — on
/// other platforms force-kill refuses rather than guess.
#[cfg(target_os = "linux")]
fn verify_daemon_pid(name: &str, pid: u32) -> Result<bool> {
    let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return Ok(false);
    };
    let args: Vec<&[u8]> = cmdline
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .collect();
    let matches = args
        .windows(2)
        .any(|pair| pair[0] == b"--server" && pair[1] == name.as_bytes());
    if matches {
        Ok(true)
    } else {
        bail!("pid {pid} no longer looks like session '{name}'s daemon; refusing to SIGKILL it");
    }
}

#[cfg(not(target_os = "linux"))]
fn verify_daemon_pid(name: &str, _pid: u32) -> Result<bool> {
    let _ = name;
    bail!("--force is only supported on Linux (no /proc for PID verification)")
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn sanitize_strips_controls_and_rejects_empty() {
        assert_eq!(
            sanitize_session_name("  a\x1b[31mb\n ".into()),
            Some("a[31mb".to_string())
        );
        assert_eq!(sanitize_session_name(" \t\n ".into()), None);
        assert_eq!(sanitize_session_name(String::new()), None);
    }

    #[test]
    fn sanitize_caps_encoded_length() {
        let deep = format!("~/{}", "very/".repeat(30));
        let name = sanitize_session_name(deep).expect("something survives");
        assert!(ekko_proto::encode_session_name(&name).len() <= 60);
    }

    #[test]
    fn uniquify_suffixes_collisions() {
        let taken = vec!["~/p a-b".to_string(), "~/p a-b-2".to_string()];
        assert_eq!(uniquify("~/p a-b".into(), &taken), "~/p a-b-3");
        assert_eq!(uniquify("fresh".into(), &taken), "fresh");
    }
}
