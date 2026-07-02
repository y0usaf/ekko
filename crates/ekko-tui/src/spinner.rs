//! Simple braille-dot spinner for the live status line.
//!
//! A lightweight 10-frame braille spinner with no color blending, no trail
//! effects, and no pre-computed frames — just a static array of glyphs.
//!
//! This is the **single source of truth** for spinner frames and timing
//! across all of phi (session-tui, mux sidebar, grid theme). Every consumer
//! should use [`SPINNER_FRAMES`], [`SPINNER_FRAME_MS`], and
//! [`spinner_frame_at`] rather than defining its own copy.

use std::time::Instant;

/// Braille dot spinner frames.
pub const SPINNER_FRAMES: [&str; 10] = [
    "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280F}",
];

/// Duration each spinner frame is displayed, in milliseconds.
///
/// At 80 ms/frame the 10-frame sequence completes one full cycle in 800 ms
/// (12.5 FPS), which reads as a smooth, unhurried spin without flickering.
pub const SPINNER_FRAME_MS: u64 = 80;

/// Compute the spinner frame index for a given millisecond timestamp or
/// elapsed duration. Any monotonically increasing millisecond value works
/// (wall-clock `now_ms`, monotonic elapsed, etc.) — only the value modulo
/// `SPINNER_FRAME_MS * SPINNER_FRAMES.len()` determines the phase.
pub fn spinner_frame_index(ms: u64) -> usize {
    ((ms / SPINNER_FRAME_MS) as usize) % SPINNER_FRAMES.len()
}

/// Return the spinner glyph for a given millisecond timestamp or elapsed
/// duration.
pub fn spinner_frame_at(ms: u64) -> &'static str {
    SPINNER_FRAMES[spinner_frame_index(ms)]
}

// ── "equalizer" animation (VU-meter-style bars ▁▂▃…█) ───────────
//
// A 5-bar VU-meter animation where each bar rises and falls independently
// via a phase-shifted sine wave, mimicking the @nerisma/pi-input-revamp
// equalizer. Accompanied by cycling "thinking" phrases.

/// Block characters used for the equalizer bars, from smallest to tallest.
pub const EQUALIZER_BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Number of bars in the equalizer cluster.
pub const EQUALIZER_BAR_COUNT: usize = 5;

/// Sine-wave divisor controlling the animation speed (ms). Lower = faster.
/// Matches the pi-input-revamp rhythm (≈6.7 Hz oscillation per bar).
const EQUALIZER_SINE_DIV: f64 = 150.0;

/// Phase offset between adjacent bars (radians). Creates the wave-like
/// cascade across the cluster.
const EQUALIZER_PHASE_SHIFT: f64 = 0.9;

/// Render the equalizer glyph cluster as a plain string (no colors).
///
/// Returns 5 block characters whose heights follow phase-shifted sine waves,
/// producing a "dancing bars" VU-meter effect. `elapsed_ms` is the wall-clock
/// elapsed time since the animation started.
pub fn equalizer_glyphs(elapsed_ms: u64) -> String {
    let t = elapsed_ms as f64;
    let mut out = String::with_capacity(EQUALIZER_BAR_COUNT * 4);
    for i in 0..EQUALIZER_BAR_COUNT {
        let val = ((t / EQUALIZER_SINE_DIV + i as f64 * EQUALIZER_PHASE_SHIFT).sin() + 1.0) / 2.0; // 0..1
        let lvl = (val * (EQUALIZER_BARS.len() - 1) as f64).round() as usize;
        out.push_str(EQUALIZER_BARS[lvl.min(EQUALIZER_BARS.len() - 1)]);
    }
    out
}

// Color blending is now unified through `crate::fade_toward` (from
// `theme.rs`), which is the single source of truth for RGB interpolation
// across all of phi.  Previously this module had a private `fade_rgb` that
// was mathematically identical; it has been removed to avoid duplication.

