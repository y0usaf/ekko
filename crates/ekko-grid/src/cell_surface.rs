//! Cell grid the mux composites every frame into, diffed against the previous
//! frame by the ANSI renderer.
//!
//! Uses interred text + packed cells (Priority 1) and spatial damage tracking
//! (Priority 2) from the renderer optimization plan. Each cell is 16 bytes of
//! stack data with zero per-cell heap allocations; drawing primitives mark a
//! `DirtyRegion` so the diff loop can skip clean regions.

#![allow(clippy::too_many_arguments)]

use crate::color::Color;
use crate::damage::DirtyRegion;
use crate::layout::CellRect;
use crate::packed::{PackedCell, PackedStyle, PackedText, StringInterner};

#[derive(Clone)]
pub struct CellSurface {
    pub cols: i32,
    pub rows: i32,
    pub cells: Vec<PackedCell>,
    pub interner: StringInterner,
    pub dirty_region: DirtyRegion,
    pub dirty_all: bool,
}

impl Default for CellSurface {
    fn default() -> Self {
        Self::new(1, 1, Color::default(), Color::default())
    }
}

impl CellSurface {
    pub fn new(cols: i32, rows: i32, fg: Color, bg: Color) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let blank = blank_packed(fg, bg);
        Self {
            cols,
            rows,
            cells: vec![blank; (cols * rows) as usize],
            interner: StringInterner::default(),
            dirty_region: DirtyRegion::new(),
            dirty_all: true,
        }
    }

    /// Resize in place, preserving overlapping content. Marks everything
    /// dirty so the next render is a full repaint. A same-size call is a
    /// no-op: callers invoke this every frame, and reallocating (and worse,
    /// setting `dirty_all`) would force a full repaint per frame, defeating
    /// the damage tracking entirely.
    pub fn resize(&mut self, cols: i32, rows: i32, fg: Color, bg: Color) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        let blank = blank_packed(fg, bg);
        let mut new_cells = vec![blank; (cols * rows) as usize];
        let copy_rows = rows.min(self.rows);
        let copy_cols = cols.min(self.cols);
        for row in 0..copy_rows {
            let src_start = (row * self.cols) as usize;
            let dst_start = (row * cols) as usize;
            let copy_len = copy_cols as usize;
            new_cells[dst_start..dst_start + copy_len]
                .copy_from_slice(&self.cells[src_start..src_start + copy_len]);
        }
        self.cols = cols;
        self.rows = rows;
        self.cells = new_cells;
        self.dirty_all = true;
    }

    #[inline]
    fn index(&self, col: i32, row: i32) -> Option<usize> {
        (col >= 0 && row >= 0 && col < self.cols && row < self.rows)
            .then_some((row * self.cols + col) as usize)
    }

    /// Write a packed cell at `index` (logical `col`,`row`), marking it dirty
    /// only when the content actually changes. This is what makes the
    /// `DirtyRegion` accurate: unchanged writes are no-ops, so the diff loop
    /// only visits cells whose final value differs from the previous frame.
    #[inline]
    fn write_cell(&mut self, index: usize, col: i32, row: i32, cell: PackedCell) {
        if self.cells[index] == cell {
            return;
        }
        self.cells[index] = cell;
        self.mark_dirty(col, row, 1, 1);
    }

    /// Inspection helper: returns the packed cell at `(col, row)`.
    pub fn cell(&self, col: i32, row: i32) -> Option<&PackedCell> {
        self.index(col, row).and_then(|i| self.cells.get(i))
    }

    /// Resolve a `PackedText` handle back to its string.
    pub fn resolve_text(&self, text: PackedText) -> &str {
        self.interner.resolve(text)
    }

    // -- dirty tracking ---------------------------------------------------

    /// Mark the entire surface dirty so the next render does a full repaint.
    pub fn invalidate(&mut self) {
        self.dirty_all = true;
        self.dirty_region.clear();
    }

    /// Mark a rectangular region dirty.
    pub fn mark_dirty(&mut self, col: i32, row: i32, width: i32, height: i32) {
        if self.dirty_all {
            return;
        }
        self.dirty_region.add_rect(col, row, width, height);
    }

    /// Consume the dirty region and return either `None` (full repaint
    /// needed) or `Some(indices)` of dirty cells.
    pub fn take_dirty_indices(&mut self) -> Option<Vec<usize>> {
        if self.dirty_all {
            self.dirty_all = false;
            self.dirty_region.clear();
            return None; // None signals full repaint
        }
        if self.dirty_region.is_empty() {
            return Some(Vec::new());
        }
        let indices: Vec<usize> = self
            .dirty_region
            .iter_indices(self.cols, self.rows)
            .collect();
        self.dirty_region.clear();
        Some(indices)
    }

    // -- drawing primitives -----------------------------------------------

    pub fn fill_rect(&mut self, rect: CellRect, fg: Color, bg: Color) {
        let row0 = rect.row.max(0);
        let row1 = (rect.row + rect.rows).min(self.rows).max(row0);
        let col0 = rect.col.max(0);
        let col1 = (rect.col + rect.cols).min(self.cols).max(col0);
        let blank = blank_packed(fg, bg);
        for row in row0..row1 {
            for col in col0..col1 {
                let index = (row * self.cols + col) as usize;
                self.write_cell(index, col, row, blank);
            }
        }
    }

    pub fn set_cell(
        &mut self,
        col: i32,
        row: i32,
        fg: Color,
        bg: Color,
        text: impl AsRef<str>,
        underline: bool,
    ) {
        self.set_cell_styled(col, row, fg, bg, text, underline, false);
    }

    pub fn set_cell_styled(
        &mut self,
        col: i32,
        row: i32,
        fg: Color,
        bg: Color,
        text: impl AsRef<str>,
        underline: bool,
        reverse: bool,
    ) {
        if self.index(col, row).is_none() {
            return;
        }
        let text = text.as_ref();
        let packed_text = self
            .interner
            .get_or_intern(if text.is_empty() { " " } else { text });
        let index = self.index(col, row).unwrap();
        self.write_cell(
            index,
            col,
            row,
            PackedCell {
                style: PackedStyle::new(fg, bg, false, underline, reverse, false),
                text: packed_text,
            },
        );
    }

    pub fn put_cell_span(
        &mut self,
        col: i32,
        row: i32,
        span: i32,
        text: &str,
        fg: Color,
        bg: Color,
        underline: bool,
    ) {
        self.put_cell_span_styled(col, row, span, text, fg, bg, underline, false, false);
    }

    pub fn put_cell_span_styled(
        &mut self,
        col: i32,
        row: i32,
        span: i32,
        text: &str,
        fg: Color,
        bg: Color,
        underline: bool,
        reverse: bool,
        bold: bool,
    ) {
        self.put_span_packed(
            col,
            row,
            span,
            text,
            PackedStyle::new(fg, bg, bold, underline, reverse, false),
        );
    }

    /// Write `text` at `(col, row)` spanning `span` cells with a
    /// fully-specified style (the grid-blit path, which carries flags like
    /// italic that the chrome helpers don't). Continuation flags for the
    /// trailing cells of a wide glyph are managed here.
    pub fn put_span_packed(
        &mut self,
        col: i32,
        row: i32,
        span: i32,
        text: &str,
        style: PackedStyle,
    ) {
        let span = span.max(1);
        let main_text = self
            .interner
            .get_or_intern(if text.is_empty() { " " } else { text });
        let cont_text = self.interner.get_or_intern(" ");
        for offset in 0..span {
            let cell_col = col + offset;
            if let Some(index) = self.index(cell_col, row) {
                let mut style = style;
                if offset > 0 {
                    style.flags |= PackedStyle::CONTINUATION_MASK;
                }
                self.write_cell(
                    index,
                    cell_col,
                    row,
                    PackedCell {
                        style,
                        text: if offset == 0 { main_text } else { cont_text },
                    },
                );
            }
        }
    }

    pub fn put_text(
        &mut self,
        col: i32,
        row: i32,
        max_cols: i32,
        fg: Color,
        bg: Color,
        value: &str,
    ) {
        self.put_text_styled(col, row, max_cols, fg, bg, value, false, false);
    }

    pub fn put_text_bold(
        &mut self,
        col: i32,
        row: i32,
        max_cols: i32,
        fg: Color,
        bg: Color,
        value: &str,
    ) {
        self.put_text_styled(col, row, max_cols, fg, bg, value, false, true);
    }

    pub fn put_text_styled(
        &mut self,
        col: i32,
        row: i32,
        max_cols: i32,
        fg: Color,
        bg: Color,
        value: &str,
        reverse: bool,
        bold: bool,
    ) {
        if max_cols <= 0 {
            return;
        }
        let mut cursor = col;
        let end = col + max_cols;
        for ch in value.chars() {
            if ch == '\n' || cursor >= end {
                break;
            }
            let width = char_cell_width(ch);
            if cursor + width > end {
                break;
            }
            let mut buf = [0; 4];
            self.put_cell_span_styled(
                cursor,
                row,
                width,
                ch.encode_utf8(&mut buf),
                fg,
                bg,
                false,
                reverse,
                bold,
            );
            cursor += width;
        }
    }
}

