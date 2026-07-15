//! The stock look-and-feel: a palette generated from the host terminal's own
//! colors (OSC 10/11/4 probe), plus the braille spinner, registered as
//! theme/spinner extensions. The out-of-the-box look is itself an extension —
//! the core's only visual opinion is a minimal monochrome fallback for the
//! bare harness.

use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{
    Color, Extension, ExtensionHost, ExtensionManifest, SpinnerSpec, ThemePalette, ThemeSpec,
};
use ekko_tui::theme::generate_theme;
use ekko_tui::{Rgb, SPINNER_FRAME_MS, SPINNER_FRAMES, TerminalColors, default_terminal_colors};

fn color(c: Rgb) -> Color {
    Color::rgb(c.0, c.1, c.2)
}

/// Map the unified generated [`ekko_tui::theme::Theme`] onto the extension
/// palette role vocabulary. Pane and sidebar backgrounds stay transparent so
/// the host terminal's own background shows through; chrome surfaces use the
/// gray ramp derived from that background.
pub fn terminal_palette(colors: &TerminalColors) -> ThemePalette {
    let t = generate_theme(colors);
    ThemePalette {
        text: color(t.foreground),
        muted: color(t.muted),
        heading: color(t.foreground),
        accent: color(t.accent),
        accent_2: color(t.accent_2),
        surface: color(t.surface),
        surface_raised: color(t.surface_raised),
        sidebar_bg: Color::TRANSPARENT,
        status_fg: color(t.foreground),
        status_bg: color(t.surface_raised),
        border: color(t.border),
        running: color(t.running),
        warning: color(t.warning),
        error: color(t.error),
        success: color(t.success),
        term_fg: color(t.foreground),
        term_bg: Color::TRANSPARENT,
        selection_fg: color(t.selection_foreground),
        selection_bg: color(t.selection_background),
        ansi: t.ansi.map(color),
    }
}

/// Registers the terminal-derived theme. Holds the colors probed by the host
/// at startup; when the terminal didn't answer the OSC queries (SSH, CI, a
/// terminal without color reporting) it derives from the standard ANSI
/// palette instead.
pub struct ThemeExtension {
    colors: TerminalColors,
}

impl ThemeExtension {
    pub fn new(colors: Option<TerminalColors>) -> Self {
        Self {
            colors: colors.unwrap_or_else(default_terminal_colors),
        }
    }
}

impl Extension for ThemeExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.theme".into(),
            name: "terminal theme".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "palette generated from the host terminal's colors + braille spinner"
                .into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.register_theme(ThemeSpec {
            name: "terminal".into(),
            description: "generated from the host terminal's reported colors".into(),
            palette: terminal_palette(&self.colors),
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

    fn colors_from(bg: Rgb, fg: Rgb) -> TerminalColors {
        TerminalColors {
            background: bg,
            foreground: fg,
            palette: std::array::from_fn(|i| Some(Rgb(i as u8 * 8, i as u8 * 8, i as u8 * 8))),
        }
    }

    #[test]
    fn palette_roles_follow_the_terminal_ansi_palette() {
        let colors = colors_from(Rgb(20, 20, 26), Rgb(220, 220, 220));
        let palette = terminal_palette(&colors);

        assert_eq!(palette.text, Color::rgb(220, 220, 220));
        assert_eq!(palette.term_fg, palette.text);
        assert_eq!(palette.selection_fg, palette.term_fg);
        assert_eq!(palette.selection_bg, palette.border);
        assert_eq!(palette.accent, palette.ansi[6]);
        assert_eq!(palette.accent_2, palette.ansi[5]);
        assert_eq!(palette.error, palette.ansi[1]);
        assert_eq!(palette.success, palette.ansi[2]);
        assert_eq!(palette.warning, palette.ansi[3]);
        assert_eq!(palette.running, palette.ansi[10]);
    }

    #[test]
    fn pane_and_sidebar_backgrounds_stay_transparent() {
        let palette = terminal_palette(&default_terminal_colors());
        assert_eq!(palette.term_bg, Color::TRANSPARENT);
        assert_eq!(palette.sidebar_bg, Color::TRANSPARENT);
    }

    #[test]
    fn statusbar_uses_a_raised_surface_derived_from_the_background() {
        let colors = colors_from(Rgb(20, 20, 26), Rgb(220, 220, 220));
        let palette = terminal_palette(&colors);
        assert_eq!(palette.status_bg, palette.surface_raised);
        assert_ne!(palette.surface_raised, Color::rgb(20, 20, 26));
        assert_ne!(palette.status_bg, palette.status_fg);
    }

    #[test]
    fn extension_defaults_to_the_standard_ansi_palette_when_probe_fails() {
        let ext = ThemeExtension::new(None);
        assert_eq!(ext.colors, default_terminal_colors());
    }
}