/// Apply a per-character left-to-right RGB gradient to `text`.
///
/// Mirrors phi-mux's `render_sidebar_gradient_text`: each character's
/// foreground is a linear blend from `from` (left) to `to` (right). Emits a
/// raw `[38;2;r;g;bm` escape before each char and resets at the end.
pub fn gradient_text(text: &str, from: crate::palette::Rgb, to: crate::palette::Rgb) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let denom = (chars.len() - 1).max(1) as u16;
    let mut out = String::with_capacity(chars.len() * 16);
    for (i, ch) in chars.iter().enumerate() {
        let mix = ((i as u16 * 255) / denom).min(255) as u8;
        let rgb = crate::fade_toward(from, to, mix);
        out.push_str(&crate::color_cap::fg_sequence(rgb));
        out.push(*ch);
    }
    out.push_str("\x1b[0m");
    out
}

// ── Shared shimmer constants ─────────────────────────────────────────
//
// These are the **single source of truth** for all shimmer/gradient
// animation parameters across phi.  Every consumer — the mux compositor
// (sidebar gradient, statusbar shimmer), the session-TUI (chrome bar,
// tool status shimmer, thinking-equalizer gradient), and the
// extension-registered text effects — must reference these constants
// rather than hardcoding its own copy.

/// Animation frame duration (ms) for all triangle-wave shimmer effects.
///
/// Each step of the triangle wave advances every 40 ms.  Combined with
/// [`SHIMMER_TRIANGLE_STEPS`] (24 steps), a full sweep cycle takes
/// 24 × 40 = 960 ms.
///
/// This replaces the former `SIDEBAR_ANIMATION_FRAME_MS` in phi-mux and
/// the separate `THINKING_ANIMATION_FRAME_MS` that existed here — they
/// were the same value (40 ms) defined in two places.
pub const SHIMMER_FRAME_MS: u64 = 40;

/// Number of steps in the triangle wave.  The wave rises for `STEPS / 2`
/// steps (0 → 12) then falls for `STEPS / 2` steps (12 → 0), producing
/// a symmetric sweep.  A full cycle is `SHIMMER_FRAME_MS × STEPS` = 960 ms.
pub const SHIMMER_TRIANGLE_STEPS: usize = 24;

/// Multiplier applied to the triangle-wave intensity (0..12) to produce
/// the blend amount (0..252).  At the peak (intensity = 12), the blend is
/// 12 × 21 = 252/255 ≈ 99 % — the character's colour is almost fully
/// replaced by the shimmer accent.  This value is shared by the mux
/// sidebar gradient, the mux statusbar shimmer, and the thinking-equalizer
/// gradient so all three peak at the same intensity.
pub const SHIMMER_TRIANGLE_BLEND: u8 = 21;

/// Compute the triangle-wave shimmer intensity for a given frame + slot.
///
/// Returns a value in `0..(STEPS/2)` (i.e. 0..12).  At `phase = 0` the
/// intensity is 0 (no shimmer); at `phase = STEPS/2` it peaks at 12.
/// This is the single function every triangle-wave consumer should call
/// instead of inlining the `if phase < 12 { phase } else { 24 - phase }`
/// formula.
#[inline]
pub fn shimmer_triangle_intensity(elapsed_ms: u64, slot: usize) -> u8 {
    let frame = (elapsed_ms / SHIMMER_FRAME_MS) as usize;
    let phase = (frame + slot) % SHIMMER_TRIANGLE_STEPS;
    let half = SHIMMER_TRIANGLE_STEPS / 2;
    if phase < half {
        phase as u8
    } else {
        (SHIMMER_TRIANGLE_STEPS - phase) as u8
    }
}

/// Compute the blend amount (0..252) from the triangle-wave intensity.
///
/// Convenience: `intensity * SHIMMER_TRIANGLE_BLEND`, clamped to 255.
#[inline]
pub fn shimmer_triangle_blend(intensity: u8) -> u8 {
    intensity.saturating_mul(SHIMMER_TRIANGLE_BLEND)
}

/// Backward-compat alias for `SHIMMER_FRAME_MS`.
pub const THINKING_ANIMATION_FRAME_MS: u64 = SHIMMER_FRAME_MS;

