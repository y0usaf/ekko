//! Mouse selection over the embedded terminal (port of pi-harness
//! `terminal::selection`): inclusive screen-cell anchor/focus points normalized
//! into a half-open row-major range.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionPoint {
    pub row: u16,
    pub col: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionRange {
    pub start: SelectionPoint,
    pub end: SelectionPoint,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalSelection {
    anchor: Option<SelectionPoint>,
    focus: Option<SelectionPoint>,
    /// `true` while the user is actively dragging (mouse-down → drag → mouse-up).
    /// When `dragging`, `shift_rows` only moves the anchor so the focus tracks
    /// the cursor (zellij's `active`-flag pattern). Once the drag ends
    /// (`end_drag`), both endpoints shift together so the completed selection
    /// persists as the viewport scrolls.
    dragging: bool,
}

impl TerminalSelection {
    pub fn clear(&mut self) {
        self.anchor = None;
        self.focus = None;
        self.dragging = false;
    }

    pub fn set(&mut self, point: SelectionPoint) {
        self.anchor = Some(point);
        self.focus = Some(point);
        self.dragging = true;
    }

    pub fn update_focus(&mut self, point: SelectionPoint) {
        self.focus = Some(point);
    }

    /// Mark the drag as finished (mouse-up). After this, `shift_rows` moves
    /// both endpoints together so the completed selection tracks its content
    /// as the viewport scrolls.
    pub fn end_drag(&mut self) {
        self.dragging = false;
    }

    pub fn is_active(&self) -> bool {
        self.anchor.is_some()
    }

    /// Shift the selection's row coordinates by `delta` (in visible rows)
    /// so the highlight tracks the same content when the viewport scrolls,
    /// mirroring zellij's `move_up`/`move_down`.
    ///
    /// While actively dragging, only the anchor moves (the focus stays at the
    /// cursor). Once the drag is finished, both endpoints shift together.
    pub fn shift_rows(&mut self, delta: i32) {
        let shift = |p: &mut SelectionPoint| {
            p.row = (i32::from(p.row) + delta).max(0).min(i32::from(u16::MAX)) as u16;
        };
        if delta == 0 {
            return;
        }
        if let Some(anchor) = &mut self.anchor {
            shift(anchor);
        }
        if !self.dragging
            && let Some(focus) = &mut self.focus
        {
            shift(focus);
        }
    }

    /// Normalize the two hovered cells into a half-open range.
    ///
    /// Both cells are included regardless of drag direction. Keeping this
    /// conversion here avoids the direction-dependent `+ 1` adjustment that
    /// otherwise drops column zero when a drag moves right-to-left.
    pub fn normalized(&self) -> Option<SelectionRange> {
        let (anchor, focus) = (self.anchor?, self.focus?);
        if anchor == focus {
            return None;
        }

        let (start, mut end) = if (anchor.row, anchor.col) <= (focus.row, focus.col) {
            (anchor, focus)
        } else {
            (focus, anchor)
        };
        end.col = end.col.saturating_add(1);
        Some(SelectionRange { start, end })
    }
}

/// The selected `(start_col, width)` of `row`, if any.
pub fn selection_span(
    selection: Option<SelectionRange>,
    row: u16,
    cols: u16,
) -> Option<(u16, u16)> {
    let selection = selection?;
    if row < selection.start.row || row > selection.end.row {
        return None;
    }

    let row_start = if row == selection.start.row {
        selection.start.col.min(cols)
    } else {
        0
    };
    let row_end = if row == selection.end.row {
        selection.end.col.min(cols)
    } else {
        cols
    };
    (row_start < row_end).then_some((row_start, row_end - row_start))
}

/// The selected text of a vt100 screen, rows joined with newlines and
/// trailing blanks per row trimmed.
pub fn selected_text(screen: &vt100::Screen, range: SelectionRange) -> String {
    let (_, cols) = screen.size();
    let mut lines = Vec::new();
    for row in range.start.row..=range.end.row {
        let Some((start, width)) = selection_span(Some(range), row, cols) else {
            lines.push(String::new());
            continue;
        };
        let mut line = String::new();
        let mut col = start;
        while col < start + width {
            match screen.cell(row, col) {
                Some(cell) if cell.is_wide_continuation() => {}
                Some(cell) if cell.contents().is_empty() => line.push(' '),
                Some(cell) => line.push_str(cell.contents()),
                None => {}
            }
            col += 1;
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(row: u16, col: u16) -> SelectionPoint {
        SelectionPoint { row, col }
    }

    #[test]
    fn normalized_orders_endpoints_and_includes_both_cells() {
        let mut selection = TerminalSelection::default();
        selection.set(point(2, 5));
        assert_eq!(selection.normalized(), None);
        selection.update_focus(point(1, 3));
        let range = selection.normalized().unwrap();
        assert_eq!(range.start, point(1, 3));
        assert_eq!(range.end, point(2, 6));
    }

    #[test]
    fn backward_drag_to_line_start_keeps_column_zero() {
        let mut selection = TerminalSelection::default();
        selection.set(point(0, 5));
        selection.update_focus(point(0, 0));

        let range = selection.normalized().unwrap();
        assert_eq!(
            range,
            SelectionRange {
                start: point(0, 0),
                end: point(0, 6)
            }
        );
        assert_eq!(selection_span(Some(range), 0, 10), Some((0, 6)));
    }

    #[test]
    fn forward_drag_from_line_start_keeps_column_zero_across_rows() {
        let mut selection = TerminalSelection::default();
        selection.set(point(0, 0));
        selection.update_focus(point(1, 2));

        let range = selection.normalized().unwrap();
        assert_eq!(
            range,
            SelectionRange {
                start: point(0, 0),
                end: point(1, 3)
            }
        );
        assert_eq!(selection_span(Some(range), 0, 10), Some((0, 10)));
        assert_eq!(selection_span(Some(range), 1, 10), Some((0, 3)));
    }

    #[test]
    fn span_covers_full_middle_rows() {
        let range = SelectionRange {
            start: point(1, 4),
            end: point(3, 2),
        };
        assert_eq!(selection_span(Some(range), 0, 10), None);
        assert_eq!(selection_span(Some(range), 1, 10), Some((4, 6)));
        assert_eq!(selection_span(Some(range), 2, 10), Some((0, 10)));
        assert_eq!(selection_span(Some(range), 3, 10), Some((0, 2)));
    }

    #[test]
    fn selected_text_extracts_screen_contents() {
        let mut parser = vt100::Parser::new(3, 10, 0);
        parser.process(b"hello\r\nworld");
        let text = selected_text(
            parser.screen(),
            SelectionRange {
                start: point(0, 0),
                end: point(1, 5),
            },
        );
        assert_eq!(text, "hello\nworld");
    }

    #[test]
    fn shift_rows_moves_both_endpoints_when_not_dragging() {
        let mut selection = TerminalSelection::default();
        selection.set(point(2, 0));
        selection.update_focus(point(4, 5));
        selection.end_drag();
        selection.shift_rows(3);
        let range = selection.normalized().unwrap();
        assert_eq!(range.start, point(5, 0));
        assert_eq!(range.end, point(7, 6));
    }

    #[test]
    fn shift_rows_moves_only_anchor_while_dragging() {
        let mut selection = TerminalSelection::default();
        selection.set(point(2, 0));
        selection.update_focus(point(4, 5));
        // Still dragging -- only the anchor should move.
        selection.shift_rows(3);
        let range = selection.normalized().unwrap();
        assert_eq!(range.start, point(4, 5)); // focus stayed (smaller)
        assert_eq!(range.end, point(5, 1)); // shifted anchor cell is included
    }

    #[test]
    fn shift_rows_clamps_to_zero() {
        let mut selection = TerminalSelection::default();
        selection.set(point(1, 0));
        selection.update_focus(point(3, 5));
        selection.end_drag();
        selection.shift_rows(-10);
        let range = selection.normalized().unwrap();
        assert_eq!(range.start, point(0, 0));
        assert_eq!(range.end, point(0, 6));
    }

    #[test]
    fn shift_rows_noop_on_zero_delta() {
        let mut selection = TerminalSelection::default();
        selection.set(point(2, 0));
        selection.update_focus(point(4, 5));
        selection.end_drag();
        selection.shift_rows(0);
        let range = selection.normalized().unwrap();
        assert_eq!(range.start, point(2, 0));
        assert_eq!(range.end, point(4, 6));
    }

    #[test]
    fn clear_resets_dragging() {
        let mut selection = TerminalSelection::default();
        selection.set(point(2, 0));
        assert!(selection.is_active());
        selection.clear();
        assert!(!selection.is_active());
        // After clear + set again, dragging should be true.
        selection.set(point(1, 0));
        selection.update_focus(point(3, 5));
        selection.shift_rows(2);
        // dragging is true, so only anchor moves: anchor (1,0) -> (3,0).
        // Focus stays at (3,5); the half-open end includes that cell.
        let range = selection.normalized().unwrap();
        assert_eq!(range.start, point(3, 0));
        assert_eq!(range.end, point(3, 6));
    }
}
