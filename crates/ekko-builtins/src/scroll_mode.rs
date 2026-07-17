//! Scroll mode: keyboard navigation through the session's server-side
//! scrollback, registered through the public API. The mechanism (the
//! `Scroll`/`ScrollToBottom`/`SearchScrollback` actions, the wire
//! round-trip, the wheel handling over the terminal pane) lives in the
//! hosts; this extension is only the key policy — j/k/u/d/g/G movement,
//! `/` search with n/N jumps, `e` to open the scrollback in $EDITOR.

use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{
    ClientSnapshot, DrawContext, Extension, ExtensionHost, ExtensionManifest, ModeOutcome,
    ModeSpec, ModeState, UiAction,
};

pub const SCROLL_MODE: &str = "scroll";

/// Per-activation state: the in-progress `/` query, if the user is typing
/// one. Finished searches live in the client's search state (highlighted
/// in the pane), not here.
#[derive(Default)]
struct SearchLine {
    query: Option<String>,
}

pub struct ScrollModeExtension;

impl Extension for ScrollModeExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.scroll-mode".into(),
            name: "scroll mode".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "keyboard scrollback navigation (j/k, u/d, g/G, /, n/N, e)".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.register_mode(ModeSpec {
            name: SCROLL_MODE.into(),
            init_state: Arc::new(|| Box::new(SearchLine::default()) as ModeState),
            on_key: Arc::new(handle_key),
            render: Some(Arc::new(render_search_line)),
        })
    }
}

fn scroll(delta: i32) -> ModeOutcome {
    ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta }])
}

fn handle_key(state: &mut ModeState, bytes: &[u8], snapshot: &ClientSnapshot) -> ModeOutcome {
    let line = state
        .downcast_mut::<SearchLine>()
        .expect("scroll mode state is SearchLine");

    // Typing a `/` query: line-editor keys only; Enter fires the search.
    if let Some(query) = &mut line.query {
        match bytes {
            b"\x1b" => {
                line.query = None;
                return ModeOutcome::Continue;
            }
            b"\r" | b"\n" => {
                let query = std::mem::take(query);
                line.query = None;
                if query.is_empty() {
                    return ModeOutcome::Continue;
                }
                return ModeOutcome::ContinueWith(vec![UiAction::SearchScrollback { query }]);
            }
            b"\x7f" | b"\x08" => {
                query.pop();
                return ModeOutcome::Continue;
            }
            // Ctrl+U: clear the line (command-mode convention).
            b"\x15" => {
                query.clear();
                return ModeOutcome::Continue;
            }
            _ => {
                if let Ok(text) = std::str::from_utf8(bytes)
                    && text.chars().all(|c| !c.is_control())
                {
                    query.push_str(text);
                }
                return ModeOutcome::Continue;
            }
        }
    }

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
        // Search: `/` opens the query line, n/N jump between hits.
        b"/" => {
            line.query = Some(String::new());
            ModeOutcome::Continue
        }
        b"n" => ModeOutcome::ContinueWith(vec![UiAction::SearchMatchJump { forward: true }]),
        b"N" => ModeOutcome::ContinueWith(vec![UiAction::SearchMatchJump { forward: false }]),
        // Edit the scrollback in $EDITOR (client-side terminal takeover).
        b"e" => ModeOutcome::ContinueWith(vec![UiAction::EditScrollback]),
        // Wheel over the terminal while the mode is active: raw SGR reports
        // reach the mode before the host's mouse routing.
        _ if is_wheel_report(bytes, true) => scroll(3),
        _ if is_wheel_report(bytes, false) => scroll(-3),
        b"q" | b"\x1b" | b"\r" | b"\n" => {
            ModeOutcome::ExitWith(vec![UiAction::SearchClear, UiAction::ScrollToBottom])
        }
        _ => ModeOutcome::Continue,
    }
}