/// Apply a per-character left-to-right RGB gradient with a time-driven,
/// position-shifted shimmer overlay.
///
/// This and phi-mux's `render_sidebar_gradient_text` both use the shared
/// [`shimmer_triangle_intensity`] / [`shimmer_triangle_blend`] functions,
/// so they produce the same shimmer strength.
///
/// For each character:
/// 1. Compute the base left->right blend (`from` -> `to`) by character index.
/// 2. Compute the triangle-wave intensity via
///    [`shimmer_triangle_intensity`] (shared with the mux sidebar/statusbar).
/// 3. Blend the base color toward `shimmer` by
///    [`shimmer_triangle_blend`] (shared).
///
/// Because the phase incorporates both `elapsed_ms` (time) and `i`
/// (position), the wave appears to sweep left-to-right across the text and
/// cycle repeatedly -- exactly the "sliding/crawling shimmer" phi-mux shows
/// on the active sidebar row.
pub fn gradient_text_animated(
    text: &str,
    from: crate::palette::Rgb,
    to: crate::palette::Rgb,
    shimmer: crate::palette::Rgb,
    elapsed_ms: u64,
) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let denom = (chars.len() - 1).max(1) as u16;
    let mut out = String::with_capacity(chars.len() * 16);
    for (i, ch) in chars.iter().enumerate() {
        let mix = ((i as u16 * 255) / denom).min(255) as u8;
        let base = crate::fade_toward(from, to, mix);
        let intensity = shimmer_triangle_intensity(elapsed_ms, i);
        let blend = shimmer_triangle_blend(intensity);
        let rgb = crate::fade_toward(base, shimmer, blend);
        out.push_str(&crate::color_cap::fg_sequence(rgb));
        out.push(*ch);
    }
    out.push_str("\x1b[0m");
    out
}

/// Full sweep cycle duration (ms). The shimmer highlight sweeps across
/// the text and back in this time, regardless of text width. This is the
/// key to **percentage-based sync**: because the position is a fraction of
/// `messageWidth` that cycles over a fixed wall-clock period, every section
/// (statusbar segments, tool headers) sweeps at the same rate and the
/// highlight sits at the same relative position across all sections
/// simultaneously.
pub const SHIMMER_CYCLE_MS: u64 = 2000;

/// Fraction of the text width occupied by the shimmer highlight window.
/// A wider window (e.g. 0.25 = 25%) keeps the shimmer visible on short
/// tool names like "read" (4 chars), while on longer text the window scales
/// up proportionally.
pub const SHIMMER_WINDOW_FRAC: f64 = 0.25;

/// Minimum shimmer window width in columns, so very short text still shows
/// a visible highlight.
pub const SHIMMER_MIN_WINDOW: usize = 3;

/// Blend strength toward the shimmer color at the glimmer peak (0..255).
/// A high value (200) makes the shimmer clearly visible even on light/white
/// text — instead of a hard color replacement (which flashes jarringly), we
/// interpolate the base fg toward the shimmer color by this amount, matching
/// claude-code's `interpolateColor` approach in its "tool-use" shimmer mode.
pub const SHIMMER_BLEND_PEAK: u8 = 200;

/// Check if an SGR escape sequence is a text attribute (bold, italic,
/// underline, etc.) rather than a color code. Used by `shimmer_ansi` to
/// preserve text styling when applying the shimmer color.
fn is_attribute_sgr(esc: &str) -> bool {
    // Parse the SGR parameter string between `[` and `m`.
    let Some(body) = esc.strip_prefix("[").and_then(|b| b.strip_suffix('m')) else {
        return false;
    };
    if body == "0" || body.is_empty() {
        return false;
    };
    // SGR parameters are semicolon-separated integers.
    body.split(';').all(|p| {
        matches!(
            p.trim(),
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" // bold, faint, italic, underline, blink, rapid, reverse, conceal, crossed-out
        )
    })
}

