//! Wire message types exchanged between the ekko client and server.

use std::path::PathBuf;

use crate::codec::{DecodeError, Wire};
use serde::{Deserialize, Serialize};

/// Messages sent from a client to the server.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientToServer {
    /// Ask to attach to the daemon's session. The daemon is per-session
    /// (one socket per session name), so the message doesn't carry a name.
    Attach {
        wire_version: u32,
        cols: u16,
        rows: u16,
        cwd: PathBuf,
        shell: Option<PathBuf>,
        force: bool,
        /// The host terminal's colors as probed by the client (OSC 10/11/4),
        /// so the server can answer the child's color queries on the host's
        /// behalf. `None` when the host terminal didn't answer the probe.
        terminal_colors: Option<TerminalColors>,
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
    /// Split the receiving client's focused pane in two, placing the new
    /// pane to the right of or below the current one. Ignored when the
    /// geometry would violate minimum pane dimensions.
    SplitPane { direction: SplitDirection },
    /// Move the receiving client's focus to the neighboring pane in a
    /// direction. A focus move that has no neighbor is a no-op.
    FocusDirection { direction: Direction },
    /// Focus one specific pane (e.g. a mouse hit). Unknown pane IDs are
    /// ignored safely.
    FocusPane { pane: u64 },
    /// Close the receiving client's focused pane; its sibling absorbs the
    /// space. Closing the last pane ends the session.
    CloseFocusedPane,
    /// Scroll the session's scrollback view. Positive `delta` moves back
    /// into history, negative toward the live screen.
    Scroll { delta: i32 },
    /// Jump the scrollback view back to the live screen.
    ScrollReset,

    /// Search the receiving client's focused pane's scrollback (plain
    /// text, smart case: an all-lowercase query is case-insensitive).
    /// Matches are reported in absolute coordinates (row 0 = the oldest
    /// stored line; live rows follow history) so they survive viewport
    /// movement.
    SearchScrollback { query: String },
    /// Dump the receiving client's focused pane's full scrollback + live
    /// screen as plain text (for edit-in-$EDITOR).
    DumpScrollback,
    /// Ask the server to kill its own (current) session.
    KillCurrentSession,
    /// Ask the server to kill a named session.
    KillSession(String),
    /// Liveness check.
    Ping,
    /// `ekko activate`: ask the daemon to have one attached client request
    /// focus/attention from its host terminal (e.g. BEL → XDG activation
    /// urgency in foot).
    Activate,
    /// `ekko send`: inject raw bytes into the session's primary pane — the
    /// focused pane of any attached client, else the first live pane. For
    /// scripting a session from outside; no attach required.
    Inject { bytes: Vec<u8> },
    /// `ekko dump`: dump the primary pane's scrollback + live screen as
    /// plain text. Replied to with `ServerToClient::ScrollbackDump`.
    DumpSession,
}

/// Messages sent from the server to a client.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerToClient {
    /// Attach succeeded.
    Attached {
        session_name: String,
        wire_version: u32,
    },
    /// Attach was refused.
    AttachRejected(AttachRejectReason),
    /// A workspace update to render: the complete pane projection plus
    /// incremental grid payloads.
    Workspace(WorkspaceUpdate),
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
    /// `ekko activate` relayed by the daemon: this client should ask its
    /// host terminal for attention/focus (e.g. BEL → XDG activation urgency).
    Activate,
    /// Reply to `ClientToServer::Activate`: whether the request was handed to
    /// an attached client.
    ActivateResult { delivered: bool },

    /// Reply to `ClientToServer::SearchScrollback`: every match in the
    /// searched pane, in absolute coordinates (see `SearchMatch`). An
    /// empty `matches` means the query was not found.
    SearchResults {
        pane: u64,
        query: String,
        matches: Vec<SearchMatch>,
    },
    /// Reply to `ClientToServer::DumpScrollback` or `DumpSession`: the
    /// pane's scrollback and live screen as plain text, wrapped logical
    /// lines joined.
    ScrollbackDump { pane: u64, text: String },
}

/// One scrollback search hit. `row` is absolute: 0 is the oldest stored
/// history line; live-screen rows continue after the history (a client
/// maps a match into the viewport as `row - (history - scrollback)` when
/// that lands inside the visible window). `col`/`len` are cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub row: u32,
    pub col: u16,
    pub len: u16,
}

