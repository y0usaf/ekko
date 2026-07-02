//! Shared event vocabulary for the ekko extension system.
//!
//! One flat enum set used by both sides of the socket: the client host and
//! the server (daemon) host each run their own `ekko_ext::AppRuntime` and each
//! dispatches only its own subset of [`EventKind`]s. Payloads are plain data
//! (`Clone` + serde) so handlers never hold borrows into live host state —
//! reads come from the payload/snapshot, writes only via the returned
//! [`EventReturn`] / [`UiAction`] values, applied by the host after the
//! handler returns.
//!
//! This crate is deliberately independent of `ekko-proto`: the wire contract
//! must stay small and stable while this vocabulary grows freely with the
//! builtins. The hub and the client event loop are the only translation
//! points between the two.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// The kind of lifecycle event that was emitted.
///
/// Client-side kinds are dispatched by the attach client's host; server-side
/// kinds by the per-session daemon's host. The doc comment on each variant
/// states which side emits it and whether handler returns are consumed
/// ("gate") or discarded ("notification").
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    // ── Client lifecycle ────────────────────────────────────────────────────
    /// Client: raw mode entered, first render about to happen. Notification.
    ClientReady,
    /// Client: attach reply received. Notification.
    SessionAttached,
    /// Client: about to detach. Gate — `Cancel` aborts the detach.
    BeforeSessionDetach,
    /// Client: detach sent. Notification.
    SessionDetached,
    /// Client: about to switch sessions in place. Gate — `Cancel` aborts.
    BeforeSessionSwitch,
    /// Client: reconnect target decided. Notification.
    SessionSwitched,
    /// Client: local session scan completed. Notification.
    SessionListRefreshed,

    // ── Client render-adjacent notifications ────────────────────────────────
    /// Client: a grid update was applied. Notification.
    GridUpdated,
    /// Client or server: the terminal rang its bell. Notification.
    Bell,
    /// Client: the host terminal was resized. Notification.
    Resize,
    /// Client: animation heartbeat. Notification.
    Tick,

    // ── Client input pipeline ──────────────────────────────────────────────
    /// Client: raw stdin bytes about to be processed. Gate —
    /// `KeyIntercept(Consume | Transform)` alters the pipeline.
    KeyInput,
    /// Client: the active mode changed. Notification.
    ModeChanged,
    /// Client: a resolved command is about to run. Gate — `Cancel` blocks it.
    CommandInvoked,

    // ── Server lifecycle ────────────────────────────────────────────────────
    /// Server: PTY about to spawn. Gate — `PtySpawnOverride` rewrites the
    /// shell/args/cwd/env before the spawn.
    BeforePtySpawn,
    /// Server: PTY spawned; the session exists. Notification.
    SessionCreated,
    /// Server: a client (re)attached. Notification.
    ClientAttached,
    /// Server: the attached client detached. Notification.
    ClientDetached,
    /// Server: the session is going away (PTY exit, kill, panic, signal).
    /// Notification.
    SessionExited,
    /// Server: the PTY was resized. Notification.
    PtyResized,
    /// Server: periodic liveness heartbeat. Notification.
    Heartbeat,
}

impl EventKind {
    /// Every variant, for iteration — the Lua bridge resolves subscription
    /// strings against this. Keep in sync with [`EventKind::name`], whose
    /// exhaustive match is the compile-time reminder when a variant is added
    /// (a test pins that every variant's name resolves back through `ALL`).
    pub const ALL: [EventKind; 21] = [
        EventKind::ClientReady,
        EventKind::SessionAttached,
        EventKind::BeforeSessionDetach,
        EventKind::SessionDetached,
        EventKind::BeforeSessionSwitch,
        EventKind::SessionSwitched,
        EventKind::SessionListRefreshed,
        EventKind::GridUpdated,
        EventKind::Bell,
        EventKind::Resize,
        EventKind::Tick,
        EventKind::KeyInput,
        EventKind::ModeChanged,
        EventKind::CommandInvoked,
        EventKind::BeforePtySpawn,
        EventKind::SessionCreated,
        EventKind::ClientAttached,
        EventKind::ClientDetached,
        EventKind::SessionExited,
        EventKind::PtyResized,
        EventKind::Heartbeat,
    ];

