//! Extension-space color and visual specs.
//!
//! [`Color`] mirrors the renderer's packed ARGB representation (alpha tag
//! `0x01` marks an ANSI palette index; alpha `0x00` means transparent —
//! the host terminal's own background shows through) so conversion at the
//! host boundary is a lossless bit copy without this crate depending on the
//! renderer.

use std::sync::Arc;

/// Packed ARGB color in extension space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    const ANSI_INDEX_ALPHA: u32 = 0x01;

    /// Fully transparent: the host terminal's background shows through.
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    pub const fn ansi_index(index: u8) -> Self {
        Self((Self::ANSI_INDEX_ALPHA << 24) | index as u32)
    }

    pub const fn ansi_index_value(self) -> Option<u8> {
        if self.0 >> 24 == Self::ANSI_INDEX_ALPHA {
            Some((self.0 & 0xff) as u8)
        } else {
            None
        }
    }

    #[inline]
    pub const fn rgb_components(self) -> (u8, u8, u8) {
        let value = self.0;
        (
            ((value >> 16) & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            (value & 0xff) as u8,
        )
    }
}

/// Saturating-add `amount` to each channel. ANSI-indexed colors pass through
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

/// The resolved chrome palette handed to extensions through the snapshot.
///
/// Field set is exactly the role vocabulary the frame chrome consumes; the
/// host converts a registered theme's palette into its internal theme type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePalette {
    pub text: Color,
    pub muted: Color,
    pub heading: Color,
    pub accent: Color,
    pub accent_2: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub sidebar_bg: Color,
    pub status_fg: Color,
    pub status_bg: Color,
    pub border: Color,
    pub running: Color,
    pub warning: Color,
    pub error: Color,
    pub success: Color,
    pub term_fg: Color,
    pub term_bg: Color,
    /// Foreground used for mouse-selected terminal text.
    pub selection_fg: Color,
    /// Opaque background used for mouse-selected terminal text.
    pub selection_bg: Color,
    pub ansi: [Color; 16],
}

impl ThemePalette {
    /// Minimal monochrome fallback used when no theme extension is
    /// registered (the bare-harness case): everything readable, nothing
    /// styled. Real look-and-feel is a builtin's job.
    pub fn fallback() -> Self {
        let fg = Color::rgb(0xC0, 0xC0, 0xC0);
        let dim = Color::rgb(0x80, 0x80, 0x80);
        Self {
            text: fg,
            muted: dim,
            heading: fg,
            accent: fg,
            accent_2: fg,
            surface: Color::TRANSPARENT,
            surface_raised: Color::TRANSPARENT,
            sidebar_bg: Color::TRANSPARENT,
            status_fg: Color::rgb(0x10, 0x10, 0x10),
            status_bg: dim,
            border: dim,
            running: fg,
            warning: fg,
            error: fg,
            success: fg,
            term_fg: fg,
            term_bg: Color::TRANSPARENT,
            selection_fg: fg,
            selection_bg: dim,
            ansi: std::array::from_fn(|i| Color::ansi_index(i as u8)),
        }
    }
}

/// A named color theme contributed by an extension.
#[derive(Clone)]
pub struct ThemeSpec {
    pub name: String,
    pub description: String,
    pub palette: ThemePalette,
}

/// A named spinner animation (frame glyphs + per-frame interval).
#[derive(Clone)]
pub struct SpinnerSpec {
    pub name: String,
    pub frames: Arc<Vec<String>>,
    pub interval_ms: u64,
}

impl SpinnerSpec {
    /// The frame glyph for a wall-clock timestamp.
    pub fn frame_at(&self, now_ms: u64) -> &str {
        if self.frames.is_empty() {
            return "";
        }
        let interval = self.interval_ms.max(1);
        let index = (now_ms / interval) as usize % self.frames.len();
        &self.frames[index]
    }
}
