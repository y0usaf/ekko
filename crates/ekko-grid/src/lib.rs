//! Cell-grid + ANSI rendering primitives for the mux compositor.
//!
//! Pure rendering layer: cell surfaces, packed cells, color management,
//! damage tracking, layout math, text selection, ANSI emission, and
//! terminal-cell color resolution. No dependency on mux app logic.

pub mod ansi;
pub mod cell_surface;
pub mod charmtone;
pub mod color;
pub mod damage;
pub mod layout;
pub mod packed;
pub mod selection;
pub mod theme;

pub use ekko_tui::{display_cell_width, truncate_to_cells};
