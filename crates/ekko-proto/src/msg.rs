//! Wire message types exchanged between the ekko client and server.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Messages sent from a client to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientToServer {
    /// Ask to attach to (and optionally create) a session.
    Attach {
        wire_version: u32,
        session_name: String,
        create_if_missing: bool,
        cols: u16,
        rows: u16,
        cwd: PathBuf,
        shell: Option<PathBuf>,
        force: bool,
    },
    /// Detach from the current session without killing it.
    Detach,
    /// The client's terminal was resized.
    Resize { cols: u16, rows: u16 },
    /// Raw key input, already encoded (e.g. escape sequences).
    Key(Vec<u8>),
    /// Bracketed-paste (or plain paste) content. The server re-wraps it in
    /// paste markers when the child has bracketed paste enabled.
    Paste(Vec<u8>),
    /// Scroll the session's scrollback view. Positive `delta` moves back
    /// into history, negative toward the live screen.
    Scroll { delta: i32 },
    /// Jump the scrollback view back to the live screen.
    ScrollReset,
    /// A higher-level client command (session management, etc).
    Command(ClientCommand),
    /// Ask the server for a list of known sessions.
    ListSessions,
    /// Ask the server to kill a named session.
    KillSession(String),
    /// Liveness check.
    Ping,
}

/// Session-management commands issued by the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientCommand {
    NewSession {
        name: Option<String>,
        cwd: Option<PathBuf>,
    },
    SwitchSession(String),
    RenameSession(String),
    KillCurrentSession,
}

/// Messages sent from the server to a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerToClient {
    /// Attach succeeded.
    Attached {
        session_name: String,
        wire_version: u32,
    },
    /// Attach was refused.
    AttachRejected(AttachRejectReason),
    /// A grid update to render.
    Grid(GridUpdate),
    /// Response to `ListSessions`.
    Sessions(Vec<SessionSummary>),
    /// Terminal bell.
    Bell,
    /// The client should disconnect.
    Exit(ExitReason),
    /// Reply to `Ping`.
    Pong,
    /// A message a server-side extension asked the hub to surface to the
    /// attached client.
    Notice(ServerNotice),
    /// The child set the window title (OSC 0/2); the client forwards it to
    /// the host terminal.
    Title(String),
    /// The child wrote to the clipboard (OSC 52). The payload is the
    /// still-base64-encoded data, ready to re-emit to the host terminal.
    ClipboardCopy(Vec<u8>),
}

/// An extension-originated message surfaced to the attached client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerNotice {
    /// The originating extension's manifest id, for attribution/filtering.
    pub source: String,
    pub level: NoticeLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeLevel {
    Info,
    Warn,
}

/// Reasons an attach attempt can be rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachRejectReason {
    WrongWireVersion,
    SessionNotFound,
    SpawnFailed(String),
}

/// Reasons a client connection is being terminated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitReason {
    Normal,
    Detached,
    Kicked,
    SessionExited(Option<i32>),
    ServerError(String),
}

/// Summary information about a session, as returned by `ListSessions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub name: String,
    pub cwd: PathBuf,
    pub attached: bool,
    pub alive: bool,
    pub created_at_secs: u64,
    pub status: SessionStatus,
}

/// Coarse status of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Running,
    Exited,
    Crashed,
}

/// An incremental or full update to the client's rendered grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridUpdate {
    /// Monotonically increasing counter; clients can use this to detect
    /// dropped or out-of-order updates.
    pub epoch: u64,
    pub cols: u16,
    pub rows: u16,
    pub cursor: Option<CursorState>,
    /// Terminal modes the client must honor (mouse reporting, focus events,
    /// alt screen), as last requested by the child.
    pub modes: TermModes,
    /// Current scrollback view offset in lines back from the live screen
    /// (0 = live).
    pub scrollback: u32,
    pub payload: GridPayload,
}

/// Child-requested terminal modes the client adapts its input handling to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermModes {
    pub alt_screen: bool,
    /// DECCKM: arrows should be sent as SS3 (`\x1bOA`) instead of CSI.
    pub app_cursor: bool,
    pub mouse_mode: MouseMode,
    pub mouse_encoding: MouseEncoding,
    /// Mode 1004: the child wants focus-in/focus-out reports.
    pub focus_reporting: bool,
}

/// Which mouse events the child asked to receive (DECSET 9/1000/1002/1003).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseMode {
    #[default]
    None,
    Press,
    PressRelease,
    ButtonMotion,
    AnyMotion,
}

/// How mouse events should be encoded for the child (DECSET 1005/1006).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseEncoding {
    #[default]
    Default,
    Utf8,
    Sgr,
}

/// Cursor position, visibility, and DECSCUSR shape (0 = terminal default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorState {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
    pub shape: u8,
}

/// Either a full redraw or a sparse set of changed rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GridPayload {
    Full(Vec<GridRow>),
    Rows(Vec<(u16, GridRow)>),
}

/// A single row of terminal cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridRow {
    pub cells: Vec<GridCell>,
}

/// A single terminal cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridCell {
    /// First codepoint of the cell's contents (`' '` when empty).
    pub ch: char,
    /// Remaining codepoints of a multi-codepoint grapheme cluster
    /// (combining marks, ZWJ emoji). Empty for the common single-codepoint
    /// case, which allocates nothing.
    pub extra: Vec<char>,
    pub fg: WireColor,
    pub bg: WireColor,
    pub attrs: u8,
}

impl GridCell {
    pub const BOLD: u8 = 1 << 0;
    pub const DIM: u8 = 1 << 1;
    pub const ITALIC: u8 = 1 << 2;
    pub const UNDERLINE: u8 = 1 << 3;
    pub const INVERSE: u8 = 1 << 4;
    pub const WIDE: u8 = 1 << 5;
    pub const WIDE_CONT: u8 = 1 << 6;
}

/// A terminal color as sent over the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireColor {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}