    /// Canonical snake_case name, used for Lua subscription strings.
    pub const fn name(self) -> &'static str {
        match self {
            EventKind::ClientReady => "client_ready",
            EventKind::SessionAttached => "session_attached",
            EventKind::BeforeSessionDetach => "before_session_detach",
            EventKind::SessionDetached => "session_detached",
            EventKind::BeforeSessionSwitch => "before_session_switch",
            EventKind::SessionSwitched => "session_switched",
            EventKind::SessionListRefreshed => "session_list_refreshed",
            EventKind::GridUpdated => "grid_updated",
            EventKind::Bell => "bell",
            EventKind::Resize => "resize",
            EventKind::Tick => "tick",
            EventKind::KeyInput => "key_input",
            EventKind::ModeChanged => "mode_changed",
            EventKind::CommandInvoked => "command_invoked",
            EventKind::BeforePtySpawn => "before_pty_spawn",
            EventKind::SessionCreated => "session_created",
            EventKind::ClientAttached => "client_attached",
            EventKind::ClientDetached => "client_detached",
            EventKind::SessionExited => "session_exited",
            EventKind::PtyResized => "pty_resized",
            EventKind::Heartbeat => "heartbeat",
        }
    }

    /// Inverse of [`EventKind::name`].
    pub fn from_name(name: &str) -> Option<EventKind> {
        EventKind::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// The payload accompanying a lifecycle event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EventPayload {
    Empty,

    // ── Client ──────────────────────────────────────────────────────────────
    SessionAttached {
        session_name: String,
        wire_version: u32,
    },
    BeforeSessionDetach {
        session_name: String,
    },
    SessionSwitch {
        from: String,
        to: String,
    },
    GridUpdated {
        epoch: u64,
        cols: u16,
        rows: u16,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Tick {
        now_ms: u64,
    },
    KeyInput {
        bytes: Vec<u8>,
    },
    ModeChanged {
        from: String,
        to: String,
    },
    CommandInvoked {
        name: String,
        raw_args: String,
    },

    // ── Server ──────────────────────────────────────────────────────────────
    PtySpawn {
        session_name: String,
        shell: PathBuf,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    },
    SessionCreated {
        session_name: String,
        shell: PathBuf,
        cwd: PathBuf,
    },
    ClientAttached {
        session_name: String,
        client_id: u64,
        cols: u16,
        rows: u16,
    },
    ClientDetached {
        session_name: String,
        client_id: u64,
    },
    SessionExited {
        session_name: String,
        exit_code: Option<i32>,
        reason: SessionExitReason,
    },
    PtyResized {
        session_name: String,
        cols: u16,
        rows: u16,
    },
    Heartbeat {
        session_name: String,
    },
    Bell {
        session_name: String,
    },
}

/// Why a server session is going away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionExitReason {
    /// The shell process exited on its own.
    ShellExited,
    /// An explicit `KillSession` request.
    Killed,
    /// A PTY thread panicked.
    Crashed,
    /// SIGTERM/SIGINT to the daemon.
    Shutdown,
}

/// A lifecycle event dispatched to extension handlers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LifecycleEvent {
    pub kind: EventKind,
    pub payload: EventPayload,
}

impl LifecycleEvent {
    pub fn empty(kind: EventKind) -> Self {
        Self {
            kind,
            payload: EventPayload::Empty,
        }
    }
}

/// The value returned by an extension event handler.
///
/// Handlers return `None` to observe-only, or `Some(EventReturn::...)` to
/// influence behavior. The variant must match the event kind — mismatched
/// returns are ignored by the host.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EventReturn {
    /// Returned for cancelable gates (`BeforeSessionDetach`,
    /// `BeforeSessionSwitch`, `CommandInvoked`): abort the operation.
    Cancel { reason: String },

    /// Returned for [`EventKind::KeyInput`]: consume or rewrite the bytes.
    KeyIntercept(KeyIntercept),

    /// Returned for [`EventKind::BeforePtySpawn`]: override spawn parameters.
    /// `None` fields keep the host-resolved value; `env` entries are appended
    /// to the child environment.
    PtySpawnOverride {
        shell: Option<PathBuf>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
    },

    /// Returned for server-side notifications: ask the hub to surface a
    /// message to the attached client (translated to a wire `Notice`).
    EmitNotice { level: NoticeLevel, message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum KeyIntercept {
    /// Swallow the input entirely.
    Consume,
    /// Replace the input bytes and continue the pipeline.
    Transform(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeLevel {
    Info,
    Warn,
}

/// Status-note severity shown in the client statusbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteKind {
    Ok,
    Error,
    /// Neutral feedback (e.g. "no other session"); no chip color change.
    Info,
}

/// A declarative host-side effect requested by a command, keybinding, mode,
/// or overlay handler. Handlers cannot touch client state directly; they
/// return actions the client interprets in exactly one place
/// (`apply_ui_action`). Anything a builtin needs that isn't expressible here
/// is an API gap to be filled by a new variant, not a hardwired branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiAction {
    /// Detach from the session and exit the client.
    Detach,
    /// Exit the client without detaching semantics.
    Quit,
    /// Spawn (if needed) and switch to a session. `None` generates a name.
    NewSession { name: Option<String> },
    /// Reconnect in place to the named session.
    SwitchSession { name: String },
    /// Ask the server to kill the current session.
    KillCurrentSession,
    /// Enter a registered input mode by name.
    EnterMode { name: String },
    /// Return to normal mode.
    ExitMode,
    /// Open a registered overlay by name.
    OpenOverlay { name: String },
    /// Close the active overlay.
    CloseOverlay,
    /// Show a transient note in the statusbar.
    SetStatusNote {
        text: String,
        kind: NoteKind,
        ttl_ms: u64,
    },
    /// Forward raw bytes to the PTY.
    ForwardKey { bytes: Vec<u8> },
    /// Parse and run a `:command` line through the command registry.
    InvokeCommand { line: String },
    /// Move the session's scrollback view by `delta` lines (positive = back
    /// into history; the server clamps to the available history).
    Scroll { delta: i32 },
    /// Jump the scrollback view back to the live screen.
    ScrollToBottom,
}

/// A handler function invoked when a lifecycle event is dispatched.
pub type EventHandler =
    Arc<dyn Fn(LifecycleEvent) -> anyhow::Result<Option<EventReturn>> + Send + Sync>;

pub struct EventHandlerRegistration {
    pub event: EventKind,
    /// Shown in logs when the handler errors or times out.
    pub label: String,
    pub handler: EventHandler,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_names_are_unique_and_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for kind in EventKind::ALL {
            assert!(seen.insert(kind.name()), "duplicate name {}", kind.name());
            assert_eq!(EventKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(EventKind::from_name("no_such_event"), None);
    }
}
