//! Scroll mode: keyboard navigation through the session's server-side
//! scrollback, registered through the public API. The mechanism (the
//! `Scroll`/`ScrollToBottom` actions, the wire round-trip, the wheel
//! handling over the terminal pane) lives in the hosts; this extension is
//! only the key policy.

use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{
    ClientSnapshot, Extension, ExtensionHost, ExtensionManifest, ModeOutcome, ModeSpec, ModeState,
    UiAction,
};

pub const SCROLL_MODE: &str = "scroll";

pub struct ScrollModeExtension;

impl Extension for ScrollModeExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.scroll-mode".into(),
            name: "scroll mode".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "keyboard scrollback navigation (j/k, u/d, g/G)".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.register_mode(ModeSpec {
            name: SCROLL_MODE.into(),
            init_state: Arc::new(|| Box::new(()) as ModeState),
            on_key: Arc::new(handle_key),
            render: None,
        })
    }
}

fn scroll(delta: i32) -> ModeOutcome {
    ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta }])
}

fn handle_key(_state: &mut ModeState, bytes: &[u8], snapshot: &ClientSnapshot) -> ModeOutcome {
    let half_page = i32::from(snapshot.grid_rows / 2).max(1);
    let full_page = i32::from(snapshot.grid_rows).max(1);
    match bytes {
        b"k" | b"\x1b[A" | b"\x1bOA" => scroll(1),
        b"j" | b"\x1b[B" | b"\x1bOB" => scroll(-1),
        // u/d: half page; PageUp/PageDown (CSI 5~/6~): full page.
        b"u" | b"\x15" => scroll(half_page),
        b"d" | b"\x04" => scroll(-half_page),
        b"\x1b[5~" => scroll(full_page),
        b"\x1b[6~" => scroll(-full_page),
        // g: jump to the top of history (the server clamps).
        b"g" => scroll(i32::MAX),
        // G: back to the live screen, stay in the mode.
        b"G" => ModeOutcome::ContinueWith(vec![UiAction::ScrollToBottom]),
        // Wheel over the terminal while the mode is active: raw SGR reports
        // reach the mode before the host's mouse routing.
        _ if is_wheel_report(bytes, true) => scroll(3),
        _ if is_wheel_report(bytes, false) => scroll(-3),
        b"q" | b"\x1b" | b"\r" | b"\n" => ModeOutcome::ExitWith(vec![UiAction::ScrollToBottom]),
        _ => ModeOutcome::Continue,
    }
}

/// Matches an SGR wheel report (`ESC [ < 64;...M` up / `65;...M` down)
/// without pulling in the host's mouse parser.
fn is_wheel_report(bytes: &[u8], up: bool) -> bool {
    let button = if up { b"64;" } else { b"65;" };
    bytes
        .strip_prefix(b"\x1b[<")
        .is_some_and(|rest| rest.starts_with(button) && rest.ends_with(b"M"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_ext::{RuntimeBuilder, ThemePalette};

    fn runtime() -> ekko_ext::AppRuntime {
        RuntimeBuilder::new()
            .register_extension(ScrollModeExtension)
            .build()
            .unwrap()
    }

    fn snapshot() -> ClientSnapshot {
        ClientSnapshot {
            panes: vec![],
            focused_pane: None,
            session_name: "s".into(),
            mode: SCROLL_MODE.into(),
            cols: 80,
            rows: 24,
            grid_cols: 80,
            grid_rows: 22,
            scrollback: 0,
            projects: vec![],
            status_note: None,
            keybindings: vec![],
            now_ms: 0,
            hidden_surfaces: Vec::new(),
            theme: ThemePalette::fallback(),
        }
    }

    fn key(bytes: &[u8]) -> ModeOutcome {
        let rt = runtime();
        let spec = rt.mode(SCROLL_MODE).unwrap();
        let mut state = (spec.init_state)();
        (spec.on_key)(&mut state, bytes, &snapshot())
    }

    #[test]
    fn line_and_page_scrolls() {
        assert_eq!(
            key(b"k"),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: 1 }])
        );
        assert_eq!(
            key(b"j"),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: -1 }])
        );
        assert_eq!(
            key(b"u"),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: 11 }])
        );
        assert_eq!(
            key(b"\x1b[6~"),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: -22 }])
        );
    }

    #[test]
    fn wheel_reports_scroll() {
        assert_eq!(
            key(b"\x1b[<64;10;5M"),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: 3 }])
        );
        assert_eq!(
            key(b"\x1b[<65;10;5M"),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: -3 }])
        );
    }

    #[test]
    fn exit_resets_to_live() {
        for bytes in [b"q".as_slice(), b"\x1b", b"\r"] {
            assert_eq!(
                key(bytes),
                ModeOutcome::ExitWith(vec![UiAction::ScrollToBottom]),
                "bytes {bytes:?}"
            );
        }
    }

    #[test]
    fn unknown_keys_are_swallowed() {
        assert_eq!(key(b"x"), ModeOutcome::Continue);
    }
}