/// Parse an SGR foreground color from an accumulated SGR state, returning
/// the RGB color if one is found. Supports `38;2;R;G;Bm` (truecolor) and
/// basic ANSI `30`–`37` / `90`–`97` (mapped to approximate RGB).
fn parse_sgr_fg(sgr_state: &[&str]) -> Option<crate::palette::Rgb> {
    // Walk in reverse — last fg color wins.
    for esc in sgr_state.iter().rev() {
        let body = esc.strip_prefix("[").and_then(|b| b.strip_suffix('m'))?;
        // Truecolor: 38;2;R;G;B
        if body.starts_with("38;2;") {
            let parts: Vec<&str> = body.split(';').collect();
            if parts.len() == 5 {
                let r = parts[2].trim().parse::<u8>().ok()?;
                let g = parts[3].trim().parse::<u8>().ok()?;
                let b = parts[4].trim().parse::<u8>().ok()?;
                return Some(crate::palette::Rgb(r, g, b));
            }
        }
        // Indexed: 38;5;N (emitted when quantizing for 256-color terminals)
        if body.starts_with("38;5;") {
            let parts: Vec<&str> = body.split(';').collect();
            if parts.len() == 3 {
                let idx = parts[2].trim().parse::<u8>().ok()?;
                return Some(crate::theme::ansi_index_to_rgb(idx));
            }
        }
        // Basic 8/16 color fg: 30-37, 90-97
        if let Ok(n) = body.trim().parse::<u32>() {
            if (30..=37).contains(&n) {
                let idx = (n - 30) as usize;
                const ANSI8: [(u8, u8, u8); 8] = [
                    (0, 0, 0),       // black
                    (170, 0, 0),     // red
                    (0, 170, 0),     // green
                    (170, 85, 0),    // yellow/brown
                    (0, 0, 170),     // blue
                    (170, 0, 170),   // magenta
                    (0, 170, 170),   // cyan
                    (170, 170, 170), // white/gray
                ];
                let (r, g, b) = ANSI8[idx];
                return Some(crate::palette::Rgb(r, g, b));
            }
            if (90..=97).contains(&n) {
                let idx = (n - 90) as usize;
                const ANSI16: [(u8, u8, u8); 8] = [
                    (85, 85, 85),    // bright black (dark gray)
                    (255, 85, 85),   // bright red
                    (85, 255, 85),   // bright green
                    (255, 255, 85),  // bright yellow
                    (85, 85, 255),   // bright blue
                    (255, 85, 255),  // bright magenta
                    (85, 255, 255),  // bright cyan
                    (255, 255, 255), // bright white
                ];
                let (r, g, b) = ANSI16[idx];
                return Some(crate::palette::Rgb(r, g, b));
            }
        }
    }
    None
}