/// The `/` prompt on the frame's bottom row while a query is being typed;
/// returns the hardware cursor position just past the query text.
fn render_search_line(
    ctx: &mut dyn DrawContext,
    state: &ModeState,
    snapshot: &ClientSnapshot,
) -> Option<(i32, i32)> {
    let line = state.downcast_ref::<SearchLine>()?;
    let query = line.query.as_ref()?;
    let (cols, rows) = ctx.size();
    let row = rows - 1;
    let text = format!("/{query}");
    ctx.put_text(
        0,
        row,
        cols,
        snapshot.theme.accent,
        snapshot.theme.term_bg,
        &text,
    );
    Some((text.chars().count() as i32, row))
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

    fn state() -> ModeState {
        Box::new(SearchLine::default())
    }

    fn runtime() -> ekko_ext::AppRuntime {
        RuntimeBuilder::new()
            .register_extension(ScrollModeExtension)
            .build()
            .unwrap()
    }

    #[test]
    fn the_mode_is_registered_through_the_public_api() {
        assert!(runtime().mode(SCROLL_MODE).is_some());
    }

    #[test]
    fn movement_keys_scroll_like_the_default_map() {
        let mut state = state();
        assert_eq!(
            handle_key(&mut state, b"k", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: 1 }])
        );
        assert_eq!(
            handle_key(&mut state, b"j", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: -1 }])
        );
        assert_eq!(
            handle_key(&mut state, b"u", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: 11 }])
        );
        assert_eq!(
            handle_key(&mut state, b"d", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: -11 }])
        );
        assert_eq!(
            handle_key(&mut state, b"g", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: i32::MAX }])
        );
        assert_eq!(
            handle_key(&mut state, b"G", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::ScrollToBottom])
        );
        assert_eq!(
            handle_key(&mut state, b"\x1b[<64;10;5M", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: 3 }])
        );
        assert_eq!(
            handle_key(&mut state, b"\x1b[<65;10;5M", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: -3 }])
        );
        assert_eq!(
            handle_key(&mut state, b"\x1b", &snapshot()),
            ModeOutcome::ExitWith(vec![UiAction::SearchClear, UiAction::ScrollToBottom])
        );
    }

    #[test]
    fn slash_opens_a_query_and_enter_fires_the_search() {
        let mut state = state();
        assert_eq!(
            handle_key(&mut state, b"/", &snapshot()),
            ModeOutcome::Continue
        );
        assert_eq!(
            handle_key(&mut state, b"f", &snapshot()),
            ModeOutcome::Continue
        );
        assert_eq!(
            handle_key(&mut state, b"o", &snapshot()),
            ModeOutcome::Continue
        );
        // Backspace edits, then more input.
        assert_eq!(
            handle_key(&mut state, b"\x7f", &snapshot()),
            ModeOutcome::Continue
        );
        assert_eq!(
            handle_key(&mut state, b"o", &snapshot()),
            ModeOutcome::Continue
        );
        assert_eq!(
            handle_key(&mut state, b"\r", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::SearchScrollback { query: "fo".into() }])
        );
        // The line is reset for the next search.
        let line = state.downcast_ref::<SearchLine>().unwrap();
        assert!(line.query.is_none());
    }

    #[test]
    fn escape_cancels_the_query_without_leaving_the_mode() {
        let mut state = state();
        let _ = handle_key(&mut state, b"/", &snapshot());
        let _ = handle_key(&mut state, b"x", &snapshot());
        assert_eq!(
            handle_key(&mut state, b"\x1b", &snapshot()),
            ModeOutcome::Continue
        );
        let line = state.downcast_ref::<SearchLine>().unwrap();
        assert!(line.query.is_none());
    }

    #[test]
    fn match_jumps_and_editor_are_plain_actions() {
        let mut state = state();
        assert_eq!(
            handle_key(&mut state, b"n", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::SearchMatchJump { forward: true }])
        );
        assert_eq!(
            handle_key(&mut state, b"N", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::SearchMatchJump { forward: false }])
        );
        assert_eq!(
            handle_key(&mut state, b"e", &snapshot()),
            ModeOutcome::ContinueWith(vec![UiAction::EditScrollback])
        );
    }
}
