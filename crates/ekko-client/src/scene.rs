//! Frame composition, registry-driven: every chrome region is drawn by the
//! surface extension that claimed it; each terminal pane is blitted at its
//! server-provided rect inside what remains; pane separators fill the gap
//! cells the daemon reserved between/around panes; the active mode and
//! overlay draw their layers last. This module is pure mechanism — it
//! holds no opinion about which surfaces or pane layouts exist.

use ekko_ext::{AppRuntime, ClientSnapshot, Rect, ResolvedLayout};
use ekko_grid::cell_surface::CellSurface;
use ekko_grid::layout::CellRect;

use crate::drawctx::{RegionDrawContext, gc, to_cell_rect};
use crate::gridblit::blit_grid;
use crate::state::ClientState;

/// Draw one full frame into `surface`. Returns the hardware cursor position
/// (col, row), if any, that should be visible after this frame.
pub fn render_frame(
    surface: &mut CellSurface,
    layout: &ResolvedLayout,
    runtime: &AppRuntime,
    state: &mut ClientState,
    snapshot: &ClientSnapshot,
) -> Option<(i32, i32)> {
    for region in &layout.regions {
        let Some(spec) = runtime.surface(&region.name) else {
            continue;
        };
        let draw = spec.draw.clone();
        let mut ctx = RegionDrawContext::new(surface, region.rect);
        draw(&mut ctx, snapshot);
    }

    // Compose every pane at its server-provided rect inside the terminal
    // region. The region can exceed the session canvas (the daemon sizes
    // to the smallest attached client); the remainder shows the terminal
    // background rather than last frame's leftovers.
    let terminal = to_cell_rect(layout.terminal);
    surface.fill_rect(
        terminal,
        gc(snapshot.theme.term_fg),
        gc(snapshot.theme.term_bg),
    );
    for (&id, pane) in &state.workspace.panes {
        let rect = CellRect::new(
            terminal.col + i32::from(pane.rect.x),
            terminal.row + i32::from(pane.rect.y),
            i32::from(pane.rect.cols),
            i32::from(pane.rect.rows),
        );
        let selection = (state.selection_pane == Some(id))
            .then(|| state.selection.normalized())
            .flatten();
        blit_grid(
            surface,
            rect,
            &pane.grid.cells,
            pane.grid.cols,
            pane.grid.rows,
            &snapshot.theme,
            selection,
        );
    }

    // Search hits overlay the pane that owns the active search: matches
    // arrive in absolute rows (0 = oldest history line) and map into the
    // viewport as `row - (history - offset)`. Hits use the selection
    // colors, dimmed; the current hit uses them full-strength.
    if let Some(search) = &state.search
        && let Some(pane) = state.workspace.panes.get(&search.pane)
    {
        let first = i64::from(pane.grid.history) - i64::from(pane.grid.scrollback);
        let dim_bg = ekko_grid::theme::fade_toward(
            gc(snapshot.theme.selection_bg),
            gc(snapshot.theme.term_bg),
            150,
        );
        for (index, hit) in search.matches.iter().enumerate() {
            let view_row = i64::from(hit.row) - first;
            if view_row < 0 || view_row >= i64::from(pane.rect.rows) {
                continue;
            }
            let current = index == search.current;
            let bg = if current {
                gc(snapshot.theme.selection_bg)
            } else {
                dim_bg
            };
            let fg = if current {
                gc(snapshot.theme.selection_fg)
            } else {
                gc(snapshot.theme.term_fg)
            };
            let row_y = terminal.row + i32::from(pane.rect.y) + view_row as i32;
            let grid_row = pane.grid.cells.get(view_row as usize);
            // `col`/`len` are byte offsets in the plain row text; the
            // common case (ASCII) lines up with cells exactly, and wider
            // text just rounds to whole cells at the same start.
            for cell_index in hit.col..hit.col.saturating_add(hit.len) {
                let col_x = terminal.col + i32::from(pane.rect.x) + i32::from(cell_index);
                if cell_index >= pane.rect.cols {
                    break;
                }
                let ch = grid_row
                    .and_then(|row| row.cells.get(cell_index as usize))
                    .map(|cell| cell.ch)
                    .unwrap_or(' ');
                surface.set_cell(col_x, row_y, fg, bg, ch.to_string(), false);
            }
        }
    }

    // Pane separators ride the cells the daemon reserved in the layout:
    // shared boundary lines (compact) or per-pane box frames (frame),
    // tinted with the accent color where they touch the focused pane.
    if state.workspace.border_style != ekko_proto::PaneBorderStyle::None {
        let panes: Vec<(u64, ekko_proto::PaneRect)> = state
            .workspace
            .panes
            .iter()
            .map(|(&id, pane)| (id, pane.rect))
            .collect();
        let border_fg = gc(snapshot.theme.border);
        let focused_fg = gc(snapshot.theme.accent);
        let bg = gc(snapshot.theme.term_bg);
        for cell in crate::borders::border_cells(
            &panes,
            state.workspace.border_style,
            state.workspace.focused,
        ) {
            surface.set_cell(
                terminal.col + cell.col,
                terminal.row + cell.row,
                if cell.focused { focused_fg } else { border_fg },
                bg,
                cell.ch.to_string(),
                false,
            );
        }
    }

    // Scroll indicator: while a pane is scrolled into history, show
    // `offset/total` in its top-right corner (overlapping the frame's top
    // edge in frame style, the pane's first row otherwise).
    for pane in state.workspace.panes.values() {
        if pane.grid.scrollback == 0 {
            continue;
        }
        let label = format!(" {}/{} ", pane.grid.scrollback, pane.grid.history);
        let right = terminal.col + i32::from(pane.rect.x) + i32::from(pane.rect.cols);
        // In frame style the pane's top frame edge carries the indicator;
        // otherwise it overlaps the pane's first content row.
        let frame_offset =
            i32::from(state.workspace.border_style == ekko_proto::PaneBorderStyle::Frame);
        let top = terminal.row + i32::from(pane.rect.y) - frame_offset;
        let start = right - label.chars().count() as i32;
        for (i, ch) in label.chars().enumerate() {
            surface.set_cell(
                start + i as i32,
                top,
                gc(snapshot.theme.accent),
                gc(snapshot.theme.term_bg),
                ch.to_string(),
                false,
            );
        }
    }

    let mut cursor = terminal_cursor(state, layout.terminal);
    let frame = Rect::new(0, 0, surface.cols, surface.rows);

    if !state.in_normal_mode()
        && let Some(spec) = runtime.mode(&state.mode)
        && let Some(render) = spec.render.clone()
        && let Some(mode_state) = &state.mode_state
    {
        let mut ctx = RegionDrawContext::new(surface, frame);
        if let Some(position) = render(&mut ctx, mode_state, snapshot) {
            cursor = Some(position);
        }
    }

    if let Some(active) = &mut state.overlay
        && let Some(spec) = runtime.overlay(&active.name)
    {
        let render = spec.render.clone();
        let mut ctx = RegionDrawContext::new(surface, frame);
        render(&mut ctx, &mut active.state, snapshot);
        cursor = None;
    }

    cursor
}

/// The hardware cursor belongs to the focused pane only, offset by that
/// pane's rect inside the terminal region.
fn terminal_cursor(state: &ClientState, terminal: Rect) -> Option<(i32, i32)> {
    let pane = state.workspace.panes.get(&state.workspace.focused)?;
    let cursor = pane.grid.cursor.filter(|c| c.visible)?;
    Some((
        terminal.col + i32::from(pane.rect.x) + i32::from(cursor.col),
        terminal.row + i32::from(pane.rect.y) + i32::from(cursor.row),
    ))
}
