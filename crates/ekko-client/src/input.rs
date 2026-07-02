//! Raw stdin byte mechanism: key tokenization, bracketed-paste extraction,
//! and SGR mouse parsing. Pure/stateless where possible so it's unit
//! testable without a tty. Keybinding *matching* lives in the extension
//! registry (`ekko_ext::AppRuntime::match_keybinding`); this module owns no
//! key policy.

pub const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
pub const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

// ---------------------------------------------------------------------------
// Key tokenization
// ---------------------------------------------------------------------------

/// Split a raw stdin read into discrete input tokens: each escape sequence
/// (CSI, SS3, or alt+key ESC pair) and each C0 control byte is its own
/// token, while maximal runs of printable bytes stay together.
///
/// A single `read()` can batch several keypresses (autorepeat, fast typing,
/// a burst of mouse reports, paste markers adjacent to their payload), so
/// chord matching against the whole buffer would fail exactly when the user
/// types fast. Tokens are matched one at a time instead.
///
/// A truncated escape sequence at the end of the buffer is emitted as-is: a
/// human keypress never splits across reads, and for machine-rate input the
/// leftover bytes fall through to the PTY unchanged, matching the pre-token
/// behavior.
pub fn split_key_tokens(bytes: &[u8]) -> Vec<&[u8]> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let len = match bytes[i] {
            0x1b => escape_sequence_len(&bytes[i..]),
            // Each control byte is one keypress (ctrl+X, enter, tab, ...).
            b if is_control_byte(b) => 1,
            _ => bytes[i..]
                .iter()
                .position(|&b| b == 0x1b || is_control_byte(b))
                .unwrap_or(bytes.len() - i),
        };
        tokens.push(&bytes[i..i + len]);
        i += len;
    }
    tokens
}

fn is_control_byte(b: u8) -> bool {
    b < 0x20 || b == 0x7f
}

/// Length of the escape sequence at the start of `bytes` (`bytes[0]` is ESC).
fn escape_sequence_len(bytes: &[u8]) -> usize {
    match bytes.get(1) {
        // CSI: ESC [ params/intermediates, terminated by a byte in 0x40-0x7e.
        Some(b'[') => {
            for (offset, &b) in bytes.iter().enumerate().skip(2) {
                if (0x40..=0x7e).contains(&b) {
                    return offset + 1;
                }
            }
            bytes.len()
        }
        // SS3: ESC O <final>.
        Some(b'O') => bytes.len().min(3),
        // alt+key: ESC followed by one byte (or a lone trailing ESC).
        Some(_) => 2,
        None => 1,
    }
}

// ---------------------------------------------------------------------------
// Bracketed paste
// ---------------------------------------------------------------------------

/// Accumulates bytes between `ESC[200~` and `ESC[201~` markers.
#[derive(Default, Debug)]
pub struct PasteAccumulator {
    buf: Option<Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PasteFeed {
    /// `bytes` were not paste framing; caller should handle them normally.
    NotPaste,
    /// Paste start/continuation consumed; nothing to forward yet.
    InProgress,
    /// Paste end marker seen; here is the accumulated inner payload.
    Complete(Vec<u8>),
}

impl PasteAccumulator {
    pub fn feed(&mut self, bytes: &[u8]) -> PasteFeed {
        if self.buf.is_some() {
            if bytes == BRACKETED_PASTE_END {
                let paste = self.buf.take().unwrap_or_default();
                return PasteFeed::Complete(paste);
            }
            self.buf
                .as_mut()
                .expect("paste buffer exists")
                .extend_from_slice(bytes);
            return PasteFeed::InProgress;
        }

        if bytes == BRACKETED_PASTE_START {
            self.buf = Some(Vec::new());
            return PasteFeed::InProgress;
        }

        PasteFeed::NotPaste
    }

    #[cfg(test)]
    pub fn in_progress(&self) -> bool {
        self.buf.is_some()
    }
}

// ---------------------------------------------------------------------------
// SGR mouse
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    LeftPress,
    /// Motion with the left button held.
    LeftDrag,
    LeftRelease,
    WheelUp,
    WheelDown,
    Other,
}

