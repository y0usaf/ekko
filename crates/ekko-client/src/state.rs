//! In-memory client state: the last-known pane workspace, the grouped
//! session list, the active mode/overlay (registry-driven, extension-owned
//! behavior), and transient status notes. Pure data + small pure helpers;
//! the event loop owns mutation, `scene.rs` owns rendering.

use std::time::{Duration, Instant};

use ekko_ext::{ClientSnapshot, ModeState, NoteKind, OverlayState, ProjectGroup, Rect};
use ekko_grid::selection::{SelectionPoint, SelectionRange, TerminalSelection, selection_span};
use ekko_proto::{
    CursorState, GridCell, GridPayload, GridRow, GridUpdate, PaneBorderStyle, PaneRect, TermModes,
    WorkspaceUpdate,
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
    /// Total scrollback history lines stored server-side (max offset).
    pub history: u32,
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
        self.history = update.history;
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
    /// How separators between/around panes should be drawn, as dictated by
    /// the daemon's config (geometry already reserves the cells).
    pub border_style: PaneBorderStyle,
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
        self.border_style = update.border_style;
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
    /// Active scrollback search: matches in absolute rows (0 = oldest
    /// history line), owned by `search.pane`. Cleared on pane close or
    /// `UiAction::SearchClear`.
    pub search: Option<SearchState>,
    /// While dragging a selection off the pane's top/bottom edge, the
    /// autoscroll direction (+1 = up into history, -1 = toward live).
    pub edge_scroll: Option<i32>,
    pub dirty: bool,
}

pub struct SearchState {
    pub pane: u64,
    pub matches: Vec<ekko_proto::SearchMatch>,
    /// Index into `matches` of the current (accented) hit.
    pub current: usize,
}

impl SearchState {
    /// The current match's viewport row for a pane showing `history` lines
    /// at view `offset`, and the scroll delta needed to reveal it. The
    /// viewport's first absolute row is `history - offset`.
    pub fn view_row(history: u32, offset: u32, abs_row: u32) -> Option<i32> {
        let first = i64::from(history) - i64::from(offset);
        let row = i64::from(abs_row) - first;
        i32::try_from(row).ok()
    }
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
            search: None,
            edge_scroll: None,
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
    // ── Selection gesture (spatiotemporal composition unit) ──────────────
    // The mouse selection is one composable unit: it mounts on left-press,
    // reads the mouse coordinate against its owning pane's rect (spatial),
    // and unmounts on left-release. Its context is `{ selection,
    // selection_pane, edge_scroll }`; unmounting reverts the transient
    // residue (drag flag, autoscroll) while the completed highlight
    // persists as the unit's observable output.

    /// Resolve a terminal-absolute mouse coordinate against the pane owning
    /// the active selection, clamped to its rect. Spatial read: an
    /// out-of-bounds coordinate (pointer left the window mid-drag) lands on
    /// the edge cell rather than being dropped, so the release still commits.
    pub fn selection_target(
        &self,
        terminal: Rect,
        col: i32,
        row: i32,
    ) -> Option<(u64, SelectionPoint)> {
        let pane_id = self.selection_pane?;
        let pane = self.workspace.panes.get(&pane_id)?;
        let local_col = (col - terminal.col - i32::from(pane.rect.x))
            .clamp(0, i32::from(pane.rect.cols).saturating_sub(1));
        let local_row = (row - terminal.row - i32::from(pane.rect.y))
            .clamp(0, i32::from(pane.rect.rows).saturating_sub(1));
        Some((
            pane_id,
            SelectionPoint {
                row: local_row as u16,
                col: local_col as u16,
            },
        ))
    }

    /// Mount the gesture: claim `pane` and anchor the selection at `point`.
    pub fn selection_begin(&mut self, pane: u64, point: SelectionPoint) {
        self.selection.set(point);
        self.selection_pane = Some(pane);
        self.edge_scroll = None;
    }

    /// Update the gesture's focus (spatial read), only while a gesture is
    /// active.
    pub fn selection_update(&mut self, point: SelectionPoint) {
        if self.selection.is_active() {
            self.selection.update_focus(point);
        }
    }

    /// Unmount the gesture: end the drag and revert the autoscroll residue.
    /// Returns the normalized range (None = plain click, no copy). The
    /// completed highlight persists as the unit's observable output.
    pub fn selection_end(&mut self) -> Option<SelectionRange> {
        self.selection.end_drag();
        self.edge_scroll = None;
        self.selection.normalized()
    }