/// An extension-originated message surfaced to the attached client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerNotice {
    /// The originating extension's manifest id, for attribution/filtering.
    pub source: String,
    pub level: NoticeLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warn,
}

/// Reasons an attach attempt can be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachRejectReason {
    WrongWireVersion,
    SpawnFailed(String),
}

/// Reasons a client connection is being terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitReason {
    Normal,
    Detached,
    Kicked,
    SessionExited(Option<i32>),
    ServerError(String),
}

/// Summary information about a session. Not a wire message: built locally
/// from resurrection manifests and live sockets (`ekko ls`, the sidebar).
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// How pane boundaries are rendered, and how much canvas geometry they
/// reserve. This is a session-level property: the daemon owns the canvas
/// and resolves every client's panes from one topology, so the style lives
/// in the server's config and rides the workspace to clients.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneBorderStyle {
    /// Panes tile edge to edge; no separator cells exist.
    #[default]
    None,
    /// One shared separator line between panes (zellij compact mode), with
    /// junction glyphs where lines meet. Reserves one cell per split edge.
    Compact,
    /// Every pane draws its own full box frame in the cells ringing its
    /// content. Reserves one cell around each pane (two between panes,
    /// one at the canvas edge).
    Frame,
}

impl PaneBorderStyle {
    /// Cells reserved between sibling subtrees at every split.
    pub fn gap(self) -> u16 {
        match self {
            Self::None => 0,
            Self::Compact => 1,
            Self::Frame => 2,
        }
    }

    /// Cells reserved between the canvas edge and the outermost panes.
    pub fn margin(self) -> u16 {
        match self {
            Self::None | Self::Compact => 0,
            Self::Frame => 1,
        }
    }
}

/// One frame of the complete pane workspace: full metadata for every live
/// pane, the receiving client's focused pane, and grid payloads for the
/// panes that changed since their last sent frame. Metadata is complete on
/// every update so removal and topology recovery are idempotent; grid
/// payloads stay incremental. `epoch` is the hub's monotonic frame counter.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceUpdate {
    pub epoch: u64,
    pub panes: Vec<PaneMeta>,
    /// The receiving client's focused pane (focus is per attached client).
    pub focused: u64,
    /// Grid updates for panes with new content only; a pane's last-known
    /// grid persists on the client when it is absent here.
    pub grids: Vec<PaneGrid>,
    /// Border style the client should draw between/around panes. Constant
    /// per session; repeated here so clients need no config agreement with
    /// the daemon.
    pub border_style: PaneBorderStyle,
}

/// The server's canonical projection of one tiled pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneMeta {
    pub id: u64,
    /// Canvas-local rectangle resolved from the split tree.
    pub rect: PaneRect,
    pub title: Option<String>,
}

/// A rectangle inside the session canvas, in cells.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneRect {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
}

/// A grid update addressed to one pane. The inner update's `epoch` equals
/// the enclosing workspace's epoch.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneGrid {
    pub pane: u64,
    pub update: GridUpdate,
}

/// Placement of the new pane in `ClientToServer::SplitPane`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// The new pane goes to the right of the focused one (vertical divider).
    Right,
    /// The new pane goes below the focused one (horizontal divider).
    Down,
}

/// A cardinal direction for directional focus moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// An incremental or full update to the client's rendered grid.
#[derive(Debug, Clone, PartialEq)]
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
    /// Total scrollback history lines currently stored (the maximum view
    /// offset). Drives the client's scroll indicator and absolute match
    /// coordinates.
    pub history: u32,
    pub payload: GridPayload,
}

/// Child-requested terminal modes the client adapts its input handling to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MouseMode {
    #[default]
    None,
    Press,
    PressRelease,
    ButtonMotion,
    AnyMotion,
}

/// How mouse events should be encoded for the child (DECSET 1005/1006).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MouseEncoding {
    #[default]
    Default,
    Utf8,
    Sgr,
}

/// Cursor position, visibility, and DECSCUSR shape (0 = terminal default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorState {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
    pub shape: u8,
}

/// Either a full redraw or a sparse set of changed rows.
#[derive(Debug, Clone, PartialEq)]
pub enum GridPayload {
    Full(Vec<GridRow>),
    Rows(Vec<(u16, GridRow)>),
}

