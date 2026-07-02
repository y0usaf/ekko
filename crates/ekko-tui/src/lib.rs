//! Terminal primitives for ekko: raw-mode/alt-screen control, terminal color
//! probing and theme generation, cell-width text helpers, and spinner math.

pub mod cells;
pub mod color_cap;
pub mod palette;
pub mod spinner;
pub mod terminal;
pub mod theme;

pub use cells::{
    display_cell_width, truncate_line_in_place_to_width, truncate_line_preserving_suffix_to_width,
    truncate_line_to_width, truncate_line_with_ellipsis_to_width, truncate_to_cells,
};
pub use color_cap::{ColorCapability, color_capability, has_truecolor, rgb_to_xterm256};
pub use palette::{Rgb, TerminalColors, detect_terminal_colors};
pub use spinner::{SPINNER_FRAME_MS, SPINNER_FRAMES, spinner_frame_at, spinner_frame_index};
pub use terminal::{
    RawModeGuard, emergency_restore, escape_len, strip_ansi, terminal_size, visible_width,
};
pub use theme::{
    Mode, Theme, ansi_index_to_rgb, bg_escape, bold_fg_escape, brighten, default_terminal_colors,
    fade_toward, fg_escape, generate_theme, luminance_rgb, terminal_mode,
};