/// Apply a percentage-based, ANSI-aware shimmer sweep to a (possibly
/// ANSI-styled) string.
///
/// Unlike a fixed-column sweep (which desyncs across different-width
/// sections), the shimmer position is a **fraction of `messageWidth`** that
/// cycles over a fixed wall-clock period (`SHIMMER_CYCLE_MS`). All sections
/// — regardless of width — sweep at the same rate, and the highlight sits
/// at the same relative position across all sections simultaneously. This
/// makes the shimmer appear to "travel" across the entire statusbar in sync.
///
/// The sweep uses a reverse sawtooth wave (1->0, then wraps to 1) so the highlight
/// sweeps right-to-left and restarts from the right, producing a continuous
/// scan that cycles cleanly without bouncing back.
///
/// Instead of a hard color replacement, characters inside the shimmer
/// window are **blended** toward the `shimmer` color by an intensity that
/// peaks at the glimmer center and falls off at the edges. This makes the
/// shimmer visible even on light/white text (where a hard green replace
/// would flash jarringly) — white text shifts toward a green tint rather
/// than snapping to green. This matches claude-code's `interpolateColor`
/// approach in its "tool-use" shimmer mode.
///
/// ANSI-awareness: the function walks the string tracking the current SGR
/// state. When a character enters the shimmer window, it parses the
/// current fg color from the SGR state, blends it toward the shimmer
/// color, and emits the blended color. Text attributes (bold, italic,
/// underline) are preserved. After the shimmered char, the prior SGR state
/// is re-opened so subsequent characters retain their original colors.
pub fn shimmer_ansi(text: &str, shimmer: crate::palette::Rgb, elapsed_ms: u64) -> String {
    use crate::terminal::{escape_len, visible_width};
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    let message_width = visible_width(text);
    if message_width == 0 {
        return text.to_owned();
    }

    // Percentage-based shimmer position: a sawtooth wave over SHIMMER_CYCLE_MS
    // produces a value in 0.0..1.0 that represents the fraction of messageWidth
    // where the glimmer center sits. All sections using the same elapsed_ms get
    // the same fraction -> the shimmer syncs across all sections.
    let cycle_pos = (elapsed_ms % SHIMMER_CYCLE_MS) as f64 / SHIMMER_CYCLE_MS as f64;
    // Reverse sawtooth: the highlight sweeps right-to-left, then restarts from
    // the right. At cycle_pos=0 the center is at the right edge; it travels
    // leftward and wraps back to the right when the cycle restarts.
    let glimmer_center = (1.0 - cycle_pos) * message_width as f64;

    // Window width proportional to text, with a minimum.
    let window =
        ((message_width as f64) * SHIMMER_WINDOW_FRAC).max(SHIMMER_MIN_WINDOW as f64) as usize;
    let window_half = (window / 2) as isize;
    let shimmer_start = (glimmer_center as isize) - window_half;
    let shimmer_end = (glimmer_center as isize) + window_half;

    // If the shimmer window is entirely off-screen, return the text as-is.
    if shimmer_start >= message_width as isize || shimmer_end < 0 {
        return text.to_owned();
    }

    let clamped_start = shimmer_start.max(0) as usize;
    let clamped_end = shimmer_end.min(message_width as isize - 1) as usize;

    // Track the active SGR state so we can re-open it after shimmering chars.
    let mut out = String::with_capacity(text.len() + 128);
    let mut sgr_state: Vec<&str> = Vec::new();
    let mut col_pos: usize = 0;
    let mut rest = text;

    while !rest.is_empty() {
        // Detect ANSI escape sequence
        if let Some(stripped) = rest.strip_prefix('') {
            let len = escape_len(stripped) + 1;
            let esc = &rest[..len];
            // Track SGR state: clear on reset, accumulate otherwise
            if let Some(body) = esc.strip_prefix("[")
                && body.ends_with('m')
            {
                if matches!(body, "m" | "0m") {
                    sgr_state.clear();
                } else {
                    sgr_state.push(esc);
                }
            }
            out.push_str(esc);
            rest = &rest[len..];
            continue;
        }

        // Get the next grapheme cluster
        let grapheme = rest.graphemes(true).next().unwrap();
        let gw = grapheme.width();

        // Check if this char is in the shimmer window
        if col_pos >= clamped_start && col_pos <= clamped_end {
            // Compute blend intensity: peaks at glimmer center, falls off
            // linearly to 0 at the window edges.
            let dist_from_center = (col_pos as f64 - glimmer_center).abs();
            let intensity_frac = 1.0 - (dist_from_center / (window_half.max(1) as f64)).min(1.0);
            let blend =
                ((intensity_frac * SHIMMER_BLEND_PEAK as f64) as u8).min(SHIMMER_BLEND_PEAK);

            // Parse the current fg color from the SGR state so we can blend it.
            let base_fg = parse_sgr_fg(&sgr_state).unwrap_or(crate::palette::Rgb(170, 170, 170));
            let blended = crate::fade_toward(base_fg, shimmer, blend);
            let blended_rgb = crate::color_cap::fg_sequence(blended);

            // Emit blended color plus preserved text attributes.
            out.push_str(&blended_rgb);
            for sgr in &sgr_state {
                if is_attribute_sgr(sgr) {
                    out.push_str(sgr);
                }
            }
            out.push_str(grapheme);
            // Re-open the prior SGR state so subsequent chars keep their colors
            out.push_str("[0m");
            for sgr in &sgr_state {
                out.push_str(sgr);
            }
        } else {
            out.push_str(grapheme);
        }

        col_pos += gw;
        rest = &rest[grapheme.len()..];
    }

    out
}

/// Thinking expressions cycled while the agent is working (no active tool).
///
/// One quiet word each — the gradient animation carries the visual weight;
/// the label is just a status word, not competing copy.
pub const THINKING_EXPRESSIONS: [&str; 4] =
    ["thinking...", "reasoning...", "pondering...", "working..."];

/// Default expression shown when a tool is actively running.
pub const DEFAULT_TOOL_EXPRESSION: &str = "running...";

/// Number of milliseconds each thinking expression is displayed before cycling.
pub const THINKING_EXPRESSION_MS: u64 = 2000;

/// Return the thinking expression for the given elapsed time.
///
/// When `tool_active` is true, always returns the default tool expression.
/// Otherwise, cycles through [`THINKING_EXPRESSIONS`] every
/// [`THINKING_EXPRESSION_MS`] milliseconds.
pub fn thinking_expression(elapsed_ms: u64, tool_active: bool) -> &'static str {
    if tool_active {
        return DEFAULT_TOOL_EXPRESSION;
    }
    let idx = (elapsed_ms / THINKING_EXPRESSION_MS) as usize % THINKING_EXPRESSIONS.len();
    THINKING_EXPRESSIONS[idx]
}