fn blank_packed(fg: Color, bg: Color) -> PackedCell {
    // A blank cell is a single space with no style flags.
    PackedCell {
        style: PackedStyle::new(fg, bg, false, false, false, false),
        text: PackedText::ascii(b' '),
    }
}

pub fn draw_box(
    surface: &mut CellSurface,
    rect: CellRect,
    fill_fg: Color,
    bg: Color,
    border: Color,
) {
    surface.fill_rect(rect, fill_fg, bg);
    if rect.cols <= 0 || rect.rows <= 0 {
        return;
    }
    let left = rect.col;
    let right = rect.col + rect.cols - 1;
    let top = rect.row;
    let bottom = rect.row + rect.rows - 1;

    if rect.rows == 1 {
        for col in left..=right {
            surface.set_cell(col, top, border, bg, "\u{2500}", false);
        }
        return;
    }
    if rect.cols == 1 {
        for row in top..=bottom {
            surface.set_cell(left, row, border, bg, "\u{2502}", false);
        }
        return;
    }

    for col in (left + 1)..right {
        surface.set_cell(col, top, border, bg, "\u{2500}", false);
        surface.set_cell(col, bottom, border, bg, "\u{2500}", false);
    }
    for row in (top + 1)..bottom {
        surface.set_cell(left, row, border, bg, "\u{2502}", false);
        surface.set_cell(right, row, border, bg, "\u{2502}", false);
    }
    surface.set_cell(left, top, border, bg, "\u{250c}", false);
    surface.set_cell(right, top, border, bg, "\u{2510}", false);
    surface.set_cell(left, bottom, border, bg, "\u{2514}", false);
    surface.set_cell(right, bottom, border, bg, "\u{2518}", false);
}

