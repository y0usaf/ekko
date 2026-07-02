//! Surface registration: extensions claim docked chrome regions (sidebar,
//! statusbar, ...) and draw them each frame. The layout resolver in
//! [`crate::layout`] turns the registered set into concrete rects; whatever
//! remains is the terminal pane.

use std::sync::Arc;

use ekko_event::UiAction;

use crate::draw::DrawContext;
use crate::snapshot::ClientSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockEdge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceSize {
    /// A fixed number of cells (rows for Top/Bottom, cols for Left/Right).
    Fixed(i32),
    /// Scales with the frame: `preferred` capped at the larger of `min` and
    /// `frame / max_fraction_denom`, and never shrinking the remaining
    /// space below `min_remaining`.
    Scaled {
        preferred: i32,
        min: i32,
        max_fraction_denom: i32,
        min_remaining: i32,
    },
}

/// Mouse input routed to the surface that owns the hit region. Coordinates
/// are local to the surface's resolved rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceMouseEvent {
    pub kind: MouseKind,
    pub col: i32,
    pub row: i32,
    pub region_cols: i32,
    pub region_rows: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    LeftPress,
    /// Motion with the left button held.
    LeftDrag,
    LeftRelease,
    WheelUp,
    WheelDown,
    Other,
}

pub type SurfaceDrawFn = Arc<dyn Fn(&mut dyn DrawContext, &ClientSnapshot) + Send + Sync>;
pub type SurfaceMouseFn =
    Arc<dyn Fn(&SurfaceMouseEvent, &ClientSnapshot) -> Vec<UiAction> + Send + Sync>;
pub type SurfaceTickFn = Arc<dyn Fn(&ClientSnapshot) -> bool + Send + Sync>;
pub type SurfaceVisibleFn = Arc<dyn Fn(&ClientSnapshot) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct SurfaceSpec {
    pub name: String,
    pub dock: DockEdge,
    /// Lower priority claims first along its edge.
    pub priority: i32,
    pub size: SurfaceSize,
    /// `(min_frame_cols, min_remaining_rows)`: skip this surface entirely
    /// when the frame is narrower than the first value or the rows left at
    /// claim time are fewer than the second.
    pub hide_below: Option<(i32, i32)>,
    /// Dynamic visibility: when present and returning `false`, the surface
    /// claims no region this cycle (its dock space returns to the terminal
    /// pane). Lets extensions toggle their chrome at runtime; `None` means
    /// always visible.
    pub visible: Option<SurfaceVisibleFn>,
    pub draw: SurfaceDrawFn,
    pub on_mouse: Option<SurfaceMouseFn>,
    /// When present and returning `true`, the host keeps an animation tick
    /// running so the surface is redrawn at the animation interval.
    pub wants_tick: Option<SurfaceTickFn>,
}
