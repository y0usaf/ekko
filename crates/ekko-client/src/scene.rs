//! Frame composition, registry-driven: every chrome region is drawn by the
//! surface extension that claimed it; the terminal grid is blitted into
//! whatever remains; the active mode and overlay draw their layers last.
//! This module is pure mechanism — it holds no opinion about which surfaces
//! exist or what they look like.

use ekko_ext::{AppRuntime, ClientSnapshot, Rect, ResolvedLayout};
use ekko_grid::cell_surface::CellSurface;

use crate::drawctx::{RegionDrawContext, to_cell_rect};
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

    blit_grid(
        surface,
        to_cell_rect(layout.terminal),
        &state.grid.cells,
        state.grid.cols,
        state.grid.rows,
        &snapshot.theme,
        state.selection.normalized(),
    );

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

fn terminal_cursor(state: &ClientState, terminal: Rect) -> Option<(i32, i32)> {
    let cursor = state.grid.cursor.filter(|c| c.visible)?;
    Some((
        terminal.col + i32::from(cursor.col),
        terminal.row + i32::from(cursor.row),
    ))
}