/// A single row of terminal cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridRow {
    pub cells: Vec<GridCell>,
}

/// A single terminal cell.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireColor {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Host terminal colors forwarded on attach: default background/foreground
/// (OSC 11/10) plus the 16 ANSI palette entries (OSC 4). Entries the host
/// terminal didn't report stay `None`. Mirrors `ekko_tui::TerminalColors`,
/// re-declared here so the wire crate stays dependency-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalColors {
    pub background: (u8, u8, u8),
    pub foreground: (u8, u8, u8),
    pub palette: [Option<(u8, u8, u8)>; 16],
}

// ---- Hand-rolled wire codec (replaces serde/bincode) ----------------------

/// Implement `Wire` for a unit (field-less) enum by explicit discriminant
/// indices, matching bincode's `u32` LE variant index.
macro_rules! wire_unit_enum {
    ($name:ident { $($index:literal => $variant:ident),* $(,)? }) => {
        impl Wire for $name {
            fn write(&self, out: &mut Vec<u8>) {
                match self {
                    $( Self::$variant => u32::write(&$index, out), )*
                }
            }
            fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
                let d = u32::read(bytes, pos)?;
                match d {
                    $( $index => Ok(Self::$variant), )*
                    _ => Err(DecodeError::InvalidTag(stringify!($name))),
                }
            }
        }
    };
}

wire_unit_enum!(NoticeLevel { 0 => Info, 1 => Warn });
wire_unit_enum!(SessionStatus { 0 => Running, 1 => Exited, 2 => Crashed });
wire_unit_enum!(PaneBorderStyle { 0 => None, 1 => Compact, 2 => Frame });
wire_unit_enum!(SplitDirection { 0 => Right, 1 => Down });
wire_unit_enum!(Direction { 0 => Left, 1 => Right, 2 => Up, 3 => Down });
wire_unit_enum!(MouseMode { 0 => None, 1 => Press, 2 => PressRelease, 3 => ButtonMotion, 4 => AnyMotion });
wire_unit_enum!(MouseEncoding { 0 => Default, 1 => Utf8, 2 => Sgr });

impl Wire for SearchMatch {
    fn write(&self, out: &mut Vec<u8>) {
        self.row.write(out);
        self.col.write(out);
        self.len.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            row: u32::read(bytes, pos)?,
            col: u16::read(bytes, pos)?,
            len: u16::read(bytes, pos)?,
        })
    }
}

impl Wire for ServerNotice {
    fn write(&self, out: &mut Vec<u8>) {
        self.source.write(out);
        self.level.write(out);
        self.message.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            source: String::read(bytes, pos)?,
            level: NoticeLevel::read(bytes, pos)?,
            message: String::read(bytes, pos)?,
        })
    }
}

impl Wire for AttachRejectReason {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::WrongWireVersion => u32::write(&0, out),
            Self::SpawnFailed(m) => {
                u32::write(&1, out);
                m.write(out);
            }
        }
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        match u32::read(bytes, pos)? {
            0 => Ok(Self::WrongWireVersion),
            1 => Ok(Self::SpawnFailed(String::read(bytes, pos)?)),
            _ => Err(DecodeError::InvalidTag("AttachRejectReason")),
        }
    }
}

impl Wire for ExitReason {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Normal => u32::write(&0, out),
            Self::Detached => u32::write(&1, out),
            Self::Kicked => u32::write(&2, out),
            Self::SessionExited(code) => {
                u32::write(&3, out);
                code.write(out);
            }
            Self::ServerError(msg) => {
                u32::write(&4, out);
                msg.write(out);
            }
        }
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        match u32::read(bytes, pos)? {
            0 => Ok(Self::Normal),
            1 => Ok(Self::Detached),
            2 => Ok(Self::Kicked),
            3 => Ok(Self::SessionExited(Option::<i32>::read(bytes, pos)?)),
            4 => Ok(Self::ServerError(String::read(bytes, pos)?)),
            _ => Err(DecodeError::InvalidTag("ExitReason")),
        }
    }
}

