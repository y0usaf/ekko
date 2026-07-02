//! Unified glyph vocabulary — the single symbol register for all of phi.
//!
//! One coherent family (box-drawing, geometric shapes, and a small set of
//! punctuation marks), mirroring crush's consolidated icon constants. Every
//! indicator role has exactly one named glyph; consumers reference these
//! constants instead of inlining their own `›`/`·`/`✓` variants, so the
//! vocabulary can't drift between surfaces.
//!
//! Nerd Font glyphs are deliberately excluded: they live only in the
//! session chrome (which has an ASCII fallback path). Everything here is
//! plain Unicode with universal font coverage.

/// Focused / selected row marker: a thick left half-block, crush-style.
pub const FOCUS_MARKER: &str = "▌";
/// Left-gutter bar marking a message's voice (user turns).
pub const GUTTER_BAR: &str = "▌";
/// List bullet / primary-role marker.
pub const BULLET: &str = "•";
/// Secondary-role / inactive marker.
pub const DOT: &str = "·";
/// Success indicator.
pub const CHECK: &str = "✓";
/// Failure indicator.
pub const CROSS: &str = "✗";
/// Directional arrow (result flows, transitions).
pub const ARROW: &str = "→";
/// Reverse arrow (attribution: `model ← "prompt"`).
pub const ARROW_LEFT: &str = "←";
/// Horizontal rule fill.
pub const RULE: &str = "─";
/// Compact separator between an anchor and its detail.
pub const SLASH: &str = "╱";
/// Truncation mark.
pub const ELLIPSIS: &str = "…";
