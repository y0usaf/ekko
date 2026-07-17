//! In-memory client state: the last-known pane workspace, the grouped
//! session list, the active mode/overlay (registry-driven, extension-owned
//! behavior), and transient status notes. Pure data + small pure helpers;
//! the event loop owns mutation, `scene.rs` owns rendering.

use std::time::{Duration, Instant};

use ekko_ext::{ClientSnapshot, ModeState, NoteKind, OverlayState, ProjectGroup};
use ekko_grid::selection::{SelectionRange, TerminalSelection, selection_span};
use ekko_proto::{
    CursorState, GridCell, GridPayload, GridRow, GridUpdate, PaneRect, TermModes, WorkspaceUpdate,
};

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

/// The last-known state of one tiled pane: its server-provided canvas rect
/// plus the grid cache updated incrementally from that pane's payloads.
#[derive(Clone, Debug, Default)]
pub struct PaneState {
    pub rect: PaneRect,
    pub title: Option<String>,
    pub grid: GridState,
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

/// The discardable client-side cache of the daemon's pane workspace. Pane
/// metadata arrives complete on every frame (topology recovery is
/// idempotent); per-pane grids patch incrementally. The daemon owns the
/// canonical topology; this map is rebuilt from each [`WorkspaceUpdate`].
#[derive(Clone, Debug, Default)]
pub struct WorkspaceState {
    pub epoch: u64,
    pub panes: std::collections::BTreeMap<u64, PaneState>,
    /// This client's focused pane, as last confirmed by the daemon.
    pub focused: u64,
}

impl WorkspaceState {
    /// Apply a workspace frame, dropping it wholesale if it is older than
    /// the last applied epoch (out-of-order/duplicate delivery). Panes
    /// absent from the frame's metadata are dropped; grids addressed to
    /// unknown panes are ignored. Returns `true` if applied.
    pub fn apply(&mut self, update: WorkspaceUpdate) -> bool {
        if self.panes.is_empty() {
            // First frame: accept unconditionally, whatever the epoch.
        } else if update.epoch < self.epoch {
            return false;
        }
        self.epoch = update.epoch;
        self.focused = update.focused;
        self.panes
            .retain(|id, _| update.panes.iter().any(|meta| meta.id == *id));
        for meta in update.panes {
            let entry = self.panes.entry(meta.id).or_default();
            entry.rect = meta.rect;
            entry.title = meta.title;
        }
        for grid in update.grids {
            if let Some(entry) = self.panes.get_mut(&grid.pane) {
                entry.grid.apply(grid.update);
            }
        }
        true
    }