impl Wire for SessionSummary {
    fn write(&self, out: &mut Vec<u8>) {
        self.name.write(out);
        self.cwd.write(out);
        self.attached.write(out);
        self.alive.write(out);
        self.created_at_secs.write(out);
        self.status.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            name: String::read(bytes, pos)?,
            cwd: PathBuf::read(bytes, pos)?,
            attached: bool::read(bytes, pos)?,
            alive: bool::read(bytes, pos)?,
            created_at_secs: u64::read(bytes, pos)?,
            status: SessionStatus::read(bytes, pos)?,
        })
    }
}

impl Wire for PaneRect {
    fn write(&self, out: &mut Vec<u8>) {
        self.x.write(out);
        self.y.write(out);
        self.cols.write(out);
        self.rows.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            x: u16::read(bytes, pos)?,
            y: u16::read(bytes, pos)?,
            cols: u16::read(bytes, pos)?,
            rows: u16::read(bytes, pos)?,
        })
    }
}

impl Wire for PaneMeta {
    fn write(&self, out: &mut Vec<u8>) {
        self.id.write(out);
        self.rect.write(out);
        self.title.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            id: u64::read(bytes, pos)?,
            rect: PaneRect::read(bytes, pos)?,
            title: Option::<String>::read(bytes, pos)?,
        })
    }
}

impl Wire for PaneGrid {
    fn write(&self, out: &mut Vec<u8>) {
        self.pane.write(out);
        self.update.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            pane: u64::read(bytes, pos)?,
            update: GridUpdate::read(bytes, pos)?,
        })
    }
}

impl Wire for WorkspaceUpdate {
    fn write(&self, out: &mut Vec<u8>) {
        self.epoch.write(out);
        self.panes.write(out);
        self.focused.write(out);
        self.grids.write(out);
        self.border_style.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            epoch: u64::read(bytes, pos)?,
            panes: Vec::<PaneMeta>::read(bytes, pos)?,
            focused: u64::read(bytes, pos)?,
            grids: Vec::<PaneGrid>::read(bytes, pos)?,
            border_style: PaneBorderStyle::read(bytes, pos)?,
        })
    }
}

impl Wire for GridUpdate {
    fn write(&self, out: &mut Vec<u8>) {
        self.epoch.write(out);
        self.cols.write(out);
        self.rows.write(out);
        self.cursor.write(out);
        self.modes.write(out);
        self.scrollback.write(out);
        self.history.write(out);
        self.payload.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            epoch: u64::read(bytes, pos)?,
            cols: u16::read(bytes, pos)?,
            rows: u16::read(bytes, pos)?,
            cursor: Option::<CursorState>::read(bytes, pos)?,
            modes: TermModes::read(bytes, pos)?,
            scrollback: u32::read(bytes, pos)?,
            history: u32::read(bytes, pos)?,
            payload: GridPayload::read(bytes, pos)?,
        })
    }
}

impl Wire for TermModes {
    fn write(&self, out: &mut Vec<u8>) {
        self.alt_screen.write(out);
        self.app_cursor.write(out);
        self.mouse_mode.write(out);
        self.mouse_encoding.write(out);
        self.focus_reporting.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            alt_screen: bool::read(bytes, pos)?,
            app_cursor: bool::read(bytes, pos)?,
            mouse_mode: MouseMode::read(bytes, pos)?,
            mouse_encoding: MouseEncoding::read(bytes, pos)?,
            focus_reporting: bool::read(bytes, pos)?,
        })
    }
}

impl Wire for CursorState {
    fn write(&self, out: &mut Vec<u8>) {
        self.row.write(out);
        self.col.write(out);
        self.visible.write(out);
        self.shape.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            row: u16::read(bytes, pos)?,
            col: u16::read(bytes, pos)?,
            visible: bool::read(bytes, pos)?,
            shape: u8::read(bytes, pos)?,
        })
    }
}

impl Wire for GridRow {
    fn write(&self, out: &mut Vec<u8>) {
        self.cells.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            cells: Vec::<GridCell>::read(bytes, pos)?,
        })
    }
}

impl Wire for GridCell {
    fn write(&self, out: &mut Vec<u8>) {
        self.ch.write(out);
        self.extra.write(out);
        self.fg.write(out);
        self.bg.write(out);
        self.attrs.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(Self {
            ch: char::read(bytes, pos)?,
            extra: Vec::<char>::read(bytes, pos)?,
            fg: WireColor::read(bytes, pos)?,
            bg: WireColor::read(bytes, pos)?,
            attrs: u8::read(bytes, pos)?,
        })
    }
}

