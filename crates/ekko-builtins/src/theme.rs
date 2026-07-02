//! The stock look-and-feel: the Charmtone Pantera palette and the braille
//! spinner, registered as theme/spinner extensions. The out-of-the-box look
//! is itself an extension — the core's only visual opinion is a minimal
//! monochrome fallback for the bare harness.

use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{
    Color, Extension, ExtensionHost, ExtensionManifest, SpinnerSpec, ThemePalette, ThemeSpec,
};
use ekko_tui::{SPINNER_FRAME_MS, SPINNER_FRAMES};

// Charmtone palette constants (verbatim hex values from
// `github.com/charmbracelet/x/exp/charmtone`), the subset Pantera uses.
mod ct {
    use ekko_ext::Color;

    pub const SRIRACHA: Color = Color::rgb(0xEB, 0x42, 0x68);
    pub const CORAL: Color = Color::rgb(0xFF, 0x57, 0x7D);
    pub const DOLLY: Color = Color::rgb(0xFF, 0x60, 0xFF);
    pub const BLUSH: Color = Color::rgb(0xFF, 0x84, 0xFF);
    pub const CHARPLE: Color = Color::rgb(0x6B, 0x50, 0xFF);
    pub const ANCHOVY: Color = Color::rgb(0x71, 0x9A, 0xFC);
    pub const MALIBU: Color = Color::rgb(0x00, 0xA4, 0xFF);
    pub const SARDINE: Color = Color::rgb(0x4F, 0xBE, 0xFE);
    pub const GUAC: Color = Color::rgb(0x12, 0xC7, 0x8F);
    pub const JULEP: Color = Color::rgb(0x00, 0xFF, 0xB2);
    pub const BOK: Color = Color::rgb(0x68, 0xFF, 0xD6);
    pub const MUSTARD: Color = Color::rgb(0xF5, 0xEF, 0x34);
    pub const CITRON: Color = Color::rgb(0xE8, 0xFF, 0x27);
    pub const PEPPER: Color = Color::rgb(0x20, 0x1F, 0x26);
    pub const BBQ: Color = Color::rgb(0x2D, 0x2C, 0x35);
    pub const CHARCOAL: Color = Color::rgb(0x3A, 0x39, 0x43);
    pub const IRON: Color = Color::rgb(0x4D, 0x4C, 0x57);
    pub const SQUID: Color = Color::rgb(0x85, 0x83, 0x92);
    pub const SMOKE: Color = Color::rgb(0xBF, 0xBC, 0xC8);
    pub const ASH: Color = Color::rgb(0xDF, 0xDB, 0xDD);
    pub const BUTTER: Color = Color::rgb(0xFF, 0xFA, 0xF1);
}

/// Charmtone Pantera — mirrors Crush's `CharmtonePantera` theme. Sidebar and
/// terminal backgrounds stay transparent so the host terminal's own
/// background shows through.
pub fn pantera() -> ThemePalette {
    ThemePalette {
        text: ct::ASH,
        muted: ct::SQUID,
        heading: ct::SMOKE,
        accent: ct::BOK,
        accent_2: ct::DOLLY,
        surface: ct::PEPPER,
        surface_raised: ct::BBQ,
        sidebar_bg: Color::TRANSPARENT,
        status_fg: ct::BUTTER,
        status_bg: ct::CHARPLE,
        border: ct::CHARCOAL,
        running: ct::CITRON,
        warning: ct::MUSTARD,
        error: ct::SRIRACHA,
        success: ct::JULEP,
        term_fg: ct::ASH,
        term_bg: Color::TRANSPARENT,
        ansi: [
            ct::PEPPER,
            ct::SRIRACHA,
            ct::JULEP,
            ct::MUSTARD,
            ct::MALIBU,
            ct::DOLLY,
            ct::BOK,
            ct::SMOKE,
            ct::IRON,
            ct::CORAL,
            ct::GUAC,
            ct::CITRON,
            ct::ANCHOVY,
            ct::BLUSH,
            ct::SARDINE,
            ct::ASH,
        ],
    }
}

pub struct ThemeExtension;

impl Extension for ThemeExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.theme".into(),
            name: "pantera theme".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Charmtone Pantera chrome palette + braille spinner".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.register_theme(ThemeSpec {
            name: "pantera".into(),
            description: "Charmtone Pantera (Crush's default dark theme)".into(),
            palette: pantera(),
        })?;
        host.register_spinner(SpinnerSpec {
            name: "braille".into(),
            frames: Arc::new(SPINNER_FRAMES.iter().map(|s| s.to_string()).collect()),
            interval_ms: SPINNER_FRAME_MS,
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pantera_matches_charmtone_role_mapping() {
        let theme = pantera();
        assert_eq!(theme.accent, ct::BOK);
        assert_eq!(theme.text, ct::ASH);
        assert_eq!(theme.error, ct::SRIRACHA);
        assert_eq!(theme.running, ct::CITRON);
        assert_eq!(theme.success, ct::JULEP);
        assert_eq!(theme.status_bg, ct::CHARPLE);
        assert_eq!(theme.status_fg, ct::BUTTER);
        assert_eq!(theme.border, ct::CHARCOAL);
        assert_eq!(theme.sidebar_bg, Color::TRANSPARENT);
        assert_eq!(theme.term_bg, Color::TRANSPARENT);
    }

    #[test]
    fn charmtone_hexes_are_verbatim() {
        assert_eq!(ct::CHARPLE, Color::rgb(0x6B, 0x50, 0xFF));
        assert_eq!(ct::BOK, Color::rgb(0x68, 0xFF, 0xD6));
        assert_eq!(ct::DOLLY, Color::rgb(0xFF, 0x60, 0xFF));
        assert_eq!(ct::SRIRACHA, Color::rgb(0xEB, 0x42, 0x68));
        assert_eq!(ct::PEPPER, Color::rgb(0x20, 0x1F, 0x26));
        assert_eq!(ct::BUTTER, Color::rgb(0xFF, 0xFA, 0xF1));
    }
}
