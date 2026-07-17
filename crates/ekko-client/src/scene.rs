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
