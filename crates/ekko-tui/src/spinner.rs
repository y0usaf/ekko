//! Simple braille-dot spinner for the live status line.
//!
//! A lightweight 10-frame braille spinner with no color blending, no trail
//! effects, and no pre-computed frames — just a static array of glyphs.
//!
//! This is the **single source of truth** for spinner frames and timing:
//! every consumer should use [`SPINNER_FRAMES`], [`SPINNER_FRAME_MS`], and
//! [`spinner_frame_at`] rather than defining its own copy.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_index_cycles_through_all_frames() {
        for (i, frame) in SPINNER_FRAMES.iter().enumerate() {
            let ms = i as u64 * SPINNER_FRAME_MS;
            assert_eq!(spinner_frame_index(ms), i);
            assert_eq!(spinner_frame_at(ms), *frame);
        }
        // Wraps after a full cycle.
        let cycle = SPINNER_FRAME_MS * SPINNER_FRAMES.len() as u64;
        assert_eq!(spinner_frame_index(cycle), 0);
    }
}
