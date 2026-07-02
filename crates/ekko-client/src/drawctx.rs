//! The client's `DrawContext` implementation: a clipping, coordinate-
//! translating adapter from a claimed region onto the real `CellSurface`.
//! Extension colors share the renderer's packed representation, so
//! conversion is a bit copy.

use ekko_ext::{Color as ExtColor, DrawContext, Rect};
use ekko_grid::cell_surface::{CellSurface, draw_box, render_cell_scrollbar};
use ekko_grid::layout::CellRect;

/// Extension color -> renderer color (same packed ARGB representation).
pub fn gc(color: ExtColor) -> ekko_grid::color::Color {
    ekko_grid::color::Color(color.0)
}

pub fn to_cell_rect(rect: Rect) -> CellRect {
    CellRect::new(rect.col, rect.row, rect.cols, rect.rows)
}

pub struct RegionDrawContext<'a> {
    surface: &'a mut CellSurface,
    region: Rect,
}

impl<'a> RegionDrawContext<'a> {
    pub fn new(surface: &'a mut CellSurface, region: Rect) -> Self {
        Self { surface, region }
    }

    /// Clip a local rect to the region and translate it to absolute cells.
    fn clip(&self, rect: Rect) -> Option<CellRect> {
        let col0 = rect.col.max(0);
        let row0 = rect.row.max(0);
        let col1 = (rect.col + rect.cols).min(self.region.cols);
        let row1 = (rect.row + rect.rows).min(self.region.rows);
        if col1 <= col0 || row1 <= row0 {
            return None;
        }
        Some(CellRect::new(
            self.region.col + col0,
            self.region.row + row0,
            col1 - col0,
            row1 - row0,
        ))
    }

    fn in_bounds(&self, col: i32, row: i32) -> bool {
        col >= 0 && row >= 0 && col < self.region.cols && row < self.region.rows
    }

    /// Cells available from `col` to the region's right edge, capped at
    /// `max_cols`.
    fn clamp_cols(&self, col: i32, max_cols: i32) -> i32 {
        max_cols.min((self.region.cols - col).max(0))
    }
}

