//! Terminal color probing: query the host terminal's default
//! foreground/background (OSC 10/11) and 16-color palette (OSC 4) so the mux
//! chrome can derive a palette that respects the user's terminal theme rather
//! than assuming a fixed one. Mirrors opencode's `renderer.getPalette`.
//!
//! The probe runs once at startup inside raw mode; it writes the query to
//! stdout, polls stdin until the terminal answers or the deadline elapses,
//! and consumes exactly the OSC response bytes (any other input is left for
//! the event loop). Terminals that don't answer in time yield `None`, and the
//! caller falls back to its built-in palette.

use std::io::Write;
use std::time::{Duration, Instant};

/// A packed sRGB triple (0..255 per channel).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// Terminal-reported colors. `background`/`foreground` come from OSC 10/11;
/// `palette` holds the 16 ANSI entries from OSC 4 (any entry the terminal
/// omits stays `None` and the consumer falls back to the 256-color cube).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalColors {
    pub background: Rgb,
    pub foreground: Rgb,
    pub palette: [Option<Rgb>; 16],
}

/// Query the terminal for its colors within `timeout`. Returns `None` if the
/// terminal isn't a tty or doesn't answer (e.g. piped output, or a terminal
/// without OSC color reporting).
pub fn detect_terminal_colors(timeout: Duration) -> Option<TerminalColors> {
    if !stdout_is_tty() {
        return None;
    }
    let query = build_query();
    write_query(&query).ok()?;

    let deadline = Instant::now() + timeout;
    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0u8; 256];
    loop {
        if Instant::now() >= deadline {
            break;
        }
        match read_available(&mut chunk, deadline) {
            Ok(0) if buf.is_empty() => continue,
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if parse_complete(&buf) {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let parsed = parse_responses(&buf)?;
    Some(normalize(parsed))
}

fn build_query() -> String {
    // ST-terminated (\x1b\\) so we cleanly delimit each request; the terminal's
    // reply may use either BEL or ST, and the parser accepts both.
    let mut q = String::from("\x1b]10;?\x1b\\\x1b]11;?\x1b\\");
    for i in 0..16u8 {
        use std::fmt::Write as _;
        let _ = write!(q, "\x1b]4;{i};?\x1b\\");
    }
    q
}

fn write_query(query: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    out.write_all(query.as_bytes())?;
    out.flush()
}

#[cfg(unix)]
fn stdout_is_tty() -> bool {
    unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 }
}

#[cfg(not(unix))]
fn stdout_is_tty() -> bool {
    false
}

#[cfg(unix)]
fn read_available(chunk: &mut [u8], deadline: Instant) -> std::io::Result<usize> {
    let now = Instant::now();
    if deadline <= now {
        return Ok(0);
    }
    let remaining = (deadline - now).as_millis().min(i32::MAX as u128) as i32;
    let mut pfd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let n = unsafe { libc::poll(&mut pfd, 1, remaining) };
    if n <= 0 || pfd.revents & libc::POLLIN == 0 {
        return Ok(0);
    }
    let n = unsafe {
        libc::read(
            libc::STDIN_FILENO,
            chunk.as_mut_ptr() as *mut _,
            chunk.len(),
        )
    };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(n as usize)
}

#[cfg(not(unix))]
fn read_available(_chunk: &mut [u8], _deadline: Instant) -> std::io::Result<usize> {
    Ok(0)
}

#[derive(Default)]
struct Parsed {
    foreground: Option<Rgb>,
    background: Option<Rgb>,
    palette: [Option<Rgb>; 16],
}

fn parse_complete(buf: &[u8]) -> bool {
    let Some(p) = parse_responses(buf) else {
        return false;
    };
    p.foreground.is_some() && p.background.is_some() && p.palette.iter().all(Option::is_some)
}

fn parse_responses(buf: &[u8]) -> Option<Parsed> {
    let mut p = Parsed::default();
    let mut found_any = false;
    let mut i = 0;
    while i < buf.len() {
        if buf[i] != 0x1b {
            i += 1;
            continue;
        }
        if i + 1 >= buf.len() {
            break;
        }
        if buf[i + 1] != b']' {
            i += 1;
            continue;
        }
        let start = i + 2;
        let Some((payload_len, term_len)) = find_terminator(&buf[start..]) else {
            break;
        };
        apply_payload(&buf[start..start + payload_len], &mut p);
        found_any = true;
        // Guard against a zero-length terminator match stalling the loop.
        let step = 2 + payload_len + term_len;
        i += step.max(1);
    }
    if found_any { Some(p) } else { None }
}

/// Returns `(payload_len, terminator_len)` for the first BEL or ST in `slice`.
fn find_terminator(slice: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < slice.len() {
        if slice[i] == 0x07 {
            return Some((i, 1));
        }
        if slice[i] == 0x1b && i + 1 < slice.len() && slice[i + 1] == b'\\' {
            return Some((i, 2));
        }
        i += 1;
    }
    None
}

fn apply_payload(payload: &[u8], p: &mut Parsed) {
    let Ok(s) = std::str::from_utf8(payload) else {
        return;
    };
    let Some((ps, rest)) = s.split_once(';') else {
        return;
    };
    match ps {
        "10" => p.foreground = parse_color(rest).or(p.foreground),
        "11" => p.background = parse_color(rest).or(p.background),
        "4" => {
            let Some((idx_s, spec)) = rest.split_once(';') else {
                return;
            };
            if let Ok(idx) = idx_s.parse::<usize>()
                && idx < 16
                && let Some(c) = parse_color(spec)
            {
                p.palette[idx] = Some(c);
            }
        }
        _ => {}
    }
}

fn parse_color(s: &str) -> Option<Rgb> {
    let s = s.trim();
    let (scheme, rest) = s.split_once(':')?;
    let parts: [&str; 3] = rest.split('/').collect::<Vec<_>>().try_into().ok()?;
    if scheme.eq_ignore_ascii_case("rgb") {
        Some(Rgb(
            parse_hex_channel(parts[0])?,
            parse_hex_channel(parts[1])?,
            parse_hex_channel(parts[2])?,
        ))
    } else if scheme.eq_ignore_ascii_case("rgbi") {
        Some(Rgb(
            parse_int_channel(parts[0])?,
            parse_int_channel(parts[1])?,
            parse_int_channel(parts[2])?,
        ))
    } else {
        None
    }
}

fn parse_hex_channel(s: &str) -> Option<u8> {
    if s.is_empty() {
        return Some(0);
    }
    let len = s.len();
    if len > 4 {
        return None;
    }
    let val = u32::from_str_radix(s, 16).ok()?;
    let scaled = match len {
        1 => val * 17,
        2 => val,
        3 => (val * 255) / 0xfff,
        4 => (val * 255) / 0xffff,
        _ => return None,
    };
    Some(scaled.min(255) as u8)
}

fn parse_int_channel(s: &str) -> Option<u8> {
    let v: u16 = s.parse().ok()?;
    Some((v & 0xff) as u8)
}

fn normalize(p: Parsed) -> TerminalColors {
    let background = p.background.or(p.palette[0]).unwrap_or(Rgb(0, 0, 0));
    let foreground = p
        .foreground
        .or(p.palette[7])
        .unwrap_or(Rgb(0xff, 0xff, 0xff));
    TerminalColors {
        background,
        foreground,
        palette: p.palette,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_4_digit_rgb_responses_with_bel_terminator() {
        let blob = b"\x1b]10;rgb:dddd/dddd/eeee\x07\x1b]11;rgb:1414/1e1e/2626\x07";
        let parsed = parse_responses(blob).unwrap();
        assert_eq!(parsed.foreground, Some(Rgb(0xdd, 0xdd, 0xee)));
        assert_eq!(parsed.background, Some(Rgb(0x14, 0x1e, 0x26)));
    }

    #[test]
    fn parses_2_digit_palette_entries_with_st_terminator() {
        let blob = b"\x1b]4;0;rgb:1d/23/2f\x1b\\\x1b]4;7;rgb:c8/d3/f5\x1b\\";
        let parsed = parse_responses(blob).unwrap();
        assert_eq!(parsed.palette[0], Some(Rgb(0x1d, 0x23, 0x2f)));
        assert_eq!(parsed.palette[7], Some(Rgb(0xc8, 0xd3, 0xf5)));
    }

    #[test]
    fn mixes_bel_and_st_and_skips_unknown_ps() {
        let blob = b"\x1b]12;rgb:00/00/00\x1b\\\x1b]10;rgb:ff/ff/ff\x07\x1b]4;1;rgb:f7/76/8e\x1b\\";
        let parsed = parse_responses(blob).unwrap();
        assert_eq!(parsed.foreground, Some(Rgb(255, 255, 255)));
        assert_eq!(parsed.palette[1], Some(Rgb(0xf7, 0x76, 0x8e)));
    }

    #[test]
    fn parse_complete_only_when_all_channels_present() {
        assert!(!parse_complete(b"\x1b]10;rgb:ff/ff/ff\x07"));
        let mut blob = Vec::from(&b"\x1b]10;rgb:ff/ff/ff\x07\x1b]11;rgb:00/00/00\x07"[..]);
        for i in 0..16u8 {
            blob.extend_from_slice(format!("\x1b]4;{i};rgb:aa/bb/cc\x1b\\").as_bytes());
        }
        assert!(parse_complete(&blob));
    }

    #[test]
    fn normalize_falls_back_to_palette_then_defaults() {
        let mut p = Parsed::default();
        p.palette[0] = Some(Rgb(10, 20, 30));
        p.palette[7] = Some(Rgb(200, 210, 220));
        let colors = normalize(p);
        assert_eq!(colors.background, Rgb(10, 20, 30));
        assert_eq!(colors.foreground, Rgb(200, 210, 220));
    }

    #[test]
    fn hex_channel_scales_by_width() {
        assert_eq!(parse_hex_channel("ff"), Some(255));
        assert_eq!(parse_hex_channel("f"), Some(255));
        assert_eq!(parse_hex_channel("0"), Some(0));
        assert_eq!(parse_hex_channel("fff"), Some(255));
        assert_eq!(parse_hex_channel("80"), Some(128));
        assert_eq!(parse_hex_channel("7fff"), Some(127));
        assert_eq!(parse_hex_channel("ffff"), Some(255));
        assert!(parse_hex_channel("fffff").is_none());
    }
}
