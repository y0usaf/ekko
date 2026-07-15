//! Resolves wire-protocol `GridCell` colors against the active palette and
//! blits a stored grid into a `CellSurface` rect.

use ekko_ext::ThemePalette;
use ekko_grid::cell_surface::CellSurface;
use ekko_grid::color::Color;
use ekko_grid::layout::CellRect;
use ekko_grid::packed::PackedStyle;
use ekko_grid::selection::{SelectionRange, selection_span};
use ekko_grid::theme::{ansi_index_to_color, brighten, fade_toward};
use ekko_proto::{GridCell, GridRow, WireColor};

use crate::drawctx::gc;

fn resolve_wire_color(color: WireColor, default: Color, palette: &ThemePalette) -> Color {
    match color {
        WireColor::Default => default,
        WireColor::Indexed(idx) if idx < 16 => gc(palette.ansi[idx as usize]),
        WireColor::Indexed(idx) => ansi_index_to_color(idx),
        WireColor::Rgb(r, g, b) => Color::rgb(r, g, b),
    }
}

/// Resolve a wire cell's (fg, bg) into `Color`s, applying INVERSE/BOLD/DIM.
pub fn resolve_cell_colors(cell: &GridCell, palette: &ThemePalette) -> (Color, Color) {
    let mut fg = resolve_wire_color(cell.fg, gc(palette.term_fg), palette);
    let mut bg = resolve_wire_color(cell.bg, gc(palette.term_bg), palette);

    if cell.attrs & GridCell::INVERSE != 0 {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.attrs & GridCell::BOLD != 0 {
        fg = brighten(fg, 18);
    }
    if cell.attrs & GridCell::DIM != 0 {
        fg = fade_toward(fg, bg, 110);
    }

    (fg, bg)
}

/// Blit `rows` (the last known full grid, `grid_cols` x `grid_rows`) into
/// `rect` of `surface`. Any area of `rect` beyond the grid's own bounds is
/// filled with the terminal background. Cells covered by `selection` (in
/// grid-local coordinates) use the theme's dedicated selection fg/bg, matching
/// Zellij's explicit text-selection palette rather than inverting cell colors.
pub fn blit_grid(
    surface: &mut CellSurface,
    rect: CellRect,
    rows: &[GridRow],
    grid_cols: u16,
    grid_rows: u16,
    palette: &ThemePalette,
    selection: Option<SelectionRange>,
) {
    let term_fg = gc(palette.term_fg);
    let term_bg = gc(palette.term_bg);
    let mut text_buf = String::with_capacity(8);
    for r in 0..rect.rows {
        let row_y = rect.row + r;
        let grid_row = if (r as u16) < grid_rows {
            rows.get(r as usize)
        } else {
            None
        };
        let selected = selection_span(selection, r as u16, grid_cols);

        for c in 0..rect.cols {
            let col_x = rect.col + c;
            let cell = grid_row.and_then(|row| {
                if (c as u16) < grid_cols {
                    row.cells.get(c as usize)
                } else {
                    None
                }
            });
            let Some(cell) = cell else {
                surface.set_cell(col_x, row_y, term_fg, term_bg, " ", false);
                continue;
            };
            // Wide glyphs are written once as a two-cell span; their
            // continuation cells are covered by that span.
            if cell.attrs & GridCell::WIDE_CONT != 0 {
                continue;
            }
            let cell_colors = resolve_cell_colors(cell, palette);
            let (fg, bg) = if let Some((start, width)) = selected
                && (c as u16) >= start
                && (c as u16) < start + width
            {
                (gc(palette.selection_fg), gc(palette.selection_bg))
            } else {
                cell_colors
            };
            let span = if cell.attrs & GridCell::WIDE != 0 {
                2
            } else {
                1
            };
            let style = PackedStyle::new(
                fg,
                bg,
                cell.attrs & GridCell::BOLD != 0,
                cell.attrs & GridCell::UNDERLINE != 0,
                false,
                false,
            )
            .with_italic(cell.attrs & GridCell::ITALIC != 0);
            if cell.extra.is_empty() {
                let mut ch_buf = [0u8; 4];
                surface.put_span_packed(
                    col_x,
                    row_y,
                    span,
                    cell.ch.encode_utf8(&mut ch_buf),
                    style,
                );
            } else {
                text_buf.clear();
                text_buf.push(cell.ch);
                text_buf.extend(cell.extra.iter());
                surface.put_span_packed(col_x, row_y, span, &text_buf, style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_builtins::theme::terminal_palette;

    fn palette() -> ThemePalette {
        terminal_palette(&ekko_tui::default_terminal_colors())
    }

    fn cell(fg: WireColor, bg: WireColor, attrs: u8) -> GridCell {
        GridCell {
            ch: 'x',
            extra: Vec::new(),
            fg,
            bg,
            attrs,
        }
    }

    #[test]
    fn default_colors_use_theme_term_fg_bg() {
        let palette = palette();
        let (fg, bg) =
            resolve_cell_colors(&cell(WireColor::Default, WireColor::Default, 0), &palette);
        assert_eq!(fg, gc(palette.term_fg));
        assert_eq!(bg, gc(palette.term_bg));
    }

    #[test]
    fn indexed_low_uses_theme_ansi_palette() {
        let palette = palette();
        let (fg, _) = resolve_cell_colors(
            &cell(WireColor::Indexed(3), WireColor::Default, 0),
            &palette,
        );
        assert_eq!(fg, gc(palette.ansi[3]));
    }

    #[test]
    fn indexed_high_uses_256_color_cube() {
        let palette = palette();
        let (fg, _) = resolve_cell_colors(
            &cell(WireColor::Indexed(200), WireColor::Default, 0),
            &palette,
        );
        assert_eq!(fg, ansi_index_to_color(200));
    }

    #[test]
    fn rgb_passes_through() {
        let palette = palette();
        let (fg, _) = resolve_cell_colors(
            &cell(WireColor::Rgb(1, 2, 3), WireColor::Default, 0),
            &palette,
        );
        assert_eq!(fg, Color::rgb(1, 2, 3));
    }

    #[test]
    fn inverse_swaps_fg_and_bg() {
        let palette = palette();
        let (fg, bg) = resolve_cell_colors(
            &cell(
                WireColor::Rgb(1, 2, 3),
                WireColor::Rgb(4, 5, 6),
                GridCell::INVERSE,
            ),
            &palette,
        );
        assert_eq!(fg, Color::rgb(4, 5, 6));
        assert_eq!(bg, Color::rgb(1, 2, 3));
    }

    #[test]
    fn bold_brightens_fg() {
        let palette = palette();
        let (fg, _) = resolve_cell_colors(
            &cell(
                WireColor::Rgb(10, 10, 10),
                WireColor::Default,
                GridCell::BOLD,
            ),
            &palette,
        );
        assert_eq!(fg, brighten(Color::rgb(10, 10, 10), 18));
    }

    #[test]
    fn blit_preserves_bold_style() {
        let palette = palette();
        let mut surface = CellSurface::new(1, 1, gc(palette.term_fg), gc(palette.term_bg));
        let rows = vec![GridRow {
            cells: vec![cell(
                WireColor::Rgb(10, 10, 10),
                WireColor::Default,
                GridCell::BOLD,
            )],
        }];
        blit_grid(
            &mut surface,
            CellRect::new(0, 0, 1, 1),
            &rows,
            1,
            1,
            &palette,
            None,
        );
        assert!(surface.cell(0, 0).unwrap().style.bold());
    }

    #[test]
    fn dim_fades_fg_toward_bg() {
        let palette = palette();
        let (fg, bg) = resolve_cell_colors(
            &cell(
                WireColor::Rgb(200, 200, 200),
                WireColor::Rgb(0, 0, 0),
                GridCell::DIM,
            ),
            &palette,
        );
        assert_eq!(fg, fade_toward(Color::rgb(200, 200, 200), bg, 110));
    }

    #[test]
    fn blit_fills_beyond_grid_bounds_with_term_bg() {
        let palette = palette();
        let mut surface = CellSurface::new(10, 4, gc(palette.term_fg), gc(palette.term_bg));
        let rows = vec![GridRow {
            cells: vec![cell(WireColor::Rgb(9, 9, 9), WireColor::Default, 0)],
        }];
        blit_grid(
            &mut surface,
            CellRect::new(0, 0, 5, 2),
            &rows,
            1,
            1,
            &palette,
            None,
        );
        let first = surface.cell(0, 0).unwrap();
        assert_eq!(first.style.fg(), Color::rgb(9, 9, 9));
        let beyond = surface.cell(1, 0).unwrap();
        assert_eq!(beyond.style.bg(), gc(palette.term_bg));
        let below = surface.cell(0, 1).unwrap();
        assert_eq!(below.style.bg(), gc(palette.term_bg));
    }

    #[test]
    fn selection_uses_theme_colors_without_inverting_cell() {
        let mut palette = palette();
        palette.selection_fg = ekko_ext::Color::rgb(220, 221, 222);
        palette.selection_bg = ekko_ext::Color::rgb(60, 61, 62);
        let mut surface = CellSurface::new(4, 2, gc(palette.term_fg), gc(palette.term_bg));
        let rows = vec![GridRow {
            cells: vec![
                cell(WireColor::Rgb(1, 1, 1), WireColor::Rgb(9, 9, 9), 0),
                cell(WireColor::Rgb(1, 1, 1), WireColor::Rgb(9, 9, 9), 0),
            ],
        }];
        let range = ekko_grid::selection::SelectionRange {
            start: ekko_grid::selection::SelectionPoint { row: 0, col: 0 },
            end: ekko_grid::selection::SelectionPoint { row: 0, col: 1 },
        };
        blit_grid(
            &mut surface,
            CellRect::new(0, 0, 4, 2),
            &rows,
            2,
            1,
            &palette,
            Some(range),
        );
        let selected = surface.cell(0, 0).unwrap();
        assert_eq!(selected.style.fg(), gc(palette.selection_fg));
        assert_eq!(selected.style.bg(), gc(palette.selection_bg));
        let unselected = surface.cell(1, 0).unwrap();
        assert_eq!(unselected.style.fg(), Color::rgb(1, 1, 1));
        assert_eq!(unselected.style.bg(), Color::rgb(9, 9, 9));
    }

    #[test]
    fn wide_cells_blit_as_spans_with_continuation() {
        let palette = palette();
        let mut surface = CellSurface::new(4, 1, gc(palette.term_fg), gc(palette.term_bg));
        let rows = vec![GridRow {
            cells: vec![
                GridCell {
                    ch: '你',
                    extra: Vec::new(),
                    fg: WireColor::Default,
                    bg: WireColor::Default,
                    attrs: GridCell::WIDE,
                },
                GridCell {
                    ch: ' ',
                    extra: Vec::new(),
                    fg: WireColor::Default,
                    bg: WireColor::Default,
                    attrs: GridCell::WIDE_CONT,
                },
            ],
        }];
        blit_grid(
            &mut surface,
            CellRect::new(0, 0, 4, 1),
            &rows,
            2,
            1,
            &palette,
            None,
        );
        assert_eq!(surface.resolve_text(surface.cell(0, 0).unwrap().text), "你");
        assert!(surface.cell(1, 0).unwrap().style.continuation());
    }
}