pub fn render_cell_scrollbar(
    surface: &mut CellSurface,
    col: i32,
    row: i32,
    rows: i32,
    visible_items: usize,
    total_items: usize,
    scroll_from_top: usize,
    fg: Color,
    bg: Color,
    track_glyph: &str,
    thumb_fg: Color,
    thumb_glyph: &str,
) {
    if rows <= 0 || visible_items == 0 || total_items <= visible_items {
        return;
    }

    for offset in 0..rows {
        surface.set_cell(col, row + offset, fg, bg, track_glyph, false);
    }

    let thumb_rows =
        (((rows as i64 * visible_items as i64) / total_items as i64).max(1) as i32).min(rows);
    let max_scroll = total_items.saturating_sub(visible_items).max(1);
    let thumb_row = row
        + (((rows - thumb_rows).max(0) as i64 * scroll_from_top as i64) / max_scroll as i64) as i32;
    for offset in 0..thumb_rows {
        surface.set_cell(col, thumb_row + offset, thumb_fg, bg, thumb_glyph, false);
    }
}

pub fn char_cell_width(ch: char) -> i32 {
    unicode_width::UnicodeWidthChar::width(ch)
        .unwrap_or(1)
        .max(1) as i32
}

pub use ekko_tui::{display_cell_width, truncate_to_cells};

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `resize` is called at the top of every client frame. A
    /// same-size call must not touch the dirty state, or every frame becomes
    /// a full repaint and the diff renderer never runs.
    #[test]
    fn same_size_resize_is_a_noop() {
        let mut surface = CellSurface::new(8, 4, Color::default(), Color::default());
        surface.put_text(0, 0, 8, Color::default(), Color::default(), "hi");
        // Consume the initial dirty_all.
        assert!(surface.take_dirty_indices().is_none());
        surface.resize(8, 4, Color::default(), Color::default());
        assert!(!surface.dirty_all);
        assert_eq!(surface.take_dirty_indices(), Some(Vec::new()));
    }

    #[test]
    fn real_resize_still_invalidates() {
        let mut surface = CellSurface::new(8, 4, Color::default(), Color::default());
        assert!(surface.take_dirty_indices().is_none());
        surface.resize(10, 4, Color::default(), Color::default());
        assert!(surface.dirty_all);
    }
}