impl Wire for WireColor {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Default => u32::write(&0, out),
            Self::Indexed(n) => {
                u32::write(&1, out);
                n.write(out);
            }
            Self::Rgb(r, g, b) => {
                u32::write(&2, out);
                r.write(out);
                g.write(out);
                b.write(out);
            }
        }
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        match u32::read(bytes, pos)? {
            0 => Ok(Self::Default),
            1 => Ok(Self::Indexed(u8::read(bytes, pos)?)),
            2 => Ok(Self::Rgb(
                u8::read(bytes, pos)?,
                u8::read(bytes, pos)?,
                u8::read(bytes, pos)?,
            )),
            _ => Err(DecodeError::InvalidTag("WireColor")),
        }
    }
}

impl Wire for GridPayload {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Full(rows) => {
                u32::write(&0, out);
                rows.write(out);
            }
            Self::Rows(rows) => {
                u32::write(&1, out);
                rows.write(out);
            }
        }
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        match u32::read(bytes, pos)? {
            0 => Ok(Self::Full(Vec::<GridRow>::read(bytes, pos)?)),
            1 => Ok(Self::Rows(Vec::<(u16, GridRow)>::read(bytes, pos)?)),
            _ => Err(DecodeError::InvalidTag("GridPayload")),
        }
    }
}

impl Wire for TerminalColors {
    fn write(&self, out: &mut Vec<u8>) {
        self.background.0.write(out);
        self.background.1.write(out);
        self.background.2.write(out);
        self.foreground.0.write(out);
        self.foreground.1.write(out);
        self.foreground.2.write(out);
        self.palette.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        let bg0 = u8::read(bytes, pos)?;
        let bg1 = u8::read(bytes, pos)?;
        let bg2 = u8::read(bytes, pos)?;
        let fg0 = u8::read(bytes, pos)?;
        let fg1 = u8::read(bytes, pos)?;
        let fg2 = u8::read(bytes, pos)?;
        let palette = <[Option<(u8, u8, u8)>; 16]>::read(bytes, pos)?;
        Ok(Self {
            background: (bg0, bg1, bg2),
            foreground: (fg0, fg1, fg2),
            palette,
        })
    }
}

impl Wire for ClientToServer {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Attach {
                wire_version,
                cols,
                rows,
                cwd,
                shell,
                force,
                terminal_colors,
            } => {
                u32::write(&0, out);
                wire_version.write(out);
                cols.write(out);
                rows.write(out);
                cwd.write(out);
                shell.write(out);
                force.write(out);
                terminal_colors.write(out);
            }
            Self::Detach => u32::write(&1, out),
            Self::Resize { cols, rows } => {
                u32::write(&2, out);
                cols.write(out);
                rows.write(out);
            }
            Self::Key(bytes) => {
                u32::write(&3, out);
                bytes.write(out);
            }
            Self::Paste(bytes) => {
                u32::write(&4, out);
                bytes.write(out);
            }
            Self::SplitPane { direction } => {
                u32::write(&5, out);
                direction.write(out);
            }
            Self::FocusDirection { direction } => {
                u32::write(&6, out);
                direction.write(out);
            }
            Self::FocusPane { pane } => {
                u32::write(&7, out);
                pane.write(out);
            }
            Self::CloseFocusedPane => u32::write(&8, out),
            Self::Scroll { delta } => {
                u32::write(&9, out);
                delta.write(out);
            }
            Self::ScrollReset => u32::write(&10, out),
            Self::SearchScrollback { query } => {
                u32::write(&11, out);
                query.write(out);
            }
            Self::DumpScrollback => u32::write(&12, out),
            Self::KillCurrentSession => u32::write(&13, out),
            Self::KillSession(name) => {
                u32::write(&14, out);
                name.write(out);
            }
            Self::Ping => u32::write(&15, out),
            Self::Activate => u32::write(&16, out),
            Self::Inject { bytes } => {
                u32::write(&17, out);
                bytes.write(out);
            }
            Self::DumpSession => u32::write(&18, out),
        }
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(match u32::read(bytes, pos)? {
            0 => Self::Attach {
                wire_version: u32::read(bytes, pos)?,
                cols: u16::read(bytes, pos)?,
                rows: u16::read(bytes, pos)?,
                cwd: PathBuf::read(bytes, pos)?,
                shell: Option::<PathBuf>::read(bytes, pos)?,
                force: bool::read(bytes, pos)?,
                terminal_colors: Option::<TerminalColors>::read(bytes, pos)?,
            },
            1 => Self::Detach,
            2 => Self::Resize {
                cols: u16::read(bytes, pos)?,
                rows: u16::read(bytes, pos)?,
            },
            3 => Self::Key(Vec::<u8>::read(bytes, pos)?),
            4 => Self::Paste(Vec::<u8>::read(bytes, pos)?),
            5 => Self::SplitPane {
                direction: SplitDirection::read(bytes, pos)?,
            },
            6 => Self::FocusDirection {
                direction: Direction::read(bytes, pos)?,
            },
            7 => Self::FocusPane {
                pane: u64::read(bytes, pos)?,
            },
            8 => Self::CloseFocusedPane,
            9 => Self::Scroll {
                delta: i32::read(bytes, pos)?,
            },
            10 => Self::ScrollReset,
            11 => Self::SearchScrollback {
                query: String::read(bytes, pos)?,
            },
            12 => Self::DumpScrollback,
            13 => Self::KillCurrentSession,
            14 => Self::KillSession(String::read(bytes, pos)?),
            15 => Self::Ping,
            16 => Self::Activate,
            17 => Self::Inject {
                bytes: Vec::<u8>::read(bytes, pos)?,
            },
            18 => Self::DumpSession,
            _ => return Err(DecodeError::InvalidTag("ClientToServer")),
        })
    }
}

