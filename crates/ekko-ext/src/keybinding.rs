//! Keybinding registration: raw-byte chords resolved against the registry
//! before input falls through to the PTY. The binding *vocabulary* (which
//! chords exist, what they do) is extension policy; matching is host
//! mechanism.

use std::sync::Arc;

use ekko_event::UiAction;

use crate::snapshot::ClientSnapshot;

/// Parse a binding string into every raw byte sequence it can arrive as.
///
/// Vocabulary:
/// - `"ctrl+<letter>"` — the single control byte (`ctrl+a` = 0x01)
/// - `"ctrl+space"` — NUL (0x00)
/// - `"alt+<char>"` — ESC-prefixed printable (`alt+h` = 0x1b 0x68)
/// - `"up" | "down" | "left" | "right"` — both the CSI (`ESC [ A`) and
///   application-cursor SS3 (`ESC O A`) forms
/// - `"alt+<arrow>"` — the modified CSI form (`ESC [ 1 ; 3 A`)
/// - `"<char>"` — a bare printable, case-sensitive (`g` ≠ `G`), plus the
///   `"space"` name. Meant for mode-scoped bindings (a leader map's keys);
///   a bare key bound in normal mode shadows typing that character.
///
/// One binding string can map to several wire encodings (arrows), hence the
/// nested `Vec`. Growing this vocabulary grows every extension's reach at
/// once.
pub fn parse_key_chords(text: &str) -> Option<Vec<Vec<u8>>> {
    // Bare printables are matched before lowercasing so case survives.
    if let Some(ch) = single_char(text.trim())
        && ch.is_ascii_graphic()
    {
        return Some(vec![vec![ch as u8]]);
    }

    let text = text.trim().to_ascii_lowercase();

    if text == "space" {
        return Some(vec![vec![0x20]]);
    }

    if let Some(rest) = text.strip_prefix("alt+") {
        if let Some(arrow) = arrow_final_byte(rest) {
            return Some(vec![vec![0x1b, b'[', b'1', b';', b'3', arrow]]);
        }
        let ch = single_char(rest)?;
        if !ch.is_ascii_graphic() {
            return None;
        }
        return Some(vec![vec![0x1b, ch as u8]]);
    }

    if let Some(arrow) = arrow_final_byte(&text) {
        return Some(vec![vec![0x1b, b'[', arrow], vec![0x1b, b'O', arrow]]);
    }

    if let Some(rest) = text.strip_prefix("ctrl+") {
        if rest == "space" {
            return Some(vec![vec![0x00]]);
        }
        let letter = single_char(rest)?;
        if !letter.is_ascii_alphabetic() {
            return None;
        }
        // ctrl+a is 0x01, ctrl+z is 0x1a.
        return Some(vec![vec![(letter as u8) - b'a' + 1]]);
    }

    None
}

/// The primary byte sequence for a binding string; see [`parse_key_chords`].
pub fn parse_key_binding(text: &str) -> Option<Vec<u8>> {
    parse_key_chords(text).and_then(|chords| chords.into_iter().next())
}

/// Resolve config binding strings into the flattened chord set plus the
/// joined display text (`"ctrl+b / alt+b"`), ready for a `KeybindingSpec`.
/// Unparseable strings are skipped; returns `None` when nothing parses
/// (callers treat the binding as absent). Callers that want to *reject*
/// bad input instead should use [`parse_key_chords`] per string.
pub fn resolve_chords(strings: &[String]) -> Option<(Vec<Vec<u8>>, String)> {
    let chords: Vec<Vec<u8>> = strings
        .iter()
        .filter_map(|s| parse_key_chords(s))
        .flatten()
        .collect();
    if chords.is_empty() {
        return None;
    }
    Some((chords, strings.join(" / ")))
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(ch)
}

fn arrow_final_byte(name: &str) -> Option<u8> {
    match name {
        "up" => Some(b'A'),
        "down" => Some(b'B'),
        "right" => Some(b'C'),
        "left" => Some(b'D'),
        _ => None,
    }
}

pub type KeybindingHandler = Arc<dyn Fn(&ClientSnapshot) -> Vec<UiAction> + Send + Sync>;

#[derive(Clone)]
pub struct KeybindingSpec {
    /// Raw byte chords that trigger this binding (see [`parse_key_chords`]).
    pub chords: Vec<Vec<u8>>,
    /// Human-readable binding text for help output, e.g. `"ctrl+q"`.
    pub chord_text: String,
    /// Mode scope: `None` matches in normal mode only.
    pub mode: Option<String>,
    pub description: String,
    pub handler: KeybindingHandler,
}

/// Listing entry for help/hint output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingInfo {
    pub chord_text: String,
    /// Mode scope: `None` matches in normal mode only.
    pub mode: Option<String>,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ctrl_bindings() {
        assert_eq!(parse_key_binding("ctrl+q"), Some(vec![0x11]));
        assert_eq!(parse_key_binding("ctrl+a"), Some(vec![0x01]));
        assert_eq!(parse_key_binding("ctrl+z"), Some(vec![0x1a]));
        assert_eq!(parse_key_binding("ctrl+1"), None);
        assert_eq!(parse_key_binding("ctrl+ab"), None);
    }

    #[test]
    fn parses_alt_bindings() {
        assert_eq!(parse_key_binding("alt+h"), Some(vec![0x1b, b'h']));
        assert_eq!(parse_key_binding("alt+X"), Some(vec![0x1b, b'x']));
        assert_eq!(parse_key_binding("alt+/"), Some(vec![0x1b, b'/']));
        assert_eq!(parse_key_binding("alt+1"), Some(vec![0x1b, b'1']));
        assert_eq!(parse_key_binding("alt+"), None);
        assert_eq!(parse_key_binding("alt+hh"), None);
    }

    #[test]
    fn parses_arrows_with_both_encodings() {
        assert_eq!(
            parse_key_chords("up"),
            Some(vec![b"\x1b[A".to_vec(), b"\x1bOA".to_vec()])
        );
        assert_eq!(
            parse_key_chords("left"),
            Some(vec![b"\x1b[D".to_vec(), b"\x1bOD".to_vec()])
        );
    }

    #[test]
    fn parses_alt_arrows() {
        assert_eq!(
            parse_key_chords("alt+up"),
            Some(vec![b"\x1b[1;3A".to_vec()])
        );
        assert_eq!(
            parse_key_chords("alt+right"),
            Some(vec![b"\x1b[1;3C".to_vec()])
        );
    }

    #[test]
    fn parses_bare_printables_case_sensitively() {
        assert_eq!(parse_key_binding("g"), Some(vec![b'g']));
        assert_eq!(parse_key_binding("G"), Some(vec![b'G']));
        assert_eq!(parse_key_binding("?"), Some(vec![b'?']));
        assert_eq!(parse_key_binding(" g "), Some(vec![b'g']));
        assert_eq!(parse_key_binding("gg"), None);
    }

    #[test]
    fn parses_space_forms() {
        assert_eq!(parse_key_binding("space"), Some(vec![0x20]));
        assert_eq!(parse_key_binding("ctrl+space"), Some(vec![0x00]));
    }

    #[test]
    fn rejects_unknown_forms() {
        assert_eq!(parse_key_binding("shift+a"), None);
        assert_eq!(parse_key_binding("f1"), None);
        assert_eq!(parse_key_binding(""), None);
    }
}
