//! ekko-keycast: a keystroke display for screencasts and pairing (WS8).
//!
//! This is the "one genuinely new extension built purely against the public
//! API" from `DESIGN.md`: it lives outside `ekko-builtins`, depends only on
//! `ekko-ext`, and exercises the surface, command, event-subscription, and
//! dynamic-visibility parts of the API the way a third-party extension
//! would. Toggled with `:keycast`; while enabled it docks a one-row surface
//! above the statusbar showing recent keystrokes, fading them out as they
//! age.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use ekko_ext::{
    ClientSnapshot, CommandInvocation, CommandOutput, CommandSpec, DockEdge, DrawContext,
    EventHandlerRegistration, EventKind, EventPayload, Extension, ExtensionHost, ExtensionManifest,
    NoteKind, Rect, SurfaceSize, SurfaceSpec, fade_toward,
};

/// Keys older than this are dropped; the fade runs across the full window.
const KEY_TTL_MS: u64 = 3_000;
/// Bound on remembered keys, well past what fits in one row.
const MAX_KEYS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Keystroke {
    label: String,
    at_ms: u64,
}

#[derive(Default)]
struct KeycastState {
    enabled: bool,
    keys: VecDeque<Keystroke>,
}

impl KeycastState {
    fn push(&mut self, labels: Vec<String>, now_ms: u64) {
        for label in labels {
            self.keys.push_back(Keystroke {
                label,
                at_ms: now_ms,
            });
        }
        self.expire(now_ms);
        while self.keys.len() > MAX_KEYS {
            self.keys.pop_front();
        }
    }

    fn expire(&mut self, now_ms: u64) {
        while let Some(front) = self.keys.front() {
            if now_ms.saturating_sub(front.at_ms) > KEY_TTL_MS {
                self.keys.pop_front();
            } else {
                break;
            }
        }
    }
}

pub struct KeycastExtension;

impl Extension for KeycastExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-keycast".into(),
            name: "keycast".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "keystroke display for screencasts (`:keycast`)".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        let state = Arc::new(Mutex::new(KeycastState::default()));

        let observer = state.clone();
        host.subscribe(EventHandlerRegistration {
            event: EventKind::KeyInput,
            label: "keycast".into(),
            handler: Arc::new(move |event| {
                if let EventPayload::KeyInput { bytes } = &event.payload {
                    let labels = decode_keys(bytes);
                    if !labels.is_empty() {
                        let mut state = observer.lock().unwrap();
                        if state.enabled {
                            state.push(labels, wall_clock_ms());
                        }
                    }
                }
                // Pure observer: never consumes or rewrites the input.
                Ok(None)
            }),
        })?;

        let toggler = state.clone();
        host.register_command(CommandSpec {
            name: "keycast".into(),
            aliases: vec!["kc".into()],
            description: "toggle the keystroke display row".into(),
            args_hint: String::new(),
            handler: Arc::new(move |_: CommandInvocation| {
                let mut state = toggler.lock().unwrap();
                state.enabled = !state.enabled;
                state.keys.clear();
                let note = if state.enabled {
                    "keycast on"
                } else {
                    "keycast off"
                };
                Ok(CommandOutput::note(note, NoteKind::Ok))
            }),
        })?;

        let visible = state.clone();
        let ticker = state.clone();
        let drawer = state;
        host.register_surface(SurfaceSpec {
            name: "keycast".into(),
            dock: DockEdge::Bottom,
            // Statusbar is priority 0 on the same edge; stack above it.
            priority: 1,
            size: SurfaceSize::Fixed(1),
            hide_below: None,
            visible: Some(Arc::new(move |_: &ClientSnapshot| {
                visible.lock().unwrap().enabled
            })),
            draw: Arc::new(move |ctx, snapshot| {
                let mut state = drawer.lock().unwrap();
                state.expire(snapshot.now_ms);
                draw_keycast(ctx, snapshot, &state.keys);
            }),
            on_mouse: None,
            // Keep redrawing while a key is still fading out.
            wants_tick: Some(Arc::new(move |_: &ClientSnapshot| {
                let state = ticker.lock().unwrap();
                state.enabled && !state.keys.is_empty()
            })),
        })
    }
}

