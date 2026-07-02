//! Raw-mode terminal control for the mux client: alt screen, mouse
//! reporting, bracketed paste. Ported from
//! pi-harness's `app/tui/raw.rs` (escape vocabulary) with phi-tui's
//! panic/signal-safe `emergency_restore` pattern, on direct termios instead
//! of an `stty` subprocess.

use std::io::{self, Write};
use std::mem::MaybeUninit;
use std::os::fd::AsRawFd;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub const SYNC_BEGIN: &str = "\x1b[?2026h";
pub const SYNC_END: &str = "\x1b[?2026l";
pub const HIDE_CURSOR: &str = "\x1b[?25l";
pub const SHOW_CURSOR: &str = "\x1b[?25h";

const ENTER_ALT_SCREEN: &str = "\x1b[?1049h";
const EXIT_ALT_SCREEN: &str = "\x1b[?1049l";
const ENABLE_MOUSE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const DISABLE_MOUSE: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";
const ENABLE_BRACKETED_PASTE: &str = "\x1b[?2004h";
const DISABLE_BRACKETED_PASTE: &str = "\x1b[?2004l";
const ENABLE_FOCUS_REPORTING: &str = "\x1b[?1004h";
const DISABLE_FOCUS_REPORTING: &str = "\x1b[?1004l";
const RESET_CURSOR_SHAPE: &str = "\x1b[0 q";

/// Original termios captured when raw mode was first entered, so a panic
/// hook or signal handler can restore the terminal without a guard
/// reference (signals and panics don't run `Drop`).
static SAVED_TERMIOS: Mutex<Option<libc::termios>> = Mutex::new(None);
static RESTORE_ACTIVE: AtomicBool = AtomicBool::new(false);

fn stdin_fd() -> i32 {
    io::stdin().as_raw_fd()
}

fn get_termios() -> io::Result<libc::termios> {
    let mut termios = MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(stdin_fd(), termios.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { termios.assume_init() })
}

fn set_termios(termios: &libc::termios) -> io::Result<()> {
    if unsafe { libc::tcsetattr(stdin_fd(), libc::TCSANOW, termios) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn write_escapes(escapes: &str) {
    let mut out = io::stdout();
    let _ = out.write_all(escapes.as_bytes());
    let _ = out.flush();
}

/// Enters raw mode + alt screen + mouse + bracketed paste. Restores
/// everything on drop.
///
/// Deliberately does NOT enable the kitty keyboard protocol: the client
/// matches chords against legacy single-byte reads and forwards everything
/// else raw to the pane's PTY, so CSI-u encoded ctrl/alt keys would neither
/// match chords nor mean anything to the inner shell.
pub struct RawModeGuard {
    _private: (),
}

impl RawModeGuard {
    pub fn new() -> io::Result<Self> {
        let saved = get_termios()?;
        let mut raw = saved;
        unsafe { libc::cfmakeraw(&mut raw) };
        // Blocking reads: return as soon as 1 byte is available.
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        set_termios(&raw)?;
        *SAVED_TERMIOS.lock().unwrap() = Some(saved);
        RESTORE_ACTIVE.store(true, Ordering::SeqCst);
        write_escapes(&format!(
            "{ENTER_ALT_SCREEN}{ENABLE_MOUSE}{ENABLE_BRACKETED_PASTE}{ENABLE_FOCUS_REPORTING}{HIDE_CURSOR}\x1b[2J\x1b[H"
        ));
        Ok(Self { _private: () })
    }

    /// Run `f` with the terminal restored to cooked mode, re-entering raw
    /// mode afterwards (e.g. handing the tty to an external process).
    pub fn with_suspended<R>(&self, f: impl FnOnce() -> R) -> R {
        restore_terminal();
        let result = f();
        let _ = Self::reenter();
        result
    }

    fn reenter() -> io::Result<()> {
        let saved = get_termios()?;
        let mut raw = saved;
        unsafe { libc::cfmakeraw(&mut raw) };
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        set_termios(&raw)?;
        {
            let mut slot = SAVED_TERMIOS.lock().unwrap();
            if slot.is_none() {
                *slot = Some(saved);
            }
        }
        RESTORE_ACTIVE.store(true, Ordering::SeqCst);
        write_escapes(&format!(
            "{ENTER_ALT_SCREEN}{ENABLE_MOUSE}{ENABLE_BRACKETED_PASTE}{ENABLE_FOCUS_REPORTING}{HIDE_CURSOR}\x1b[2J\x1b[H"
        ));
        Ok(())
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    if !RESTORE_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    write_escapes(&format!(
        "\x1b[0m{SYNC_END}{RESET_CURSOR_SHAPE}{DISABLE_FOCUS_REPORTING}{DISABLE_BRACKETED_PASTE}{DISABLE_MOUSE}{SHOW_CURSOR}{EXIT_ALT_SCREEN}"
    ));
    if let Some(saved) = *SAVED_TERMIOS.lock().unwrap() {
        let _ = set_termios(&saved);
    }
}

/// Restore the terminal from anywhere: panic hooks and signal handlers,
/// where the guard itself is out of reach. Idempotent; a no-op while no
/// guard is active.
pub fn emergency_restore() {
    restore_terminal();
}

/// Current terminal size as (cols, rows), with an env/default fallback for
/// non-tty contexts. Ported from pi-harness `terminal_size`.
pub fn terminal_size() -> (u16, u16) {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let fd = io::stdout().as_raw_fd();
    let ok = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut size) } == 0;
    if ok && size.ws_col > 0 && size.ws_row > 0 {
        return (size.ws_col, size.ws_row);
    }
    let cols = std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(120);
    let rows = std::env::var("LINES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(40);
    (cols, rows)
}

use unicode_width::UnicodeWidthStr;

/// Display width of `s` with ANSI escapes stripped.
pub fn visible_width(s: &str) -> usize {
    strip_ansi(s).width()
}

/// Remove ANSI CSI/OSC escape sequences from `s`.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            // CSI: ESC [ ... final byte in @..~
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: ESC ] ... BEL or ESC \
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {
                chars.next();
            }
        }
    }
    out
}

/// Length in bytes of the escape sequence body starting just after an ESC.
pub fn escape_len(s: &str) -> usize {
    let mut chars = s.char_indices();
    match chars.next() {
        Some((_, '[')) => {
            for (i, c) in chars {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    return i + c.len_utf8();
                }
            }
            s.len()
        }
        Some((_, ']')) => {
            let mut prev_esc = false;
            for (i, c) in chars {
                if c == '\x07' {
                    return i + 1;
                }
                if prev_esc && c == '\\' {
                    return i + 1;
                }
                prev_esc = c == '\x1b';
            }
            s.len()
        }
        Some((_, c)) => c.len_utf8(),
        None => 0,
    }
}
