//! Dock layout resolver: turns the registered surface set into concrete
//! rects. Pure math, generalized from the client's original fixed
//! sidebar/terminal/statusbar layout — the algorithm is core mechanism;
//! *which surfaces exist* is extension policy.

use crate::draw::Rect;
use crate::surface::{DockEdge, SurfaceSize, SurfaceSpec};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedRegion {
    pub name: String,
    pub rect: Rect,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedLayout {
    /// Claimed chrome regions, in claim order.
    pub regions: Vec<ResolvedRegion>,
    /// Whatever the surfaces left over: the terminal pane.
    pub terminal: Rect,
}

impl ResolvedLayout {
    pub fn region(&self, name: &str) -> Option<Rect> {
        self.regions.iter().find(|r| r.name == name).map(|r| r.rect)
    }
}

/// Resolve the frame layout. Edges are processed Top, Bottom, Left, Right;
/// surfaces on the same edge stack by ascending `priority`. Each claim
/// shrinks the remaining rect; the remainder is the terminal pane.
pub fn resolve_layout(cols: u16, rows: u16, surfaces: &[&SurfaceSpec]) -> ResolvedLayout {
    let frame_cols = i32::from(cols).max(1);
    let frame_rows = i32::from(rows).max(1);
    let mut remaining = Rect::new(0, 0, frame_cols, frame_rows);
    let mut regions = Vec::new();

    for edge in [
        DockEdge::Top,
        DockEdge::Bottom,
        DockEdge::Left,
        DockEdge::Right,
    ] {
        let mut docked: Vec<&&SurfaceSpec> = surfaces.iter().filter(|s| s.dock == edge).collect();
        docked.sort_by_key(|s| s.priority);

        for spec in docked {
            if let Some((min_frame_cols, min_remaining_rows)) = spec.hide_below
                && (frame_cols < min_frame_cols || remaining.rows < min_remaining_rows)
            {
                continue;
            }

            let along = match edge {
                DockEdge::Top | DockEdge::Bottom => remaining.rows,
                DockEdge::Left | DockEdge::Right => remaining.cols,
            };
            let frame_along = match edge {
                DockEdge::Top | DockEdge::Bottom => frame_rows,
                DockEdge::Left | DockEdge::Right => frame_cols,
            };
            let size = match spec.size {
                SurfaceSize::Fixed(n) => n.min(along).max(0),
                SurfaceSize::Scaled {
                    preferred,
                    min,
                    max_fraction_denom,
                    min_remaining,
                } => {
                    let cap = (frame_along / max_fraction_denom.max(1)).max(min);
                    preferred
                        .min(cap)
                        .min((along - min_remaining).max(0))
                        .max(0)
                }
            };
            if size <= 0 {
                continue;
            }

            let rect = match edge {
                DockEdge::Top => {
                    let r = Rect::new(remaining.col, remaining.row, remaining.cols, size);
                    remaining.row += size;
                    remaining.rows -= size;
                    r
                }
                DockEdge::Bottom => {
                    let r = Rect::new(
                        remaining.col,
                        remaining.row + remaining.rows - size,
                        remaining.cols,
                        size,
                    );
                    remaining.rows -= size;
                    r
                }
                DockEdge::Left => {
                    let r = Rect::new(remaining.col, remaining.row, size, remaining.rows);
                    remaining.col += size;
                    remaining.cols -= size;
                    r
                }
                DockEdge::Right => {
                    let r = Rect::new(
                        remaining.col + remaining.cols - size,
                        remaining.row,
                        size,
                        remaining.rows,
                    );
                    remaining.cols -= size;
                    r
                }
            };
            regions.push(ResolvedRegion {
                name: spec.name.clone(),
                rect,
            });
        }
    }

    ResolvedLayout {
        regions,
        terminal: remaining,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// The original sidebar/statusbar layout expressed as surface specs;
    /// the assertions below are the original `compute_cell_layout` tests'
    /// exact expected values.
    fn statusbar() -> SurfaceSpec {
        SurfaceSpec {
            name: "statusbar".into(),
            dock: DockEdge::Bottom,
            priority: 0,
            size: SurfaceSize::Fixed(1),
            hide_below: None,
            visible: None,
            draw: Arc::new(|_, _| {}),
            on_mouse: None,
            wants_tick: None,
        }
    }

    fn sidebar(width: u16) -> SurfaceSpec {
        SurfaceSpec {
            name: "sidebar".into(),
            dock: DockEdge::Left,
            priority: 0,
            size: SurfaceSize::Scaled {
                preferred: i32::from(width),
                min: 24,
                max_fraction_denom: 4,
                min_remaining: 32,
            },
            hide_below: Some((64, 4)),
            visible: None,
            draw: Arc::new(|_, _| {}),
            on_mouse: None,
            wants_tick: None,
        }
    }

    fn layout(cols: u16, rows: u16) -> ResolvedLayout {
        let statusbar = statusbar();
        let sidebar = sidebar(36);
        resolve_layout(cols, rows, &[&statusbar, &sidebar])
    }

    #[test]
    fn hides_sidebar_when_compact() {
        let l = layout(60, 20);
        assert_eq!(l.region("sidebar"), None);
        assert_eq!(l.region("statusbar"), Some(Rect::new(0, 19, 60, 1)));
        assert_eq!(l.terminal, Rect::new(0, 0, 60, 19));
    }

    #[test]
    fn shows_sidebar_on_wide_grid() {
        // Sidebar scales with the frame: a quarter of 120 cols.
        let l = layout(120, 40);
        assert_eq!(l.region("sidebar"), Some(Rect::new(0, 0, 30, 39)));
        assert_eq!(l.terminal, Rect::new(30, 0, 90, 39));
        assert_eq!(l.region("statusbar"), Some(Rect::new(0, 39, 120, 1)));
    }

    #[test]
    fn sidebar_caps_at_configured_width_on_very_wide_grid() {
        let l = layout(200, 40);
        assert_eq!(l.region("sidebar"), Some(Rect::new(0, 0, 36, 39)));
    }

    #[test]
    fn sidebar_slims_down_instead_of_vanishing_on_medium_grid() {
        let l = layout(70, 30);
        assert_eq!(l.region("sidebar").unwrap().cols, 24);
        assert!(l.terminal.cols >= 32);
    }

    #[test]
    fn empty_surface_set_gives_fullscreen_terminal() {
        let l = resolve_layout(80, 24, &[]);
        assert!(l.regions.is_empty());
        assert_eq!(l.terminal, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn same_edge_surfaces_stack_by_priority() {
        let statusbar = statusbar();
        let mut second = statusbar.clone();
        second.name = "second".into();
        second.priority = 1;
        let l = resolve_layout(80, 24, &[&second, &statusbar]);
        // priority 0 claims the bottom row first; priority 1 stacks above it.
        assert_eq!(l.region("statusbar"), Some(Rect::new(0, 23, 80, 1)));
        assert_eq!(l.region("second"), Some(Rect::new(0, 22, 80, 1)));
        assert_eq!(l.terminal, Rect::new(0, 0, 80, 22));
    }
}