fn wall_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn draw_keycast(ctx: &mut dyn DrawContext, snapshot: &ClientSnapshot, keys: &VecDeque<Keystroke>) {
    let (cols, rows) = ctx.size();
    if cols <= 0 || rows <= 0 {
        return;
    }
    let theme = &snapshot.theme;
    ctx.fill_rect(Rect::new(0, 0, cols, rows), theme.muted, theme.surface);

    let tag = " keycast ";
    ctx.put_text(0, 0, tag.len() as i32, theme.muted, theme.surface, tag);

    // Newest key sits at the right edge; older keys trail left and fade
    // toward the surface color as they age out.
    let mut col = cols - 1;
    for key in keys.iter().rev() {
        let cell = format!(" {} ", key.label);
        let width = cell.chars().count() as i32;
        col -= width;
        if col < tag.len() as i32 {
            break;
        }
        let age = snapshot.now_ms.saturating_sub(key.at_ms).min(KEY_TTL_MS);
        let mix = ((age * 255) / KEY_TTL_MS.max(1)) as u8;
        let fg = fade_toward(theme.text, theme.surface, mix);
        ctx.put_text(col, 0, width, fg, theme.surface, &cell);
    }
}

/// Decode a raw stdin chunk into human-readable key labels. One chunk can
/// carry several keys (escape sequences, fast typing); mouse reports are
/// skipped entirely.
fn decode_keys(bytes: &[u8]) -> Vec<String> {
    let mut labels = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let (label, consumed) = decode_one(rest);
        if let Some(label) = label {
            labels.push(label);
        }
        rest = &rest[consumed..];
    }
    labels
}

/// Decode the first key in `bytes`, returning its label (or `None` for
/// input that isn't a keystroke, e.g. mouse reports) and how many bytes it
/// consumed. Always consumes at least one byte.
fn decode_one(bytes: &[u8]) -> (Option<String>, usize) {
    match bytes {
        // SGR mouse report: swallow through its `M`/`m` terminator.
        [0x1b, b'[', b'<', ..] => {
            let end = bytes
                .iter()
                .position(|&b| b == b'M' || b == b'm')
                .map_or(bytes.len(), |i| i + 1);
            (None, end)
        }
        [0x1b, b'[', tail @ ..] => decode_csi(tail),
        [0x1b, b'O', code, ..] => {
            let label = match code {
                b'P' => Some("F1"),
                b'Q' => Some("F2"),
                b'R' => Some("F3"),
                b'S' => Some("F4"),
                _ => None,
            };
            (label.map(String::from), 3)
        }
        // Alt+key: ESC prefix on a printable byte.
        [0x1b, key, ..] if key.is_ascii_graphic() => (Some(format!("alt+{}", *key as char)), 2),
        [0x1b, ..] => (Some("esc".into()), 1),
        [b'\r', ..] | [b'\n', ..] => (Some("⏎".into()), 1),
        [b'\t', ..] => (Some("⇥".into()), 1),
        [0x7f, ..] | [0x08, ..] => (Some("⌫".into()), 1),
        [b' ', ..] => (Some("␣".into()), 1),
        // Ctrl chords: 0x01..=0x1a (minus the ones handled above).
        [code @ 0x01..=0x1a, ..] => (Some(format!("^{}", (code - 1 + b'A') as char)), 1),
        [code, ..] if *code < 0x20 => (None, 1),
        _ => {
            // First UTF-8 scalar in the chunk, as typed.
            match std::str::from_utf8(bytes) {
                Ok(text) => {
                    let ch = text.chars().next().unwrap();
                    (Some(ch.to_string()), ch.len_utf8())
                }
                Err(err) if err.valid_up_to() > 0 => {
                    let text = std::str::from_utf8(&bytes[..err.valid_up_to()]).unwrap();
                    let ch = text.chars().next().unwrap();
                    (Some(ch.to_string()), ch.len_utf8())
                }
                Err(_) => (None, 1),
            }
        }
    }
}

