//! Display-cell width helpers shared by the inline TUI and the mux
//! compositor: terminal-column accounting for monospaced grids.

pub fn display_cell_width(value: &str) -> usize {
    value
        .chars()
        .map(|ch| {
            unicode_width::UnicodeWidthChar::width(ch)
                .unwrap_or(1)
                .max(1)
        })
        .sum()
}

pub fn truncate_to_cells(value: &str, max_cols: usize) -> String {
    if display_cell_width(value) <= max_cols {
        return value.to_string();
    }
    if max_cols == 0 {
        return String::new();
    }
    let ellipsis = "\u{2026}";
    let ellipsis_w = display_cell_width(ellipsis);
    if max_cols <= ellipsis_w {
        return ellipsis.to_string();
    }
    let mut out = String::with_capacity(max_cols);
    let mut width = 0;
    for ch in value.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch)
            .unwrap_or(1)
            .max(1);
        if width + ch_width + ellipsis_w > max_cols {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push_str(ellipsis);
    out
}

// ---------------------------------------------------------------------------
// Style-preserving span truncation (ported from jcode).
//
// phi doesn't use ratatui; its styled text is a `String` with embedded ANSI
// escape sequences (SGR for colors/attributes, OSC 8 for hyperlinks).  The
// functions below walk the ANSI string, preserve escape sequences within the
// kept region, track active SGR state, and re-emit it for the ellipsis so
// truncation never drops colors or attributes.
// ---------------------------------------------------------------------------

/// Byte length of an ANSI escape-sequence body (the part after `ESC`).
fn ansi_escape_len(after_esc: &str) -> usize {
    let mut chars = after_esc.char_indices();
    match chars.next() {
        Some((_, '[')) => {
            for (i, c) in chars {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    return i + c.len_utf8();
                }
            }
            after_esc.len()
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
            after_esc.len()
        }
        Some((_, c)) => c.len_utf8(),
        None => 0,
    }
}

/// Track whether any SGR (color/attribute) escape is currently active.
/// We only need to know *whether* style is open at the truncation point so we
/// can emit a trailing `RESET` — the actual codes are already in the output
/// buffer, so a simple boolean suffices.
fn sgr_is_reset(esc: &str) -> bool {
    matches!(esc, "\x1b[0m" | "\x1b[m")
}