    /// The focused pane's grid, if the frame's focus names a live pane.
    pub fn focused_grid(&self) -> Option<&GridState> {
        self.panes.get(&self.focused).map(|pane| &pane.grid)
    }
}

pub struct ClientState {
    pub session_name: String,
    pub workspace: WorkspaceState,
    pub projects: Vec<ProjectGroup>,
    /// Active mode name; [`ClientSnapshot::NORMAL_MODE`] when no registered
    /// mode is intercepting input.
    pub mode: String,
    /// The active mode's extension-owned state (present iff a mode is active).
    pub mode_state: Option<ModeState>,
    pub overlay: Option<ActiveOverlay>,
    pub status_note: Option<StatusNote>,
    /// Mouse selection over a terminal pane, in pane-local coordinates.
    pub selection: TerminalSelection,
    /// The pane `selection` belongs to; highlights render only there.
    pub selection_pane: Option<u64>,
    /// Surfaces toggled off by `UiAction::ToggleSurface`. Client-local:
    /// a fresh attach starts with everything visible.
    pub hidden_surfaces: std::collections::HashSet<String>,
    pub dirty: bool,
}

impl ClientState {
    pub fn new(session_name: String) -> Self {
        Self {
            session_name,
            workspace: WorkspaceState::default(),
            projects: Vec::new(),
            mode: ClientSnapshot::NORMAL_MODE.to_string(),
            mode_state: None,
            overlay: None,
            status_note: None,
            selection: TerminalSelection::default(),
            selection_pane: None,
            hidden_surfaces: std::collections::HashSet::new(),
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

    fn grid_update(epoch: u64, payload: GridPayload) -> GridUpdate {
        GridUpdate {
            epoch,
            cols: 1,
            rows: 2,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            payload,
        }
    }

    fn workspace(
        epoch: u64,
        panes: &[u64],
        focused: u64,
        grids: Vec<(u64, GridUpdate)>,
    ) -> WorkspaceUpdate {
        WorkspaceUpdate {
            epoch,
            panes: panes
                .iter()
                .map(|&id| ekko_proto::PaneMeta {
                    id,
                    rect: ekko_proto::PaneRect {
                        x: 0,
                        y: 0,
                        cols: 1,
                        rows: 2,
                    },
                    title: None,
                })
                .collect(),
            focused,
            grids: grids
                .into_iter()
                .map(|(pane, update)| ekko_proto::PaneGrid { pane, update })
                .collect(),
        }
    }

    #[test]
    fn workspace_patches_panes_independently() {
        let mut state = WorkspaceState::default();
        assert!(state.apply(workspace(
            1,
            &[1, 2],
            2,
            vec![
                (
                    1,
                    grid_update(1, GridPayload::Full(vec![row('a'), row('b')]))
                ),
                (
                    2,
                    grid_update(1, GridPayload::Full(vec![row('c'), row('d')]))
                ),
            ],
        )));
        assert_eq!(state.focused, 2);
        assert_eq!(state.focused_grid().unwrap().cells[0].cells[0].ch, 'c');

        // A later frame touches only pane 1; pane 2's cache persists.
        assert!(state.apply(workspace(
            2,
            &[1, 2],
            1,
            vec![(1, grid_update(2, GridPayload::Rows(vec![(1, row('z'))])))],
        )));
        assert_eq!(state.panes[&1].grid.cells[1].cells[0].ch, 'z');
        assert_eq!(state.panes[&2].grid.cells[0].cells[0].ch, 'c');
        assert_eq!(state.focused_grid().unwrap().cells[0].cells[0].ch, 'a');
    }

    #[test]
    fn workspace_metadata_removal_drops_the_panes_cache() {
        let mut state = WorkspaceState::default();
        state.apply(workspace(
            1,
            &[1, 2],
            2,
            vec![
                (1, grid_update(1, GridPayload::Full(vec![row('a')]))),
                (2, grid_update(1, GridPayload::Full(vec![row('b')]))),
            ],
        ));
        assert!(state.apply(workspace(
            2,
            &[1],
            1,
            vec![(2, grid_update(2, GridPayload::Full(vec![row('x')])))],
        )));
        assert!(!state.panes.contains_key(&2), "removed pane's cache drops");
        assert_eq!(state.panes[&1].grid.cells[0].cells[0].ch, 'a');
        assert_eq!(state.focused, 1);
    }

    #[test]
    fn workspace_stale_epoch_drops_the_whole_frame() {
        let mut state = WorkspaceState::default();
        state.apply(workspace(
            5,
            &[1],
            1,
            vec![(1, grid_update(5, GridPayload::Full(vec![row('a')])))],
        ));
        assert!(!state.apply(workspace(
            3,
            &[1],
            1,
            vec![(1, grid_update(3, GridPayload::Full(vec![row('z')])))],
        )));
        assert_eq!(state.epoch, 5);
        assert_eq!(state.panes[&1].grid.cells[0].cells[0].ch, 'a');
    }

    #[test]
    fn one_pane_workspace_matches_the_singular_grid_contract() {
        let mut state = WorkspaceState::default();
        state.apply(workspace(
            1,
            &[1],
            1,
            vec![(
                1,
                grid_update(1, GridPayload::Full(vec![row('a'), row('b')])),
            )],
        ));
        state.apply(workspace(
            2,
            &[1],
            1,
            vec![(1, grid_update(2, GridPayload::Rows(vec![(1, row('z'))])))],
        ));
        let grid = state.focused_grid().unwrap();
        assert_eq!(grid.cells[0].cells[0].ch, 'a');
        assert_eq!(grid.cells[1].cells[0].ch, 'z');
        assert_eq!(grid.cols, 1);
        assert_eq!(grid.rows, 2);
    }
}