impl From<MouseKind> for ekko_ext::MouseKind {
    fn from(kind: MouseKind) -> Self {
        match kind {
            MouseKind::LeftPress => Self::LeftPress,
            MouseKind::LeftDrag => Self::LeftDrag,
            MouseKind::LeftRelease => Self::LeftRelease,
            MouseKind::WheelUp => Self::WheelUp,
            MouseKind::WheelDown => Self::WheelDown,
            MouseKind::Other => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    /// 0-based cell column.
    pub col: i32,
    /// 0-based cell row.
    pub row: i32,
    /// The raw SGR button code (buttons + modifier/motion/wheel bits), kept
    /// so the event can be re-encoded for a mouse-aware child program.
    pub raw_button: u16,
    /// `true` for press/motion reports, `false` for release reports.
    pub pressed: bool,
}

const MOTION_FLAG: i64 = 32;
const WHEEL_FLAG: i64 = 64;

/// Parse a single SGR mouse report: `ESC [ < Cb ; Cx ; Cy (M|m)`.
pub fn parse_sgr_mouse(bytes: &[u8]) -> Option<MouseEvent> {
    let rest = bytes.strip_prefix(b"\x1b[<")?;
    let (&last, body) = rest.split_last()?;
    let pressed = match last {
        b'M' => true,
        b'm' => false,
        _ => return None,
    };
    let body = std::str::from_utf8(body).ok()?;
    let mut parts = body.split(';');
    let cb: i64 = parts.next()?.parse().ok()?;
    let cx: i64 = parts.next()?.parse().ok()?;
    let cy: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let kind = if cb & WHEEL_FLAG != 0 {
        if cb & 0x1 == 0 {
            MouseKind::WheelUp
        } else {
            MouseKind::WheelDown
        }
    } else if cb & MOTION_FLAG != 0 {
        if cb & 0x3 == 0 {
            MouseKind::LeftDrag
        } else {
            MouseKind::Other
        }
    } else {
        let button = cb & 0x3;
        match (button, pressed) {
            (0, true) => MouseKind::LeftPress,
            (0, false) => MouseKind::LeftRelease,
            _ => MouseKind::Other,
        }
    };

    Some(MouseEvent {
        kind,
        col: (cx - 1).max(0) as i32,
        row: (cy - 1).max(0) as i32,
        raw_button: cb.clamp(0, i64::from(u16::MAX)) as u16,
        pressed,
    })
}

/// Re-encode a mouse event for the child program, honoring the tracking
/// mode and encoding it requested. `col`/`row` are 0-based, local to the
/// terminal pane. Returns `None` when the child's mode excludes this event
/// (or the coordinates don't fit the legacy encoding).
pub fn encode_mouse_for_child(
    event: &MouseEvent,
    modes: &ekko_proto::TermModes,
    col: i32,
    row: i32,
) -> Option<Vec<u8>> {
    use ekko_proto::{MouseEncoding, MouseMode};

    let is_wheel = matches!(event.kind, MouseKind::WheelUp | MouseKind::WheelDown);
    let is_motion = i64::from(event.raw_button) & MOTION_FLAG != 0 && !is_wheel;
    match modes.mouse_mode {
        MouseMode::None => return None,
        MouseMode::Press => {
            if !event.pressed || is_motion {
                return None;
            }
        }
        MouseMode::PressRelease => {
            if is_motion {
                return None;
            }
        }
        MouseMode::ButtonMotion | MouseMode::AnyMotion => {}
    }

    let x = col + 1;
    let y = row + 1;
    match modes.mouse_encoding {
        MouseEncoding::Sgr => {
            let terminator = if event.pressed { 'M' } else { 'm' };
            Some(format!("\x1b[<{};{};{}{}", event.raw_button, x, y, terminator).into_bytes())
        }
        MouseEncoding::Default | MouseEncoding::Utf8 => {
            // Legacy X10 bytes: release is reported as button 3, and
            // coordinates beyond 223 don't fit a byte.
            if x > 223 || y > 223 {
                return None;
            }
            let mut cb = event.raw_button;
            if !event.pressed && !is_wheel {
                cb = (cb & !0x3) | 3;
            }
            let cb = u8::try_from(32 + cb).ok()?;
            Some(vec![0x1b, b'[', b'M', cb, (32 + x) as u8, (32 + y) as u8])
        }
    }
}

/// Focus-in/focus-out reports from the host terminal (mode 1004).
pub fn is_focus_event(bytes: &[u8]) -> bool {
    bytes == b"\x1b[I" || bytes == b"\x1b[O"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_split_batched_control_bytes() {
        // Two autorepeated ctrl+q presses in one read match separately.
        assert_eq!(
            split_key_tokens(b"\x11\x11"),
            vec![b"\x11" as &[u8], b"\x11"]
        );
        // Same for escape-prefixed keys (alt+j alt+j).
        assert_eq!(
            split_key_tokens(b"\x1bj\x1bj"),
            vec![b"\x1bj" as &[u8], b"\x1bj"]
        );
        // Text followed by enter splits at the control byte.
        assert_eq!(split_key_tokens(b"ls\r"), vec![b"ls" as &[u8], b"\r"]);
    }

    #[test]
    fn tokens_split_text_around_escape_sequences() {
        assert_eq!(
            split_key_tokens(b"ab\x1b[Acd"),
            vec![b"ab" as &[u8], b"\x1b[A", b"cd"]
        );
    }

    #[test]
    fn tokens_split_batched_mouse_reports() {
        assert_eq!(
            split_key_tokens(b"\x1b[<64;3;3M\x1b[<64;3;3M"),
            vec![b"\x1b[<64;3;3M" as &[u8], b"\x1b[<64;3;3M"]
        );
    }

    #[test]
    fn tokens_isolate_paste_markers_from_payload() {
        assert_eq!(
            split_key_tokens(b"\x1b[200~hello\x1b[201~"),
            vec![b"\x1b[200~" as &[u8], b"hello", b"\x1b[201~"]
        );
    }

    #[test]
    fn lone_escape_is_a_single_token() {
        assert_eq!(split_key_tokens(b"\x1b"), vec![b"\x1b" as &[u8]]);
    }

    #[test]
    fn truncated_csi_is_emitted_as_is() {
        assert_eq!(split_key_tokens(b"\x1b[1;3"), vec![b"\x1b[1;3" as &[u8]]);
    }

    #[test]
    fn ss3_arrow_is_one_token() {
        assert_eq!(split_key_tokens(b"\x1bOAx"), vec![b"\x1bOA" as &[u8], b"x"]);
    }

    #[test]
    fn paste_accumulator_extracts_inner_bytes() {
        let mut acc = PasteAccumulator::default();
        assert_eq!(acc.feed(BRACKETED_PASTE_START), PasteFeed::InProgress);
        assert!(acc.in_progress());
        assert_eq!(acc.feed(b"hello "), PasteFeed::InProgress);
        assert_eq!(acc.feed(b"world"), PasteFeed::InProgress);
        assert_eq!(
            acc.feed(BRACKETED_PASTE_END),
            PasteFeed::Complete(b"hello world".to_vec())
        );
        assert!(!acc.in_progress());
    }

    #[test]
    fn paste_accumulator_ignores_non_paste_bytes() {
        let mut acc = PasteAccumulator::default();
        assert_eq!(acc.feed(b"regular input"), PasteFeed::NotPaste);
    }

    #[test]
    fn parses_left_click() {
        let ev = parse_sgr_mouse(b"\x1b[<0;5;10M").unwrap();
        assert_eq!(ev.kind, MouseKind::LeftPress);
        assert_eq!(ev.col, 4);
        assert_eq!(ev.row, 9);
    }

    #[test]
    fn parses_left_release() {
        let ev = parse_sgr_mouse(b"\x1b[<0;1;1m").unwrap();
        assert_eq!(ev.kind, MouseKind::LeftRelease);
        assert_eq!(ev.col, 0);
        assert_eq!(ev.row, 0);
    }

    #[test]
    fn parses_wheel_events() {
        let up = parse_sgr_mouse(b"\x1b[<64;3;3M").unwrap();
        assert_eq!(up.kind, MouseKind::WheelUp);
        let down = parse_sgr_mouse(b"\x1b[<65;3;3M").unwrap();
        assert_eq!(down.kind, MouseKind::WheelDown);
    }

    #[test]
    fn rejects_malformed_sequences() {
        assert_eq!(parse_sgr_mouse(b"\x1b[<0;1M"), None);
        assert_eq!(parse_sgr_mouse(b"not mouse"), None);
    }

    #[test]
    fn parses_left_drag() {
        let ev = parse_sgr_mouse(b"\x1b[<32;5;10M").unwrap();
        assert_eq!(ev.kind, MouseKind::LeftDrag);
        assert_eq!(ev.raw_button, 32);
        // Motion with no button held (any-motion mode) is not a drag.
        let ev = parse_sgr_mouse(b"\x1b[<35;5;10M").unwrap();
        assert_eq!(ev.kind, MouseKind::Other);
    }

    fn modes(mode: ekko_proto::MouseMode, encoding: ekko_proto::MouseEncoding) -> ekko_proto::TermModes {
        ekko_proto::TermModes {
            mouse_mode: mode,
            mouse_encoding: encoding,
            ..Default::default()
        }
    }

    #[test]
    fn encode_respects_child_tracking_mode() {
        use ekko_proto::{MouseEncoding, MouseMode};
        let press = parse_sgr_mouse(b"\x1b[<0;5;10M").unwrap();
        let release = parse_sgr_mouse(b"\x1b[<0;5;10m").unwrap();
        let drag = parse_sgr_mouse(b"\x1b[<32;5;10M").unwrap();

        let none = modes(MouseMode::None, MouseEncoding::Sgr);
        assert_eq!(encode_mouse_for_child(&press, &none, 4, 9), None);

        let press_only = modes(MouseMode::Press, MouseEncoding::Sgr);
        assert!(encode_mouse_for_child(&press, &press_only, 4, 9).is_some());
        assert_eq!(encode_mouse_for_child(&release, &press_only, 4, 9), None);
        assert_eq!(encode_mouse_for_child(&drag, &press_only, 4, 9), None);

        let button_motion = modes(MouseMode::ButtonMotion, MouseEncoding::Sgr);
        assert!(encode_mouse_for_child(&drag, &button_motion, 4, 9).is_some());
    }

    #[test]
    fn encode_sgr_rewrites_local_coordinates() {
        use ekko_proto::{MouseEncoding, MouseMode};
        let press = parse_sgr_mouse(b"\x1b[<0;40;10M").unwrap();
        let m = modes(MouseMode::PressRelease, MouseEncoding::Sgr);
        // Sidebar occupies 30 cols: pane-local col 9 (0-based) -> 10.
        assert_eq!(
            encode_mouse_for_child(&press, &m, 9, 9).unwrap(),
            b"\x1b[<0;10;10M".to_vec()
        );
    }

    #[test]
    fn encode_legacy_bytes_and_release_button() {
        use ekko_proto::{MouseEncoding, MouseMode};
        let release = parse_sgr_mouse(b"\x1b[<0;5;6m").unwrap();
        let m = modes(MouseMode::PressRelease, MouseEncoding::Default);
        assert_eq!(
            encode_mouse_for_child(&release, &m, 4, 5).unwrap(),
            vec![0x1b, b'[', b'M', 32 + 3, 32 + 5, 32 + 6]
        );
        // Coordinates beyond the legacy range are unencodable.
        assert_eq!(encode_mouse_for_child(&release, &m, 250, 5), None);
    }

    #[test]
    fn focus_events_are_recognized() {
        assert!(is_focus_event(b"\x1b[I"));
        assert!(is_focus_event(b"\x1b[O"));
        assert!(!is_focus_event(b"\x1b[A"));
    }
}
