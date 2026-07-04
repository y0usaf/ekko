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
    /// Surfaces the user has toggled off (`UiAction::ToggleSurface`);
    /// [`crate::AppRuntime::visible_surfaces`] skips these regardless of the
    /// surface's own `visible` predicate.
    pub hidden_surfaces: Vec<String>,
    /// The resolved chrome palette.
    pub theme: ThemePalette,
}

impl ClientSnapshot {
    pub const NORMAL_MODE: &'static str = "normal";
}

/// Policy hook grouping the flat session list into named project groups.
pub type SessionGroupFn = Arc<dyn Fn(Vec<SessionEntry>) -> Vec<ProjectGroup> + Send + Sync>;

/// The grouping used when no session grouper is registered (and the safe
/// fallback when a registered grouper fails): one flat, name-sorted
/// "sessions" group.
pub fn fallback_group(mut sessions: Vec<SessionEntry>) -> Vec<ProjectGroup> {
    if sessions.is_empty() {
        return Vec::new();
    }
    sessions.sort_by(|a, b| a.name.cmp(&b.name));
    vec![ProjectGroup {
        name: "sessions".to_string(),
        sessions,
    }]
}

pub struct SessionGrouperSpec {
    pub name: String,
    pub group: SessionGroupFn,
}

/// Creation context handed to the registered session namer when a session
/// is created without an explicit name.
#[derive(Clone, Debug)]
pub struct NamerInput {
    /// Working directory the new session will start in.
    pub cwd: PathBuf,
    /// Names already in use (live + resurrectable). Namers should avoid
    /// these; the host still uniquifies and sanitizes whatever comes back.
    pub taken: Vec<String>,
}

/// Policy hook producing a session name from its creation context. Pure:
/// inputs in, name out — invariants (uniqueness, filename safety, fallback)
/// are enforced by the host after the call returns.
pub type SessionNameFn = Arc<dyn Fn(&NamerInput) -> String + Send + Sync>;

pub struct SessionNamerSpec {
    pub name: String,
    pub generate: SessionNameFn,
}