impl DrawContext for RegionDrawContext<'_> {
    fn size(&self) -> (i32, i32) {
        (self.region.cols, self.region.rows)
    }

    fn fill_rect(&mut self, rect: Rect, fg: ExtColor, bg: ExtColor) {
        if let Some(rect) = self.clip(rect) {
            self.surface.fill_rect(rect, gc(fg), gc(bg));
        }
    }

    fn set_cell(
        &mut self,
        col: i32,
        row: i32,
        fg: ExtColor,
        bg: ExtColor,
        text: &str,
        underline: bool,
    ) {
        if self.in_bounds(col, row) {
            self.surface.set_cell(
                self.region.col + col,
                self.region.row + row,
                gc(fg),
                gc(bg),
                text,
                underline,
            );
        }
    }

    fn put_text(
        &mut self,
        col: i32,
        row: i32,
        max_cols: i32,
        fg: ExtColor,
        bg: ExtColor,
        value: &str,
    ) {
        self.put_text_styled(col, row, max_cols, fg, bg, value, false, false);
    }

    fn put_text_bold(
        &mut self,
        col: i32,
        row: i32,
        max_cols: i32,
        fg: ExtColor,
        bg: ExtColor,
        value: &str,
    ) {
        self.put_text_styled(col, row, max_cols, fg, bg, value, false, true);
    }

    fn put_text_styled(
        &mut self,
        col: i32,
        row: i32,
        max_cols: i32,
        fg: ExtColor,
        bg: ExtColor,
        value: &str,
        reverse: bool,
        bold: bool,
    ) {
        if !self.in_bounds(col.max(0), row) {
            return;
        }
        let max_cols = self.clamp_cols(col, max_cols);
        self.surface.put_text_styled(
            self.region.col + col,
            self.region.row + row,
            max_cols,
            gc(fg),
            gc(bg),
            value,
            reverse,
            bold,
        );
    }

    fn draw_box(&mut self, rect: Rect, fill_fg: ExtColor, bg: ExtColor, border: ExtColor) {
        if let Some(rect) = self.clip(rect) {
            draw_box(self.surface, rect, gc(fill_fg), gc(bg), gc(border));
        }
    }

    fn render_scrollbar(
        &mut self,
        col: i32,
        row: i32,
        rows: i32,
        visible_items: usize,
        total_items: usize,
        scroll_from_top: usize,
        fg: ExtColor,
        bg: ExtColor,
        track_glyph: &str,
        thumb_fg: ExtColor,
        thumb_glyph: &str,
    ) {
        if !self.in_bounds(col, row.max(0)) {
            return;
        }
        let rows = rows.min((self.region.rows - row).max(0));
        render_cell_scrollbar(
            self.surface,
            self.region.col + col,
            self.region.row + row,
            rows,
            visible_items,
            total_items,
            scroll_from_top,
            gc(fg),
            gc(bg),
            track_glyph,
            gc(thumb_fg),
            thumb_glyph,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_grid::color::Color;

    fn surface() -> CellSurface {
        CellSurface::new(10, 6, Color::rgb(1, 1, 1), Color::rgb(2, 2, 2))
    }

    #[test]
    fn writes_are_translated_to_the_region_origin() {
        let mut s = surface();
        let mut ctx = RegionDrawContext::new(&mut s, Rect::new(3, 2, 5, 3));
        ctx.set_cell(
            0,
            0,
            ExtColor::rgb(9, 9, 9),
            ExtColor::rgb(0, 0, 0),
            "x",
            false,
        );
        let cell = s.cell(3, 2).unwrap();
        assert_eq!(cell.style.fg(), Color::rgb(9, 9, 9));
    }

    #[test]
    fn writes_outside_the_region_are_clipped() {
        let mut s = surface();
        let mut ctx = RegionDrawContext::new(&mut s, Rect::new(3, 2, 5, 3));
        ctx.set_cell(
            5,
            0,
            ExtColor::rgb(9, 9, 9),
            ExtColor::rgb(0, 0, 0),
            "x",
            false,
        );
        ctx.set_cell(
            -1,
            0,
            ExtColor::rgb(9, 9, 9),
            ExtColor::rgb(0, 0, 0),
            "x",
            false,
        );
        ctx.fill_rect(
            Rect::new(0, 0, 100, 100),
            ExtColor::rgb(7, 7, 7),
            ExtColor::rgb(8, 8, 8),
        );
        // Fill stayed inside the region.
        assert_eq!(s.cell(2, 2).unwrap().style.bg(), Color::rgb(2, 2, 2));
        assert_eq!(s.cell(3, 2).unwrap().style.bg(), Color::rgb(8, 8, 8));
        assert_eq!(s.cell(7, 4).unwrap().style.bg(), Color::rgb(8, 8, 8));
        assert_eq!(s.cell(8, 2).unwrap().style.bg(), Color::rgb(2, 2, 2));
        assert_eq!(s.cell(3, 5).unwrap().style.bg(), Color::rgb(2, 2, 2));
    }

    #[test]
    fn put_text_clamps_to_region_width() {
        let mut s = surface();
        let mut ctx = RegionDrawContext::new(&mut s, Rect::new(0, 0, 4, 1));
        ctx.put_text(
            2,
            0,
            100,
            ExtColor::rgb(9, 9, 9),
            ExtColor::rgb(0, 0, 0),
            "abcdef",
        );
        assert_eq!(s.resolve_text(s.cell(2, 0).unwrap().text), "a");
        assert_eq!(s.resolve_text(s.cell(3, 0).unwrap().text), "b");
        // Beyond the region: untouched blank.
        assert_eq!(s.resolve_text(s.cell(4, 0).unwrap().text), " ");
    }
}