impl Wire for ServerToClient {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Attached {
                session_name,
                wire_version,
            } => {
                u32::write(&0, out);
                session_name.write(out);
                wire_version.write(out);
            }
            Self::AttachRejected(reason) => {
                u32::write(&1, out);
                reason.write(out);
            }
            Self::Workspace(ws) => {
                u32::write(&2, out);
                ws.write(out);
            }
            Self::Bell => u32::write(&3, out),
            Self::Exit(reason) => {
                u32::write(&4, out);
                reason.write(out);
            }
            Self::Pong => u32::write(&5, out),
            Self::Notice(notice) => {
                u32::write(&6, out);
                notice.write(out);
            }
            Self::Title(title) => {
                u32::write(&7, out);
                title.write(out);
            }
            Self::ClipboardCopy(data) => {
                u32::write(&8, out);
                data.write(out);
            }
            Self::Activate => u32::write(&9, out),
            Self::ActivateResult { delivered } => {
                u32::write(&10, out);
                delivered.write(out);
            }
            Self::SearchResults {
                pane,
                query,
                matches,
            } => {
                u32::write(&11, out);
                pane.write(out);
                query.write(out);
                matches.write(out);
            }
            Self::ScrollbackDump { pane, text } => {
                u32::write(&12, out);
                pane.write(out);
                text.write(out);
            }
        }
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok(match u32::read(bytes, pos)? {
            0 => Self::Attached {
                session_name: String::read(bytes, pos)?,
                wire_version: u32::read(bytes, pos)?,
            },
            1 => Self::AttachRejected(AttachRejectReason::read(bytes, pos)?),
            2 => Self::Workspace(WorkspaceUpdate::read(bytes, pos)?),
            3 => Self::Bell,
            4 => Self::Exit(ExitReason::read(bytes, pos)?),
            5 => Self::Pong,
            6 => Self::Notice(ServerNotice::read(bytes, pos)?),
            7 => Self::Title(String::read(bytes, pos)?),
            8 => Self::ClipboardCopy(Vec::<u8>::read(bytes, pos)?),
            9 => Self::Activate,
            10 => Self::ActivateResult {
                delivered: bool::read(bytes, pos)?,
            },
            11 => Self::SearchResults {
                pane: u64::read(bytes, pos)?,
                query: String::read(bytes, pos)?,
                matches: Vec::<SearchMatch>::read(bytes, pos)?,
            },
            12 => Self::ScrollbackDump {
                pane: u64::read(bytes, pos)?,
                text: String::read(bytes, pos)?,
            },
            _ => return Err(DecodeError::InvalidTag("ServerToClient")),
        })
    }
}
