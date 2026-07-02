//! Terminal color capability: detect whether the host terminal renders
//! 24-bit color, and quantize to the xterm-256 palette when it doesn't.
//! Emitting raw truecolor SGRs into a terminal that never advertised them
//! renders wrong colors (or none) on urxvt, screen, and older xterms.

use std::sync::OnceLock;

use crate::palette::Rgb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCapability {
    TrueColor,
    Color256,
}

static CAPABILITY: OnceLock<ColorCapability> = OnceLock::new();

/// The detected capability, computed once from the environment.
pub fn color_capability() -> ColorCapability {
    *CAPABILITY.get_or_init(detect)
}

pub fn has_truecolor() -> bool {
    color_capability() == ColorCapability::TrueColor
}

/// Pin the capability before first use — for tests that assert on exact
/// escape sequences and must not depend on the harness environment.
/// First caller wins; a no-op once detection has run.
#[doc(hidden)]
pub fn force_color_capability(cap: ColorCapability) {
    let _ = CAPABILITY.set(cap);
}

fn detect() -> ColorCapability {
    // Explicit user override beats all heuristics.
    if let Ok(raw) = std::env::var("PHI_COLOR") {
        match raw.trim().to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" => return ColorCapability::TrueColor,
            "256" | "256color" => return ColorCapability::Color256,
            _ => {}
        }
    }
    let raw = detect_from_env();
    // Some macOS terminal renderers key their GPU glyph atlas on the full
    // 24-bit cell color; phi's continuous color animations (shimmer,
    // gradients) then generate an unbounded set of atlas entries and the
    // cache corrupts (garbled glyphs). Quantizing to the 256 palette
    // bounds the distinct-color space for exactly those terminals.
    if raw == ColorCapability::TrueColor
        && cfg!(target_os = "macos")
        && matches!(
            std::env::var("TERM_PROGRAM").map(|v| v.to_ascii_lowercase()),
            Ok(ref tp) if tp == "vscode" || tp == "apple_terminal"
        )
    {
        return ColorCapability::Color256;
    }
    raw
}

fn detect_from_env() -> ColorCapability {
    if let Ok(val) = std::env::var("COLORTERM") {
        let v = val.to_ascii_lowercase();
        if v == "truecolor" || v == "24bit" {
            return ColorCapability::TrueColor;
        }
    }
    if let Ok(tp) = std::env::var("TERM_PROGRAM") {
        let tp = tp.to_ascii_lowercase();
        if matches!(
            tp.as_str(),
            "ghostty" | "iterm.app" | "wezterm" | "warp" | "alacritty" | "hyper"
        ) {
            return ColorCapability::TrueColor;
        }
    }
    // Terminals that reliably do truecolor but don't always set COLORTERM.
    if std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some()
        || std::env::var_os("GHOSTTY_BIN_DIR").is_some()
        || std::env::var_os("WEZTERM_EXECUTABLE").is_some()
        || std::env::var_os("WEZTERM_PANE").is_some()
        || std::env::var_os("KITTY_WINDOW_ID").is_some()
    {
        return ColorCapability::TrueColor;
    }
    if let Ok(term) = std::env::var("TERM") {
        let t = term.to_ascii_lowercase();
        if t.contains("kitty") || t.contains("ghostty") || t.contains("alacritty") {
            return ColorCapability::TrueColor;
        }
    }
    ColorCapability::Color256
}

/// Foreground SGR sequence for `rgb`, honoring the detected capability:
/// `ESC[38;2;r;g;bm` on truecolor, `ESC[38;5;Nm` quantized otherwise.
pub fn fg_sequence(rgb: Rgb) -> String {
    format!("\x1b[{}m", fg_params(rgb))
}

/// Background SGR sequence for `rgb` (see [`fg_sequence`]).
pub fn bg_sequence(rgb: Rgb) -> String {
    format!("\x1b[{}m", bg_params(rgb))
}

/// Bare foreground SGR parameters (no `ESC[`/`m` framing), for callers
/// that splice parameters into a larger SGR.
pub fn fg_params(rgb: Rgb) -> String {
    match color_capability() {
        ColorCapability::TrueColor => format!("38;2;{};{};{}", rgb.0, rgb.1, rgb.2),
        ColorCapability::Color256 => format!("38;5;{}", rgb_to_xterm256(rgb.0, rgb.1, rgb.2)),
    }
}

/// Bare background SGR parameters (see [`fg_params`]).
pub fn bg_params(rgb: Rgb) -> String {
    match color_capability() {
        ColorCapability::TrueColor => format!("48;2;{};{};{}", rgb.0, rgb.1, rgb.2),
        ColorCapability::Color256 => format!("48;5;{}", rgb_to_xterm256(rgb.0, rgb.1, rgb.2)),
    }
}

