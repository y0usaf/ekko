//! Color utilities in the mux `Color` space: ANSI 256-color cube resolution
//! and the `brighten` / `fade_toward` blends the client uses when resolving
//! wire cells. All theme *derivation* lives in `ekko_tui::theme` — the active
//! palette reaches the client as an `ekko_ext::ThemePalette` from the theme
//! extension.

use crate::color::Color;
use ekko_tui::ansi_index_to_rgb;

/// Resolve an ANSI 256-color index to a `Color` via the standard 256-color cube.
pub fn ansi_index_to_color(idx: u8) -> Color {
    let r = ansi_index_to_rgb(idx);
    Color::rgb(r.0, r.1, r.2)
}

/// Saturating-add `amount` to each channel.  ANSI-indexed colors pass through
/// unchanged (they are resolved by the terminal, not by us).
pub fn brighten(color: Color, amount: u8) -> Color {
    if color.ansi_index_value().is_some() {
        return color;
    }
    let (r, g, b) = color.rgb_components();
    Color::rgb(
        r.saturating_add(amount),
        g.saturating_add(amount),
        b.saturating_add(amount),
    )
}

/// Linearly interpolate `color` toward `target` by `mix` (0..255).
/// ANSI-indexed colors pass through (threshold behavior at mix 128).
pub fn fade_toward(color: Color, target: Color, mix: u8) -> Color {
    if color.ansi_index_value().is_some() || target.ansi_index_value().is_some() {
        return if mix < 128 { color } else { target };
    }
    let (r1, g1, b1) = color.rgb_components();
    let (r2, g2, b2) = target.rgb_components();
    let blend = |a: u8, b: u8| -> u8 {
        let a = u16::from(a);
        let b = u16::from(b);
        let mix = u16::from(mix);
        (((a * (255 - mix)) + (b * mix)) / 255) as u8
    };
    Color::rgb(blend(r1, r2), blend(g1, g2), blend(b1, b2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_index_to_color_handles_base_cube_and_grayscale_ranges() {
        assert_eq!(ansi_index_to_color(1), Color::rgb(0xf7, 0x76, 0x8e));
        assert_eq!(ansi_index_to_color(16), Color::rgb(0, 0, 0));
        assert_eq!(ansi_index_to_color(231), Color::rgb(255, 255, 255));
        assert_eq!(ansi_index_to_color(232), Color::rgb(8, 8, 8));
        assert_eq!(ansi_index_to_color(255), Color::rgb(238, 238, 238));
    }

    #[test]
    fn indexed_colors_survive_brighten_and_fade() {
        let indexed = Color::ansi_index(12);
        let target = Color::rgb(100, 120, 140);
        assert_eq!(brighten(indexed, 20), indexed);
        assert_eq!(fade_toward(indexed, target, 64), indexed);
        assert_eq!(fade_toward(indexed, target, 192), target);
    }

    #[test]
    fn brighten_saturates_and_fade_interpolates() {
        assert_eq!(
            brighten(Color::rgb(250, 250, 250), 20),
            Color::rgb(255, 255, 255)
        );
        assert_eq!(
            fade_toward(Color::rgb(0, 0, 0), Color::rgb(255, 255, 255), 128),
            Color::rgb(128, 128, 128)
        );
    }
}