/// Decode a CSI sequence body (after `ESC [`). Returns the label and the
/// total bytes consumed including the two-byte prefix.
fn decode_csi(tail: &[u8]) -> (Option<String>, usize) {
    // Find the final byte (0x40..=0x7e ends a CSI sequence).
    let Some(end) = tail.iter().position(|&b| (0x40..=0x7e).contains(&b)) else {
        return (None, tail.len() + 2);
    };
    let (params, final_byte) = (&tail[..end], tail[end]);
    let consumed = end + 3;
    let label = match (params, final_byte) {
        (_, b'A') => Some("↑"),
        (_, b'B') => Some("↓"),
        (_, b'C') => Some("→"),
        (_, b'D') => Some("←"),
        (_, b'H') => Some("home"),
        (_, b'F') => Some("end"),
        (b"2", b'~') => Some("ins"),
        (b"3", b'~') => Some("del"),
        (b"5", b'~') => Some("pgup"),
        (b"6", b'~') => Some("pgdn"),
        (b"15", b'~') => Some("F5"),
        (b"17", b'~') => Some("F6"),
        (b"18", b'~') => Some("F7"),
        (b"19", b'~') => Some("F8"),
        (b"20", b'~') => Some("F9"),
        (b"21", b'~') => Some("F10"),
        (b"23", b'~') => Some("F11"),
        (b"24", b'~') => Some("F12"),
        _ => None,
    };
    (label.map(String::from), consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(bytes: &[u8]) -> Vec<String> {
        decode_keys(bytes)
    }

    #[test]
    fn decodes_printable_and_control_keys() {
        assert_eq!(labels(b"a"), vec!["a"]);
        assert_eq!(labels(b"A"), vec!["A"]);
        assert_eq!(labels(b" "), vec!["␣"]);
        assert_eq!(labels(b"\r"), vec!["⏎"]);
        assert_eq!(labels(b"\t"), vec!["⇥"]);
        assert_eq!(labels(&[0x7f]), vec!["⌫"]);
        assert_eq!(labels(&[0x11]), vec!["^Q"]);
        assert_eq!(labels(&[0x01]), vec!["^A"]);
    }

    #[test]
    fn decodes_escape_sequences() {
        assert_eq!(labels(b"\x1b"), vec!["esc"]);
        assert_eq!(labels(b"\x1b[A"), vec!["↑"]);
        assert_eq!(labels(b"\x1b[3~"), vec!["del"]);
        assert_eq!(labels(b"\x1b[5~"), vec!["pgup"]);
        assert_eq!(labels(b"\x1bOP"), vec!["F1"]);
        assert_eq!(labels(b"\x1bx"), vec!["alt+x"]);
    }

    #[test]
    fn decodes_multi_key_chunks_and_utf8() {
        assert_eq!(labels(b"ab\r"), vec!["a", "b", "⏎"]);
        assert_eq!(labels("é".as_bytes()), vec!["é"]);
        assert_eq!(labels("λx".as_bytes()), vec!["λ", "x"]);
    }

    #[test]
    fn skips_mouse_reports() {
        assert!(labels(b"\x1b[<0;10;5M").is_empty());
        assert_eq!(labels(b"\x1b[<0;10;5Ma"), vec!["a"]);
    }

    #[test]
    fn state_expires_and_bounds_keys() {
        let mut state = KeycastState {
            enabled: true,
            keys: VecDeque::new(),
        };
        state.push(vec!["a".into()], 0);
        state.push(vec!["b".into()], KEY_TTL_MS + 1);
        assert_eq!(state.keys.len(), 1);
        assert_eq!(state.keys[0].label, "b");

        let many: Vec<String> = (0..(MAX_KEYS + 10)).map(|i| i.to_string()).collect();
        state.push(many, KEY_TTL_MS + 2);
        assert_eq!(state.keys.len(), MAX_KEYS);
    }

    #[test]
    fn registers_against_the_public_api() {
        let runtime = ekko_ext::RuntimeBuilder::new()
            .register_extension(KeycastExtension)
            .build()
            .unwrap();
        assert!(runtime.surface("keycast").is_some());
        assert!(matches!(
            runtime.invoke_command(":keycast"),
            ekko_ext::CommandDispatch::Invoked(_)
        ));
        assert!(runtime.has_subscribers(EventKind::KeyInput));
    }
}
