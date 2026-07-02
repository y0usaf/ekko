//! Terminal primitives for ekko: raw-mode/alt-screen control, terminal color
//! probing and theme generation, cell-width text helpers, spinner/shimmer
//! animation, and the Charmtone chrome palette. Ported from phi-tui and
//! pi-harness.

pub mod cells;
pub mod clock;
pub mod color_cap;
pub mod palette;
pub mod spinner;
pub mod style;
pub mod symbols;
pub mod terminal;
pub mod theme;

pub use cells::{
    display_cell_width, truncate_line_in_place_to_width, truncate_line_preserving_suffix_to_width,
    truncate_line_to_width, truncate_line_with_ellipsis_to_width, truncate_to_cells,
};
pub use clock::{ANIMATION_FRAME_MS, AnimationClock};
pub use color_cap::{ColorCapability, color_capability, has_truecolor, rgb_to_xterm256};
pub use palette::{Rgb, TerminalColors, detect_terminal_colors};
pub use spinner::{
    EQUALIZER_BAR_COUNT, EQUALIZER_BARS, SHIMMER_BLEND_PEAK, SHIMMER_CYCLE_MS, SHIMMER_FRAME_MS,
    SHIMMER_MIN_WINDOW, SHIMMER_TRIANGLE_BLEND, SHIMMER_TRIANGLE_STEPS, SHIMMER_WINDOW_FRAC,
    SPINNER_FRAME_MS, SPINNER_FRAMES, Spinner, equalizer_glyphs, gradient_text,
    gradient_text_animated, shimmer_ansi, shimmer_triangle_blend, shimmer_triangle_intensity,
    spinner_frame_at, spinner_frame_index,
};
pub use symbols::{
    ARROW, ARROW_LEFT, BULLET, CHECK, CROSS, DOT, ELLIPSIS, FOCUS_MARKER, GUTTER_BAR, RULE, SLASH,
};
pub use terminal::{
    RawModeGuard, emergency_restore, escape_len, strip_ansi, terminal_size, visible_width,
};
pub use theme::{
    Mode, Theme, ansi_index_to_rgb, bg_escape, bold_fg_escape, brighten, default_terminal_colors,
    fade_toward, fg_escape, generate_theme, luminance_rgb, terminal_mode,
};
