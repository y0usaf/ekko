//! Drawing surface trait for extension-provided UI chrome.
//!
//! `DrawContext` is a scoped view over the terminal cell grid. The host owns
//! the underlying surface; extensions receive a `&mut dyn DrawContext`
//! bounded (and coordinate-translated) to the region they claimed. All
//! methods are data-only commands, so a scripted bridge can buffer them as
//! ops and drain after the callback returns without changing this trait.
//!
//! This trait lives in `ekko-ext` (not `ekko-grid`) so the extension system has
//! no dependency on the renderer crate; the client implements it as a thin
//! clipping adapter over its `CellSurface`.

use crate::visual::Color;

/// A rectangular cell region. Mirrors the renderer's rect type so the host
/// conversion is field-for-field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub col: i32,
    pub row: i32,
    pub cols: i32,
    pub rows: i32,
}

impl Rect {
    pub fn new(col: i32, row: i32, cols: i32, rows: i32) -> Self {
        Self {
            col,
            row,
            cols,
            rows,
        }
    }

    pub fn contains_cell(self, col: i32, row: i32) -> bool {
        col >= self.col
            && row >= self.row
            && col < self.col + self.cols
            && row < self.row + self.rows
    }
}

/// Colors and style flags for [`DrawContext::put_text_styled`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextStyle {
    pub fg: Color,
    pub bg: Color,
    pub reverse: bool,
    pub bold: bool,
}

impl TextStyle {
    /// Plain text in the given colors (no reverse, no bold).
    pub fn plain(fg: Color, bg: Color) -> Self {
        Self {
            fg,
            bg,
            reverse: false,
            bold: false,
        }
    }
}

/// Scroll geometry for [`DrawContext::render_scrollbar`]: what the bar
/// represents, independent of how it is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollbarModel {
    pub visible_items: usize,
    pub total_items: usize,
    pub scroll_from_top: usize,
}

/// Visual parameters for [`DrawContext::render_scrollbar`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollbarStyle<'a> {
    pub fg: Color,
    pub bg: Color,
    pub track_glyph: &'a str,
    pub thumb_fg: Color,
    pub thumb_glyph: &'a str,
}

/// The drawing surface exposed to extensions. Coordinates are 0-based and
/// local to the context's region; the host clips out-of-bounds writes.
pub trait DrawContext {
    /// Region dimensions as `(cols, rows)`.
    fn size(&self) -> (i32, i32);

    /// Fill a region with blank cells in the given colors.
    fn fill_rect(&mut self, rect: Rect, fg: Color, bg: Color);

    /// Set a single cell's text and colors.
    fn set_cell(&mut self, col: i32, row: i32, fg: Color, bg: Color, text: &str, underline: bool);

    /// Write text left-aligned at `(col, row)`, clipped to `max_cols` cells.
    fn put_text(&mut self, col: i32, row: i32, max_cols: i32, fg: Color, bg: Color, value: &str);

    /// Write bold text.
    fn put_text_bold(
        &mut self,
        col: i32,
        row: i32,
        max_cols: i32,
        fg: Color,
        bg: Color,
        value: &str,
    );

    /// Write text with explicit colors and reverse/bold style flags.
    fn put_text_styled(&mut self, col: i32, row: i32, max_cols: i32, value: &str, style: TextStyle);

    /// Draw a box border around `rect`, filling the interior.
    fn draw_box(&mut self, rect: Rect, fill_fg: Color, bg: Color, border: Color);

    /// Render a vertical scrollbar at `col` spanning `rows`.
    fn render_scrollbar(
        &mut self,
        col: i32,
        row: i32,
        rows: i32,
        model: ScrollbarModel,
        style: ScrollbarStyle<'_>,
    );
}