    /// Abort the gesture, reverting every committed mutation (inverse
    /// replay): clear the selection, release the pane, drop autoscroll.
    pub fn selection_abort(&mut self) {
        self.selection.clear();
        self.selection_pane = None;
        self.edge_scroll = None;
    }
    // ── Scroll-view attach (spatiotemporal composition unit) ───────────────
    // The scroll view attaches a search marker to its pane: mounting pins
    // the absolute-row match set plus the current hit on the pane; jumping
    // advances composition; leaving reverts the transient marker via
    // `scroll_end` (commit) / `scroll_abort` (cancel full inverse). The
    // completed scroll position/revealed match persists as observable
    // output.

    /// Mount the scroll-view marker on `pane` starting at match `first`.
    /// A mounted marker pins (`pane`, `matches`, `current`) with its
    /// absolute-row indexing. Rejects (and clears) when the pane is gone or
    /// no matches remain — never a half-mounted unit.
    pub fn scroll_attach(
        &mut self,
        pane: u64,
        matches: Vec<ekko_proto::SearchMatch>,
        first: usize,
    ) -> Option<()> {
        if !self.workspace.panes.contains_key(&pane) || matches.is_empty() {
            // Unmount-at-stale-mount: a missing pane or empty result set
            // means there is nothing to pin; revert to pre-scroll state now.
            self.search = None;
            return None;
        }
        let first = first.min(matches.len() - 1);
        self.search = Some(SearchState {
            pane,
            matches,
            current: first,
        });
        self.dirty = true;
        Some(())
    }

    /// Advance the scroll-marker one hit forward (`forward = true`) or back
    /// (`false`), wrapping around the match list (composition exercise).
    /// Returns `false` when no marker is mounted.
    pub fn scroll_jump(&mut self, forward: bool) -> bool {
        let Some(search) = &mut self.search else {
            return false;
        };
        let len = search.matches.len();
        if len == 0 {
            self.scroll_abort();
            return false;
        }
        search.current = if forward {
            (search.current + 1) % len
        } else {
            (search.current + len - 1) % len
        };
        self.dirty = true;
        true
    }

    /// Finish composition: the matched hit is committed to the pane's view
    /// (observable output) while the transient marker reverts. Returns the
    /// pane and the finalized hit so the caller can keep the view on it.
    ///
    /// Ceremony-observation surface: the host's live scroll paths resolve
    /// through the same state via the single write path; this aside lives for
    /// the spatiotemporal ceremony tests that pin the `_end` unmount contract.
    #[allow(dead_code)] // sp-temporal ceremony observation; test consumer.
    pub fn scroll_end(&mut self) -> Option<(u64, ekko_proto::SearchMatch)> {
        let hit = self
            .search
            .as_ref()
            .and_then(|s| s.matches.get(s.current).copied())
            .map(|m| (self.search.as_ref().map_or(0, |s| s.pane), m));
        self.search = None;
        self.dirty = true;
        hit
    }

    /// Cancel the scroll-view marker in full: clear the search and leave the
    /// pane riding the live scroll, reverting every committed mutation.
    pub fn scroll_abort(&mut self) {
        self.search = None;
        self.dirty = true;
    }
    // ── Transient overlay (spatiotemporal composition unit) ────────────────
    // The overlay unit mounts an [`ActiveOverlay`] (its registry name plus
    // extension-owned, type-erased state). Any close is the unmount inverse
    // reverting it to `None`; the extension-owned state dies with it.

    /// Mount the overlay. Drops any prior overlay (a modal overlay is
    /// single-slot: mounting replaces, never stacks — the replaced one's
    /// inverse runs by assignment).
    pub fn overlay_open(&mut self, overlay: ActiveOverlay) {
        self.overlay = Some(overlay);
        self.dirty = true;
    }

    /// Unmount the overlay: revert to the pre-mount `None` (full inverse).
    pub fn overlay_close(&mut self) {
        self.overlay = None;
        self.dirty = true;
    }

    /// Whether the overlay unit is currently mounted.
    #[allow(dead_code)] // spatiotemporal ceremony observation; test consumer.
    pub fn overlay_active(&self) -> bool {
        self.overlay.is_some()
    }
    // ── Gate-dispatched transient-owned progress (composition unit) ────────
    // A gate/lifecycle handler runs thread-per-dispatch through the runtime
    // and reports progress by returning `UiAction::SetStatusNote`; the client
    // mounts the resulting [`StatusNote`] through the single write path here
    // and reverts it (expiry/clear) when the operation completes — the
    // transient is owned by the dispatched operation, not the host.

    /// Mount the progress note (a gate handler's `SetStatusNote` resolved by
    /// `apply_ui_action`).
    #[allow(dead_code)] // spatiotemporal ceremony observation; test consumer.
    pub fn progress_begin(&mut self, text: impl Into<String>, kind: NoteKind, ttl: Duration) {
        self.set_note(text, kind, ttl);
    }

    /// The progress residue: a note is currently mounted.
    #[allow(dead_code)] // spatiotemporal ceremony observation; test consumer.
    pub fn progress_active(&self) -> bool {
        self.status_note.is_some()
    }