/// Wall-clock-driven braille spinner.
///
/// Stores a start [`Instant`] so that [`frame`](Self::frame) always returns
/// the correct glyph for the current wall-clock time without manual ticking.
/// Multiple `Spinner` instances (or the [`spinner_frame_at`] free function)
/// will stay in phase because they all derive from the same clock.
#[derive(Debug)]
pub struct Spinner {
    start: Instant,
}

impl Default for Spinner {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Spinner {
    /// Construct a new spinner. Color arguments are accepted for API
    /// compatibility but ignored — the braille dots use terminal default fg.
    pub fn new(_accent: crate::palette::Rgb, _background: crate::palette::Rgb) -> Self {
        Self::default()
    }

    /// Current frame as a plain glyph string, computed from wall-clock
    /// elapsed time since construction.
    pub fn frame(&self) -> &str {
        spinner_frame_at(self.start.elapsed().as_millis() as u64)
    }

    /// No-op retained for API compatibility. The spinner is now wall-clock
    /// driven, so advancing a tick counter is unnecessary — [`frame`](Self::frame)
    /// always returns the correct glyph for the current time.
    pub fn tick(&mut self) -> &str {
        self.frame()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_non_empty() {
        let s = Spinner::default();
        assert!(!s.frame().is_empty());
    }

    #[test]
    fn frame_matches_free_function() {
        let s = Spinner::default();
        // The struct and free function should agree on the frame for the
        // same elapsed time (both derive from SPINNER_FRAMES + SPINNER_FRAME_MS).
        let elapsed = s.start.elapsed().as_millis() as u64;
        assert_eq!(s.frame(), spinner_frame_at(elapsed));
    }

    #[test]
    fn spinner_frame_at_wraps() {
        // Exactly one full cycle later → same frame as t=0.
        let cycle_ms = SPINNER_FRAME_MS * SPINNER_FRAMES.len() as u64;
        assert_eq!(spinner_frame_at(0), spinner_frame_at(cycle_ms));
        assert_eq!(spinner_frame_at(0), spinner_frame_at(cycle_ms * 7));
    }

    #[test]
    fn spinner_frame_at_advances() {
        let f0 = spinner_frame_at(0);
        let f1 = spinner_frame_at(SPINNER_FRAME_MS);
        assert_ne!(f0, f1);
    }

    #[test]
    fn equalizer_produces_5_bars() {
        let glyphs = equalizer_glyphs(0);
        assert_eq!(glyphs.chars().count(), EQUALIZER_BAR_COUNT);
    }

    #[test]
    fn equalizer_changes_over_time() {
        // At different timestamps the bar heights should differ (unless all
        // 5 sines happen to land on the same level, which is vanishingly
        // unlikely).
        let g0 = equalizer_glyphs(0);
        let g1 = equalizer_glyphs(200);
        assert_ne!(g0, g1);
    }

    #[test]
    fn equalizer_uses_only_block_chars() {
        let glyphs = equalizer_glyphs(42);
        for c in glyphs.chars() {
            assert!(
                EQUALIZER_BARS.contains(&c.to_string().as_str()),
                "unexpected char: {c}"
            );
        }
    }

    #[test]
    fn shimmer_constants_are_consistent() {
        // The shared constants must match the values that were previously
        // hardcoded in phi-mux (SIDEBAR_ANIMATION_FRAME_MS = 40, % 24, * 21).
        assert_eq!(SHIMMER_FRAME_MS, 40);
        assert_eq!(SHIMMER_TRIANGLE_STEPS, 24);
        assert_eq!(SHIMMER_TRIANGLE_BLEND, 21);
        // Full cycle = 24 * 40 = 960 ms.
        assert_eq!(SHIMMER_FRAME_MS * SHIMMER_TRIANGLE_STEPS as u64, 960);
        // THINKING_ANIMATION_FRAME_MS is now an alias for SHIMMER_FRAME_MS.
        assert_eq!(THINKING_ANIMATION_FRAME_MS, SHIMMER_FRAME_MS);
    }

    #[test]
    fn shimmer_triangle_intensity_matches_old_inline_math() {
        // Verify that shimmer_triangle_intensity produces the same values as
        // the old inline formula: phase = (frame + slot) % 24, intensity =
        // if phase < 12 { phase } else { 24 - phase }.
        for now_ms in (0..2000).step_by(40) {
            for slot in 0..30 {
                let frame = (now_ms / SHIMMER_FRAME_MS) as usize;
                let phase = (frame + slot) % SHIMMER_TRIANGLE_STEPS;
                let expected = if phase < 12 {
                    phase as u8
                } else {
                    (24 - phase) as u8
                };
                assert_eq!(
                    shimmer_triangle_intensity(now_ms, slot),
                    expected,
                    "mismatch at now_ms={now_ms}, slot={slot}"
                );
            }
        }
    }

    #[test]
    fn shimmer_triangle_blend_clamps() {
        // Peak intensity = 12, blend = 12 * 21 = 252.
        assert_eq!(shimmer_triangle_blend(12), 252);
        // Zero intensity → zero blend.
        assert_eq!(shimmer_triangle_blend(0), 0);
        // Saturates at the u8 ceiling for high intensities.
        assert_eq!(shimmer_triangle_blend(255), 255);
    }

    #[test]
    fn shimmer_triangle_intensity_cycles() {
        // One full cycle later → same intensity.
        let cycle = SHIMMER_FRAME_MS * SHIMMER_TRIANGLE_STEPS as u64;
        for slot in 0..24 {
            assert_eq!(
                shimmer_triangle_intensity(300, slot),
                shimmer_triangle_intensity(300 + cycle, slot),
                "not periodic at slot={slot}"
            );
        }
    }

    #[test]
    fn gradient_text_animated_uses_shared_blend() {
        // The animated gradient should blend at most SHIMMER_TRIANGLE_BLEND
        // * (STEPS/2) = 252/255 toward the shimmer color. Verify that at the
        // peak the output differs from the base gradient but is not a hard
        // replacement (i.e. it's a blend, not shimmer == base).
        crate::color_cap::force_color_capability(crate::color_cap::ColorCapability::TrueColor);
        let from = crate::palette::Rgb(0, 0, 0);
        let to = crate::palette::Rgb(0, 0, 0);
        let shimmer = crate::palette::Rgb(255, 255, 255);
        // Use enough chars that some char index falls on a peak slot.
        let text = "abcdefghijklmnop"; // 16 chars, indices 0..15
        // Find a timestamp where some char gets intensity 12.
        for ms in (0..960).step_by(40) {
            let result = gradient_text_animated(text, from, to, shimmer, ms);
            if result.contains("38;2;252;252;252m") {
                // Peak blend (12 * 21 = 252) found.
                return;
            }
        }
        panic!("never found a peak blend (252) in the gradient output");
    }

    #[test]
    fn thinking_expression_cycles() {
        assert_eq!(thinking_expression(0, false), THINKING_EXPRESSIONS[0]);
        assert_eq!(
            thinking_expression(THINKING_EXPRESSION_MS, false),
            THINKING_EXPRESSIONS[1]
        );
        // Wraps around after the full cycle.
        let full_cycle = THINKING_EXPRESSION_MS * THINKING_EXPRESSIONS.len() as u64;
        assert_eq!(
            thinking_expression(full_cycle, false),
            THINKING_EXPRESSIONS[0]
        );
    }

    #[test]
    fn thinking_expression_tool_active_returns_default() {
        assert_eq!(thinking_expression(0, true), DEFAULT_TOOL_EXPRESSION);
        assert_eq!(thinking_expression(99999, true), DEFAULT_TOOL_EXPRESSION);
    }

    #[test]
    fn shimmer_ansi_preserves_visible_width() {
        let text = "[1m[37mread[0m [38;2;130;180;230msrc/parser.rs[0m";
        let shimmer = crate::palette::Rgb(80, 250, 120);
        let width = crate::terminal::visible_width(text);
        // Sample across the full cycle.
        for ms in (0..SHIMMER_CYCLE_MS).step_by(200) {
            let result = shimmer_ansi(text, shimmer, ms);
            assert_eq!(
                crate::terminal::visible_width(&result),
                width,
                "width changed at ms={ms}"
            );
        }
    }

    #[test]
    fn shimmer_ansi_preserves_ansi_escapes() {
        let text = "[1m[37mread[0m file";
        let shimmer = crate::palette::Rgb(80, 250, 120);
        for ms in (0..SHIMMER_CYCLE_MS).step_by(100) {
            let result = shimmer_ansi(text, shimmer, ms);
            // The bold code must be present (attributes preserved).
            assert!(result.contains("[1m"), "bold code lost at ms={ms}");
        }
    }

    #[test]
    fn shimmer_ansi_blends_at_midcycle() {
        // At midcycle (SHIMMER_CYCLE_MS / 2), reverse sawtooth = 0.5, glimmer center
        // = 0.5 * messageWidth. For "hello" (width=5), center=2.5, window = max(1, 3) = 3,
        // half = 1. Shimmer window = [1.5, 3.5] clamped to [2, 4]. The char at
        // col 2 ('l') should get a blended color.
        let text = "hello";
        let shimmer = crate::palette::Rgb(80, 250, 120);
        let elapsed = SHIMMER_CYCLE_MS / 2;
        let result = shimmer_ansi(text, shimmer, elapsed);
        // A blended fg escape should appear (not the exact shimmer color since
        // it's a blend, but a truecolor escape modifying the fg).
        assert!(
            result.contains("[38;2;"),
            "blended color not found in output: {result:?}"
        );
        assert_eq!(crate::terminal::visible_width(&result), 5);
    }

    #[test]
    fn shimmer_ansi_empty_text() {
        let result = shimmer_ansi("", crate::palette::Rgb(80, 250, 120), 1000);
        assert!(result.is_empty());
    }

    #[test]
    fn shimmer_ansi_cycles() {
        // One full cycle later, the output should match the starting output.
        let text = "read src/parser.rs";
        let shimmer = crate::palette::Rgb(80, 250, 120);
        let a = shimmer_ansi(text, shimmer, 300);
        let b = shimmer_ansi(text, shimmer, 300 + SHIMMER_CYCLE_MS);
        assert_eq!(a, b);
    }

    #[test]
    fn shimmer_ansi_syncs_across_widths() {
        // Percentage-based: at the same elapsed_ms, the shimmer center is at
        // the same *fraction* of width for different texts. Verify by checking
        // that two texts of different widths both show a blended color at
        // quarter-cycle (reverse sawtooth=0.75, center = 0.75*width).
        let short = "hi";
        let long = "read src/parser.rs";
        let shimmer = crate::palette::Rgb(80, 250, 120);
        let elapsed = SHIMMER_CYCLE_MS / 4; // reverse sawtooth = 0.75
        let r_short = shimmer_ansi(short, shimmer, elapsed);
        let r_long = shimmer_ansi(long, shimmer, elapsed);
        // Both should contain a blended fg escape (both have their center
        // within their text at 75% position).
        assert!(
            r_short.contains("[38;2;"),
            "short text not shimmered: {r_short:?}"
        );
        assert!(
            r_long.contains("[38;2;"),
            "long text not shimmered: {r_long:?}"
        );
    }

    #[test]
    fn shimmer_ansi_white_text_blends_visible() {
        // White text (bold white = [1m[37m) should produce a visible
        // blend toward the shimmer color, not a hard replacement. The blended
        // color should be between white (255,255,255) and shimmer (80,250,120).
        let text = "[1m[37mread[0m";
        let shimmer = crate::palette::Rgb(80, 250, 120);
        let elapsed = SHIMMER_CYCLE_MS / 4;
        let result = shimmer_ansi(text, shimmer, elapsed);
        // The output should contain a truecolor fg escape.
        assert!(
            result.contains("[38;2;"),
            "no blend color found for white text: {result:?}"
        );
        // At least one of those truecolor escapes should NOT be pure white,
        // proving the color was blended toward the shimmer color.
        let mut found_non_white = false;
        for part in result.split("[38;2;").skip(1) {
            if let Some(m_end) = part.find('m')
                && &part[..m_end] != "255;255;255"
            {
                found_non_white = true;
                break;
            }
        }
        assert!(
            found_non_white,
            "white text was not blended toward shimmer: {result:?}"
        );
    }
}