/// Nearest xterm-256 index for an RGB color. Indices 16-231 form a 6x6x6
/// cube (axis values 0, 95, 135, 175, 215, 255); 232-255 are a grayscale
/// ramp from 8 to 238 in steps of 10. Near-gray colors compare against
/// both and take the closer, since the ramp is much finer than the cube
/// diagonal.
pub fn rgb_to_xterm256(r: u8, g: u8, b: u8) -> u8 {
    let is_grayish = (r as i16 - g as i16).unsigned_abs() < 15
        && (g as i16 - b as i16).unsigned_abs() < 15
        && (r as i16 - b as i16).unsigned_abs() < 15;

    let cube_idx = nearest_cube_index(r, g, b);
    let cube = cube_index_to_rgb(cube_idx);
    let cube_dist = color_distance(r, g, b, cube.0, cube.1, cube.2);

    if is_grayish {
        let gray_avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
        let gray_idx = nearest_gray_index(gray_avg);
        let gray_val = gray_index_to_value(gray_idx);
        if color_distance(r, g, b, gray_val, gray_val, gray_val) < cube_dist {
            return 232 + gray_idx;
        }
    }
    cube_idx as u8 + 16
}

const CUBE_VALUES: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn nearest_cube_component(v: u8) -> u8 {
    let mut best = 0u8;
    let mut best_dist = u16::MAX;
    for (i, &cv) in CUBE_VALUES.iter().enumerate() {
        let d = (v as i16 - cv as i16).unsigned_abs();
        if d < best_dist {
            best_dist = d;
            best = i as u8;
        }
    }
    best
}

fn nearest_cube_index(r: u8, g: u8, b: u8) -> u16 {
    let ri = nearest_cube_component(r) as u16;
    let gi = nearest_cube_component(g) as u16;
    let bi = nearest_cube_component(b) as u16;
    ri * 36 + gi * 6 + bi
}

fn cube_index_to_rgb(idx: u16) -> (u8, u8, u8) {
    let bi = (idx % 6) as usize;
    let gi = ((idx / 6) % 6) as usize;
    let ri = (idx / 36) as usize;
    (CUBE_VALUES[ri], CUBE_VALUES[gi], CUBE_VALUES[bi])
}

fn nearest_gray_index(v: u8) -> u8 {
    // Ramp values 8, 18, ..., 238. Signed math so 0..=7 round to index 0
    // instead of underflowing.
    if v > 243 {
        return 23;
    }
    (((v as i16 - 8 + 5) / 10).clamp(0, 23)) as u8
}

fn gray_index_to_value(idx: u8) -> u8 {
    8 + idx * 10
}

fn color_distance(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> u32 {
    let dr = r1 as i32 - r2 as i32;
    let dg = g1 as i32 - g2 as i32;
    let db = b1 as i32 - b2 as i32;
    // Weighted: the eye is most sensitive to green.
    (2 * dr * dr + 4 * dg * dg + 3 * db * db) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizes_cube_corners_exactly() {
        assert_eq!(rgb_to_xterm256(0, 0, 0), 16);
        assert_eq!(rgb_to_xterm256(255, 255, 255), 231);
        assert_eq!(rgb_to_xterm256(255, 0, 0), 196);
        assert_eq!(rgb_to_xterm256(0, 255, 0), 46);
        assert_eq!(rgb_to_xterm256(0, 0, 255), 21);
    }

    #[test]
    fn quantizes_grays_to_the_ramp() {
        // 128,128,128 sits between cube 135 and ramp 128 (idx 232+12).
        assert_eq!(rgb_to_xterm256(128, 128, 128), 232 + 12);
        assert_eq!(rgb_to_xterm256(8, 8, 8), 232);
        assert_eq!(rgb_to_xterm256(238, 238, 238), 255);
    }

    #[test]
    fn roundtrip_stays_close() {
        // Quantizing any color and mapping the index back must stay within
        // half a cube step on every channel.
        for &(r, g, b) in &[(130u8, 160u8, 230u8), (250, 210, 120), (30, 30, 40)] {
            let idx = rgb_to_xterm256(r, g, b);
            let back = if idx >= 232 {
                let v = gray_index_to_value(idx - 232);
                (v, v, v)
            } else {
                cube_index_to_rgb((idx - 16) as u16)
            };
            assert!(
                (r as i16 - back.0 as i16).unsigned_abs() <= 40,
                "{r} vs {}",
                back.0
            );
            assert!(
                (g as i16 - back.1 as i16).unsigned_abs() <= 40,
                "{g} vs {}",
                back.1
            );
            assert!(
                (b as i16 - back.2 as i16).unsigned_abs() <= 40,
                "{b} vs {}",
                back.2
            );
        }
    }
}