    /// Unmount the progress unit NOW (the operation completed): revert the
    /// note regardless of the pending TTL (time-based inverse forced early).
    #[allow(dead_code)] // spatiotemporal ceremony observation; test consumer.
    pub fn progress_end(&mut self) {
        self.status_note = None;
        self.dirty = true;
    }

    /// Cancel the progress: same full inverse as `progress_end`, named to
    /// mirror the selection/`scroll_abort` cancel path.
    #[allow(dead_code)] // spatiotemporal ceremony observation; test consumer.
    pub fn progress_abort(&mut self) {
        self.progress_end();
    }
}

#[cfg(test)]
mod search_tests {
    use super::SearchState;

    #[test]
    fn match_rows_map_into_the_viewport_from_absolute_coordinates() {
        // 100 history lines, offset 10: the viewport's first absolute row
        // is 90; absolute row 95 shows as viewport row 5.
        assert_eq!(SearchState::view_row(100, 10, 95), Some(5));
        // Rows above/below the window map to out-of-pane rows.
        assert_eq!(SearchState::view_row(100, 10, 89), Some(-1));
        assert_eq!(SearchState::view_row(100, 10, 110), Some(20));
        // Live view (offset 0): history row 95 of 100 is 5 rows up.
        assert_eq!(SearchState::view_row(100, 0, 95), Some(-5));
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
            history: 0,
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
            history: 0,
            payload: GridPayload::Full(vec![row('a'), row('b')]),
        });
        grid.apply(GridUpdate {
            epoch: 2,
            cols: 1,
            rows: 2,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            history: 0,
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
            history: 0,
            payload: GridPayload::Full(vec![row('a')]),
        });
        let applied = grid.apply(GridUpdate {
            epoch: 3,
            cols: 1,
            rows: 1,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            history: 0,
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
            history: 0,
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
            border_style: PaneBorderStyle::None,
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

#[cfg(test)]
mod selection_gesture_tests {
    use super::ClientState;
    use ekko_ext::Rect;
    use ekko_grid::selection::SelectionPoint;
    use ekko_proto::PaneRect;

    fn state_with_pane() -> ClientState {
        let mut state = ClientState::new("s".into());
        state.workspace.panes.insert(
            0,
            crate::state::PaneState {
                rect: PaneRect {
                    x: 0,
                    y: 0,
                    cols: 10,
                    rows: 5,
                },
                ..Default::default()
            },
        );
        state
    }

    #[test]
    fn selection_target_clamps_out_of_bounds_coordinates() {
        let mut state = state_with_pane();
        state.selection_begin(0, SelectionPoint { row: 0, col: 0 });
        let terminal = Rect {
            col: 0,
            row: 0,
            cols: 10,
            rows: 5,
        };
        // Pointer left the window far above-left: clamp to the top-left edge.
        let (pane, point) = state.selection_target(terminal, 999, -50).unwrap();
        assert_eq!(pane, 0);
        assert_eq!(point, SelectionPoint { row: 0, col: 9 });
        // Far below: clamp to the bottom edge.
        let (_, point) = state.selection_target(terminal, 5, 999).unwrap();
        assert_eq!(point, SelectionPoint { row: 4, col: 5 });
    }

    #[test]
    fn selection_end_reverts_transient_residue() {
        let mut state = state_with_pane();
        state.selection_begin(0, SelectionPoint { row: 0, col: 0 });
        state.selection_update(SelectionPoint { row: 2, col: 3 });
        state.edge_scroll = Some(1);
        let range = state.selection_end().unwrap();
        assert_eq!(range.start, SelectionPoint { row: 0, col: 0 });
        assert_eq!(range.end, SelectionPoint { row: 2, col: 4 });
        assert_eq!(state.edge_scroll, None); // transient residue reverted
        assert!(state.selection.is_active()); // completed highlight persists
    }

    #[test]
    fn selection_abort_reverts_everything() {
        let mut state = state_with_pane();
        state.selection_begin(0, SelectionPoint { row: 1, col: 1 });
        state.edge_scroll = Some(-1);
        state.selection_abort();
        assert!(!state.selection.is_active());
        assert_eq!(state.selection_pane, None);
        assert_eq!(state.edge_scroll, None);
    }
}

#[cfg(test)]
mod transient_ceremony_tests {
    //! Spatiotemporal ceremony for the client transient units (scroll-view
    //! attach, transient overlay, gate-dispatched transient-owned progress).
    //! Each follows the model mouse unit's timing: snapshot the context,
    //! mount on trigger, exercise the effects, unmount, then diff the
    //! context against the snapshot — any residue beyond the completed
    //! observable output is a bug.
    use super::{ActiveOverlay, ClientState};
    use crate::state::PaneState;
    use ekko_ext::NoteKind;
    use ekko_proto::{PaneRect, SearchMatch};
    use std::time::Duration;

    fn state_with_pane() -> ClientState {
        let mut state = ClientState::new("s".into());
        state.workspace.panes.insert(
            0,
            PaneState {
                rect: PaneRect {
                    x: 0,
                    y: 0,
                    cols: 10,
                    rows: 5,
                },
                ..Default::default()
            },
        );
        state
    }

    /// The scroll-view residue measure: only `search` and `dirty` move;
    /// everything else is context to be left untouched.
    fn scroll_snapshot(state: &ClientState) -> (Option<usize>, bool) {
        (state.search.as_ref().map(|s| s.current), state.dirty)
    }

    #[test]
    fn scroll_attach_mounts_and_end_commits_without_residue() {
        let mut state = state_with_pane();
        let matches = vec![
            SearchMatch {
                row: 2,
                col: 0,
                len: 1,
            },
            SearchMatch {
                row: 7,
                col: 1,
                len: 3,
            },
        ];

        // mount (on a wheel/search trigger) → pin the match set on pane 0.
        assert!(state.scroll_attach(0, matches.clone(), 0).is_some());
        assert!(state.search.is_some());
        assert_eq!(state.search.as_ref().unwrap().pane, 0);
        assert_eq!(state.search.as_ref().unwrap().current, 0);

        // exercise → jump forward wraps to the next hit.
        assert!(state.scroll_jump(true));
        assert_eq!(state.search.as_ref().unwrap().current, 1);
        // exercise → jump back wraps to the first hit.
        assert!(state.scroll_jump(false));
        assert_eq!(state.search.as_ref().unwrap().current, 0);

        // unmount (end) → commit the finalized hit, revert the marker.
        let committed = state.scroll_end().unwrap();
        assert_eq!(committed.0, 0);
        assert_eq!(committed.1, matches[0]);
        assert_eq!(
            scroll_snapshot(&state),
            (None, true),
            "marker residue after end"
        );
        assert!(state.search.is_none());
        assert!(state.dirty, "a repaint is still bound (observable present)");
        // No residue beyond context returning to an unattached, dirty scroll.
    }

    #[test]
    fn scroll_attach_reverts_everything_on_abort() {
        let mut state = state_with_pane();
        let matches = vec![SearchMatch {
            row: 2,
            col: 0,
            len: 1,
        }];
        let before = scroll_snapshot(&state);
        let _ = state.scroll_attach(0, matches.clone(), 0);
        assert!(state.scroll_jump(true));
        // unmount (cancel) → full inverse: nothing left of the marker.
        state.scroll_abort();
        assert!(state.search.is_none());
        assert_eq!(scroll_snapshot(&state).0, before.0);
        assert!(state.dirty, "repaint bound by the abort");
    }

    #[test]
    fn scroll_attach_mounts_stale_free() {
        // Unmount at stale-free mount: a marker can never be pinned to a pane
        // that isn't in the workspace, and an empty match set never half-sticks.
        let mut state = state_with_pane();
        let matches = vec![SearchMatch {
            row: 2,
            col: 0,
            len: 1,
        }];
        assert!(state.scroll_attach(0, vec![], 0).is_none());
        assert!(state.search.is_none());
        assert!(state.scroll_attach(99, matches.clone(), 0).is_none());
        assert!(state.search.is_none());
    }

    #[test]
    fn overlay_mounts_and_unmounts_leaving_no_residue() {
        let mut state = state_with_pane();
        assert!(!state.overlay_active());

        // mount → the overlay opens.
        state.overlay_open(ActiveOverlay {
            name: "help".into(),
            state: Box::new(7usize),
        });
        assert!(state.overlay_active());
        assert_eq!(state.overlay.as_ref().unwrap().name, "help");

        // exercise → the overlay owns the input (name/state present).
        let opaque = &state.overlay.as_ref().unwrap().state;
        assert_eq!((**opaque).downcast_ref::<usize>(), Some(&7));

        // unmount → full inverse back to None.
        state.overlay_close();
        assert!(!state.overlay_active());
        assert!(state.overlay.is_none());
    }

    #[test]
    fn progress_note_mounts_and_unmount_clears_residue() {
        // A gate-dispatched operation owns a StatusNote transient: mounted on
        // its first SetStatusNote, and reverted when it completes.
        let mut state = state_with_pane();
        assert!(!state.progress_active());

        state.progress_begin("searching…", NoteKind::Info, Duration::from_millis(500));
        assert!(state.progress_active());
        assert_eq!(state.status_note.as_ref().unwrap().text, "searching…");

        // unmount (operation completed / canceled): note reverts, no residue.
        state.progress_end();
        assert!(!state.progress_active());
        assert!(state.status_note.is_none());
        assert!(state.dirty, "repaint bound by the note lifecycle");
    }
}
