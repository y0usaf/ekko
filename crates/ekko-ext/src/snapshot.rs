//! Immutable state snapshot handed to extension callbacks.
//!
//! Extensions never see `&mut` host state (the takhti discipline): reads
//! come from a [`ClientSnapshot`] built once per render/input cycle; writes
//! happen only through returned [`ekko_event::UiAction`]s, applied by the host
//! after the callback returns.

use std::path::PathBuf;
use std::sync::Arc;

use ekko_event::NoteKind;

use crate::visual::ThemePalette;

/// Coarse session state as seen locally by the client (no server round-trip).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// Socket exists: a server process is presumably alive.
    Alive,
    /// No socket, but a manifest was found: resurrectable.
    Gone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEntry {
    pub name: String,
    pub cwd: PathBuf,
    pub state: SessionState,
    pub created_at_secs: u64,
}

/// A group of sessions sharing a "project" as decided by the registered
/// session grouper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectGroup {
    pub name: String,
    pub sessions: Vec<SessionEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusNote {
    pub text: String,
    pub kind: NoteKind,
}

/// Read-only view of the client's state for one render/input cycle.
#[derive(Clone, Debug)]
pub struct ClientSnapshot {
    /// The attached session's name.
    pub session_name: String,
    /// The active mode name; `"normal"` when no registered mode is active.
    pub mode: String,
    /// Host terminal dimensions.
    pub cols: u16,
    pub rows: u16,
    /// The server-side PTY grid dimensions.
    pub grid_cols: u16,
    pub grid_rows: u16,
    /// Scrollback view offset in lines back from the live screen (0 = live).
    pub scrollback: u32,
    /// Grouped session list from the last local scan.
    pub projects: Vec<ProjectGroup>,
    /// Transient statusbar note, if one is active.
    pub status_note: Option<StatusNote>,
    /// Every registered keybinding with its mode scope (`None` = normal),
    /// for hint/help/panel rendering.
    pub keybindings: Vec<crate::KeybindingInfo>,
    /// Wall-clock milliseconds for animation.
    pub now_ms: u64,
    /// The resolved chrome palette.
    pub theme: ThemePalette,
}

impl ClientSnapshot {
    pub const NORMAL_MODE: &'static str = "normal";
}

/// Policy hook grouping the flat session list into named project groups.
pub type SessionGroupFn = Arc<dyn Fn(Vec<SessionEntry>) -> Vec<ProjectGroup> + Send + Sync>;

pub struct SessionGrouperSpec {
    pub name: String,
    pub group: SessionGroupFn,
}