/// Visible (display) width of `s`, ignoring ANSI escape sequences.
fn ansi_visible_width(s: &str) -> usize {
    let mut width = 0usize;
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('\x1b') {
            let len = ansi_escape_len(after) + 1;
            rest = &rest[len..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        width += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        rest = &rest[ch.len_utf8()..];
    }
    width
}

/// Truncate an ANSI-styled line to at most `width` visible columns,
/// preserving embedded SGR escape sequences. Appends `RESET` if any
/// style was left open at the cut point.
pub fn truncate_line_to_width(line: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(line.len().min(width * 4));
    let mut sgr_active = false;
    let mut remaining = width;

    let mut rest = line;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('\x1b') {
            if remaining == 0 {
                break;
            }
            let len = ansi_escape_len(after) + 1;
            let esc = &rest[..len];
            out.push_str(esc);
            sgr_active = !sgr_is_reset(esc);
            rest = &rest[len..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if cw > remaining {
            break;
        }
        out.push(ch);
        remaining = remaining.saturating_sub(cw);
        rest = &rest[ch.len_utf8()..];
    }

    if sgr_active {
        out.push_str("\x1b[0m");
    }
    out
}

/// Truncate an ANSI-styled line in place to at most `max_width` visible
/// columns. See [`truncate_line_to_width`].
pub fn truncate_line_in_place_to_width(line: &mut String, max_width: usize) {
    *line = truncate_line_to_width(line, max_width);
}

/// Truncate an ANSI-styled line to `width` visible columns, appending `...`
/// (styled with the SGR state at the truncation point) when content is cut.
pub fn truncate_line_with_ellipsis_to_width(line: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if ansi_visible_width(line) <= width {
        return line.to_owned();
    }
    if width == 1 {
        return "\u{2026}".to_owned();
    }

    // Reserve 1 cell for the ellipsis.
    let budget = width - 1;
    let mut out = String::with_capacity(line.len().min(width * 4));
    let mut sgr_active = false;
    let mut remaining = budget;

    let mut rest = line;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('\x1b') {
            if remaining == 0 {
                break;
            }
            let len = ansi_escape_len(after) + 1;
            let esc = &rest[..len];
            out.push_str(esc);
            sgr_active = !sgr_is_reset(esc);
            rest = &rest[len..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if cw > remaining {
            break;
        }
        out.push(ch);
        remaining = remaining.saturating_sub(cw);
        rest = &rest[ch.len_utf8()..];
    }

    // The ellipsis inherits the active SGR state (escape codes are already
    // in `out` and haven't been reset).
    out.push('\u{2026}');
    if sgr_active {
        out.push_str("\x1b[0m");
    }
    out
}

/// Fit `prefix` + `suffix` into `width` visible columns. When the
/// combination overflows, the prefix is truncated (with ellipsis) to
/// leave room for the full suffix -- e.g. for file extensions.
pub fn truncate_line_preserving_suffix_to_width(
    prefix: &str,
    suffix: &str,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }

    let suffix_width = ansi_visible_width(suffix);
    if suffix_width == 0 {
        return truncate_line_with_ellipsis_to_width(prefix, width);
    }

    let combined = format!("{prefix}{suffix}");
    if ansi_visible_width(&combined) <= width {
        return combined;
    }

    if suffix_width >= width {
        return truncate_line_with_ellipsis_to_width(suffix, width);
    }

    let prefix_budget = width.saturating_sub(suffix_width);
    let mut prefix_part = truncate_line_with_ellipsis_to_width(prefix, prefix_budget);
    prefix_part.push_str(suffix);
    prefix_part
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lock `U+23BF` (bottom-left crop) to width 1.
    #[test]
    fn crop_mark_is_width_one() {
        assert_eq!(display_cell_width("\u{23bf}"), 1);
        assert_eq!(display_cell_width("\u{23bf} "), 2);
        assert_eq!(display_cell_width("\u{23bf} \u{23bf} "), 4);
    }

    // ---- truncate_line_to_width ----

    #[test]
    fn truncate_line_to_width_plain() {
        assert_eq!(truncate_line_to_width("Hello, world!", 5), "Hello");
        assert_eq!(truncate_line_to_width("Hello", 10), "Hello");
        assert_eq!(truncate_line_to_width("", 5), "");
        assert_eq!(truncate_line_to_width("ABC", 0), "");
    }

    #[test]
    fn truncate_line_to_width_preserves_ansi() {
        let line = "\x1b[31mHello\x1b[0m";
        assert_eq!(truncate_line_to_width(line, 3), "\x1b[31mHel\x1b[0m");
    }

    #[test]
    fn truncate_line_to_width_wide_chars() {
        assert_eq!(
            truncate_line_to_width("\u{65e5}\u{672c}\u{8a9e}", 4),
            "\u{65e5}\u{672c}"
        );
    }

    #[test]
    fn truncate_line_in_place_basic() {
        let mut line = String::from("Hello, world!");
        truncate_line_in_place_to_width(&mut line, 5);
        assert_eq!(line, "Hello");
    }

    // ---- truncate_line_with_ellipsis_to_width ----

    #[test]
    fn ellipsis_plain() {
        assert_eq!(
            truncate_line_with_ellipsis_to_width("Hello, world!", 8),
            "Hello, \u{2026}"
        );
        assert_eq!(truncate_line_with_ellipsis_to_width("Hi", 5), "Hi");
        assert_eq!(truncate_line_with_ellipsis_to_width("AB", 1), "\u{2026}");
        assert_eq!(truncate_line_with_ellipsis_to_width("ABC", 0), "");
    }

    #[test]
    fn ellipsis_preserves_style() {
        let line = "\x1b[31mHello\x1b[0m";
        assert_eq!(
            truncate_line_with_ellipsis_to_width(line, 4),
            "\x1b[31mHel\u{2026}\x1b[0m"
        );
    }

    #[test]
    fn ellipsis_no_trailing_reset_when_unstyled() {
        let line = "\x1b[31mAB\x1b[0mCDEF";
        assert_eq!(
            truncate_line_with_ellipsis_to_width(line, 4),
            "\x1b[31mAB\x1b[0mC\u{2026}"
        );
    }

    // ---- truncate_line_preserving_suffix_to_width ----

    #[test]
    fn suffix_preserving_fits() {
        assert_eq!(
            truncate_line_preserving_suffix_to_width("file", ".rs", 10),
            "file.rs"
        );
    }

    #[test]
    fn suffix_preserving_truncates_prefix() {
        assert_eq!(
            truncate_line_preserving_suffix_to_width("very_long_filename", ".rs", 10),
            "very_l\u{2026}.rs"
        );
    }

    #[test]
    fn suffix_preserving_suffix_too_wide() {
        assert_eq!(
            truncate_line_preserving_suffix_to_width("ab", "very_long_suffix", 5),
            "very\u{2026}"
        );
    }

    #[test]
    fn suffix_preserving_empty_suffix() {
        assert_eq!(
            truncate_line_preserving_suffix_to_width("Hello", "", 3),
            "He\u{2026}"
        );
    }

    #[test]
    fn suffix_preserving_styled() {
        let prefix = "\x1b[31mlong_prefix\x1b[0m";
        let suffix = "\x1b[32m.rs\x1b[0m";
        let result = truncate_line_preserving_suffix_to_width(prefix, suffix, 8);
        assert_eq!(result, "\x1b[31mlong\u{2026}\x1b[0m\x1b[32m.rs\x1b[0m");
    }
}
