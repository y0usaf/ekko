//! In-memory client state: the last-known grid, the grouped session list,
//! the active mode/overlay (registry-driven, extension-owned behavior), and
//! transient status notes. Pure data + small pure helpers; the event loop
//! owns mutation, `scene.rs` owns rendering.

use std::time::{Duration, Instant};

use ekko_ext::{ClientSnapshot, ModeState, NoteKind, OverlayState, ProjectGroup};
use ekko_grid::selection::{SelectionRange, TerminalSelection, selection_span};
use ekko_proto::{CursorState, GridCell, GridPayload, GridRow, GridUpdate, TermModes};

#[derive(Debug)]
pub struct StatusNote {
    pub text: String,
    pub kind: NoteKind,
    pub expires_at: Instant,
}

/// The open overlay, if any: its registry name plus its extension-owned,
/// type-erased state.
pub struct ActiveOverlay {
    pub name: String,
    pub state: OverlayState,
}

/// The last-known full grid contents, updated incrementally from `Rows`
/// payloads and wholesale from `Full` payloads.
#[derive(Clone, Debug, Default)]
pub struct GridState {
    pub epoch: u64,
    pub cols: u16,
    pub rows: u16,
    pub cursor: Option<CursorState>,
    /// Child-requested terminal modes (mouse, focus, alt screen, ...).
    pub modes: TermModes,
    /// Scrollback view offset in lines back from the live screen.
    pub scrollback: u32,
    pub cells: Vec<GridRow>,
}

impl GridState {
    /// Apply an update, dropping it if it's from an older epoch than what we
    /// already have (out-of-order/duplicate delivery). Returns `true` if
    /// applied.
    pub fn apply(&mut self, update: GridUpdate) -> bool {
        if self.cells.is_empty() {
            // First update: accept unconditionally, whatever the epoch.
        } else if update.epoch < self.epoch {
            return false;
        }
        self.epoch = update.epoch;
        self.cols = update.cols;
        self.rows = update.rows;
        self.cursor = update.cursor;
        self.modes = update.modes;
        self.scrollback = update.scrollback;
        match update.payload {
            GridPayload::Full(rows) => self.cells = rows,
            GridPayload::Rows(patches) => {
                for (index, row) in patches {
                    let index = index as usize;
                    if index >= self.cells.len() {
                        self.cells
                            .resize_with(index + 1, || GridRow { cells: Vec::new() });
                    }
                    self.cells[index] = row;
                }
            }
        }
        true
    }

    /// The text covered by `range`, rows joined with newlines and trailing
    /// blanks per row trimmed (the client-side sibling of
    /// `ekko_grid::selection::selected_text`, reading wire rows instead of a
    /// vt100 screen).
    pub fn selected_text(&self, range: SelectionRange) -> String {
        let mut lines = Vec::new();
        for row in range.start.row..=range.end.row {
            let Some((start, width)) = selection_span(Some(range), row, self.cols) else {
                lines.push(String::new());
                continue;
            };
            let mut line = String::new();
            for col in start..start + width {
                let Some(cell) = self
                    .cells
                    .get(row as usize)
                    .and_then(|r| r.cells.get(col as usize))
                else {
                    continue;
                };
                if cell.attrs & GridCell::WIDE_CONT != 0 {
                    continue;
                }
                line.push(cell.ch);
                line.extend(cell.extra.iter());
            }
            lines.push(line.trim_end().to_owned());
        }
        lines.join("\n")
    }
}

pub struct ClientState {
    pub session_name: String,
    pub grid: GridState,
    pub projects: Vec<ProjectGroup>,
    /// Active mode name; [`ClientSnapshot::NORMAL_MODE`] when no registered
    /// mode is intercepting input.
    pub mode: String,
    /// The active mode's extension-owned state (present iff a mode is active).
    pub mode_state: Option<ModeState>,
    pub overlay: Option<ActiveOverlay>,
    pub status_note: Option<StatusNote>,
    /// Mouse selection over the terminal pane, in grid-local coordinates.
    pub selection: TerminalSelection,
    pub dirty: bool,
}

impl ClientState {
    pub fn new(session_name: String) -> Self {
        Self {
            session_name,
            grid: GridState::default(),
            projects: Vec::new(),
            mode: ClientSnapshot::NORMAL_MODE.to_string(),
            mode_state: None,
            overlay: None,
            status_note: None,
            selection: TerminalSelection::default(),
            dirty: true,
        }
    }

    pub fn in_normal_mode(&self) -> bool {
        self.mode == ClientSnapshot::NORMAL_MODE
    }

    pub fn set_note(&mut self, text: impl Into<String>, kind: NoteKind, ttl: Duration) {
        self.status_note = Some(StatusNote {
            text: text.into(),
            kind,
            expires_at: Instant::now() + ttl,
        });
        self.dirty = true;
    }

    pub fn expire_note(&mut self) {
        if let Some(note) = &self.status_note
            && Instant::now() >= note.expires_at
        {
            self.status_note = None;
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_proto::{GridCell, WireColor};

    fn row(ch: char) -> GridRow {
        GridRow {
            cells: vec![GridCell {
                ch,
                extra: Vec::new(),
                fg: WireColor::Default,
                bg: WireColor::Default,
                attrs: 0,
            }],
        }
    }

    #[test]
    fn full_payload_replaces_all_rows() {
        let mut grid = GridState::default();
        assert!(grid.apply(GridUpdate {
            epoch: 1,
            cols: 1,
            rows: 2,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            payload: GridPayload::Full(vec![row('a'), row('b')]),
        }));
        assert_eq!(grid.cells.len(), 2);
        assert_eq!(grid.cells[0].cells[0].ch, 'a');
    }

    #[test]
    fn rows_payload_patches_in_place() {
        let mut grid = GridState::default();
        grid.apply(GridUpdate {
            epoch: 1,
            cols: 1,
            rows: 2,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            payload: GridPayload::Full(vec![row('a'), row('b')]),
        });
        grid.apply(GridUpdate {
            epoch: 2,
            cols: 1,
            rows: 2,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            payload: GridPayload::Rows(vec![(1, row('z'))]),
        });
        assert_eq!(grid.cells[0].cells[0].ch, 'a');
        assert_eq!(grid.cells[1].cells[0].ch, 'z');
    }

    #[test]
    fn stale_epoch_is_dropped() {
        let mut grid = GridState::default();
        grid.apply(GridUpdate {
            epoch: 5,
            cols: 1,
            rows: 1,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            payload: GridPayload::Full(vec![row('a')]),
        });
        let applied = grid.apply(GridUpdate {
            epoch: 3,
            cols: 1,
            rows: 1,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            payload: GridPayload::Full(vec![row('z')]),
        });
        assert!(!applied);
        assert_eq!(grid.cells[0].cells[0].ch, 'a');
    }
}
