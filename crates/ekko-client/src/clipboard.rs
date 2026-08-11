//! Clipboard helpers: write to the host terminal via OSC 52, or to the system
//! clipboard directly via a subprocess (wl-copy / xclip / xsel / pbcopy) with
//! OSC 52 as fallback.  The subprocess approach matches zellij's
//! ClipboardProvider::Command pattern (ref/zellij/zellij-server/src/tab/
//! clipboard.rs), reliably reaching the Wayland / X11 / macOS clipboard
//! without depending on terminal emulator OSC 52 support.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Copy text to the system clipboard.
///
/// Tries an external clipboard program first (wl-copy, xclip, xsel, pbcopy),
/// detected once on the first call, then falls back to OSC 52 if nothing
/// suitable is available on PATH.  This mirrors zellij's
/// ClipboardProvider::Command model so users on Wayland, X11, or macOS get
/// reliable clipboard writes without depending on terminal-emulator OSC 52
/// support.
pub fn selection_to_clipboard(text: &str) {
    static CMD: OnceLock<Option<ClipboardCmd>> = OnceLock::new();
    let cmd = CMD.get_or_init(discover_clipboard_cmd);

    if let Some(cmd) = cmd
        && cmd.set(text)
    {
        return;
    }
    // Command failed (e.g. binary disappeared between detection and use)
    // — fall through to OSC 52.

    // OSC 52 fallback
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(&osc52_set_clipboard(text.as_bytes()));
}

struct ClipboardCmd {
    command: String,
    args: Vec<String>,
}

impl ClipboardCmd {
    /// Pipe text to the clipboard command's stdin, reap in background
    /// with a 1 s timeout.  Returns true if the process was launched.
    fn set(&self, text: &str) -> bool {
        let mut child = match Command::new(&self.command)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return false,
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
            // Drop closes stdin so the child know to flush and exit.
        }

        // Background reap with 1 s timeout (zellij-style).
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) => {
                        if start.elapsed() > std::time::Duration::from_secs(1) {
                            let _ = child.kill();
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(_) => return,
                }
            }
        });

        true
    }
}

const CLIPBOARD_CANDIDATES: &[(&str, &[&str])] = &[
    ("wl-copy", &[] as &[&str]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["-ib", "--clipboard"]),
    ("pbcopy", &[] as &[&str]),
];

fn discover_clipboard_cmd() -> Option<ClipboardCmd> {
    for (name, args) in CLIPBOARD_CANDIDATES {
        if let Some(path) = find_on_path(name)
            && path.is_file()
        {
            return Some(ClipboardCmd {
                command: path.to_string_lossy().into_owned(),
                args: args.iter().copied().map(String::from).collect(),
            });
        }
    }
    None
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let full = dir.join(name);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

/// Build an OSC 52 sequence setting the system clipboard to `data`.
pub fn osc52_set_clipboard(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 4 / 3 + 16);
    out.extend_from_slice(b"\x1b]52;c;");
    out.extend_from_slice(base64_encode(data).as_bytes());
    out.extend_from_slice(b"\x1b\\");
    out
}

/// Build an OSC 52 sequence from data that is already base64-encoded (as
/// delivered by the server's vt100 callback).
pub fn osc52_set_clipboard_base64(encoded: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(encoded.len() + 16);
    out.extend_from_slice(b"\x1b]52;c;");
    out.extend_from_slice(encoded);
    out.extend_from_slice(b"\x1b\\");
    out
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_ALPHABET[(triple >> 18) as usize & 0x3f] as char);
        out.push(BASE64_ALPHABET[(triple >> 12) as usize & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[(triple >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[triple as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn osc52_wraps_encoded_payload() {
        assert_eq!(osc52_set_clipboard(b"hi"), b"\x1b]52;c;aGk=\x1b\\".to_vec());
        assert_eq!(
            osc52_set_clipboard_base64(b"aGk="),
            b"\x1b]52;c;aGk=\x1b\\".to_vec()
        );
    }
}
