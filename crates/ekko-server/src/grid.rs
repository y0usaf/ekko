//! Conversion from a `vt100::Screen` to the wire `GridUpdate` type.

use ekko_proto::{CursorState, GridCell, GridRow, MouseEncoding, MouseMode, TermModes, WireColor};

/// Extract every row of the screen as wire rows. The hub diffs consecutive
/// extractions to build sparse `GridPayload::Rows` updates.
pub fn screen_rows(screen: &vt100::Screen) -> Vec<GridRow> {
    let (rows, cols) = screen.size();
    let mut grid_rows = Vec::with_capacity(rows as usize);
    for row in 0..rows {
        let mut cells = Vec::with_capacity(cols as usize);
        for col in 0..cols {
            cells.push(match screen.cell(row, col) {
                Some(cell) => cell_to_wire(cell),
                None => empty_cell(),
            });
        }
        grid_rows.push(GridRow { cells });
    }
    grid_rows
}

/// The screen's current cursor position/visibility as the wire type. The
/// DECSCUSR shape is tracked outside the parser; the hub fills it in.
pub fn cursor_state(screen: &vt100::Screen) -> CursorState {
    let (cursor_row, cursor_col) = screen.cursor_position();
    CursorState {
        row: cursor_row,
        col: cursor_col,
        visible: !screen.hide_cursor(),
        shape: 0,
    }
}

/// The child-requested terminal modes the client must honor. The focus
/// reporting flag is tracked outside the parser; the hub fills it in.
pub fn term_modes(screen: &vt100::Screen) -> TermModes {
    TermModes {
        alt_screen: screen.alternate_screen(),
        app_cursor: screen.application_cursor(),
        mouse_mode: match screen.mouse_protocol_mode() {
            vt100::MouseProtocolMode::None => MouseMode::None,
            vt100::MouseProtocolMode::Press => MouseMode::Press,
            vt100::MouseProtocolMode::PressRelease => MouseMode::PressRelease,
            vt100::MouseProtocolMode::ButtonMotion => MouseMode::ButtonMotion,
            vt100::MouseProtocolMode::AnyMotion => MouseMode::AnyMotion,
        },
        mouse_encoding: match screen.mouse_protocol_encoding() {
            vt100::MouseProtocolEncoding::Default => MouseEncoding::Default,
            vt100::MouseProtocolEncoding::Utf8 => MouseEncoding::Utf8,
            vt100::MouseProtocolEncoding::Sgr => MouseEncoding::Sgr,
        },
        focus_reporting: false,
    }
}

fn cell_to_wire(cell: &vt100::Cell) -> GridCell {
    let mut chars = cell.contents().chars();
    let ch = chars.next().unwrap_or(' ');
    let extra: Vec<char> = chars.collect();
    let mut attrs = 0u8;
    if cell.bold() {
        attrs |= GridCell::BOLD;
    }
    if cell.dim() {
        attrs |= GridCell::DIM;
    }
    if cell.italic() {
        attrs |= GridCell::ITALIC;
    }
    if cell.underline() {
        attrs |= GridCell::UNDERLINE;
    }
    if cell.inverse() {
        attrs |= GridCell::INVERSE;
    }
    if cell.is_wide() {
        attrs |= GridCell::WIDE;
    }
    if cell.is_wide_continuation() {
        attrs |= GridCell::WIDE_CONT;
    }
    GridCell {
        ch,
        extra,
        fg: wire_color(cell.fgcolor()),
        bg: wire_color(cell.bgcolor()),
        attrs,
    }
}

fn empty_cell() -> GridCell {
    GridCell {
        ch: ' ',
        extra: Vec::new(),
        fg: WireColor::Default,
        bg: WireColor::Default,
        attrs: 0,
    }
}

fn wire_color(color: vt100::Color) -> WireColor {
    match color {
        vt100::Color::Default => WireColor::Default,
        vt100::Color::Idx(idx) => WireColor::Indexed(idx),
        vt100::Color::Rgb(r, g, b) => WireColor::Rgb(r, g, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_rows_reflect_processed_bytes() {
        let mut parser = vt100::Parser::new(3, 10, 0);
        parser.process(b"hi");
        let rows = screen_rows(parser.screen());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].cells.len(), 10);
        assert_eq!(rows[0].cells[0].ch, 'h');
        assert_eq!(rows[0].cells[1].ch, 'i');
        assert_eq!(rows[0].cells[2].ch, ' ');
    }

    #[test]
    fn combining_marks_survive_as_extra_codepoints() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        parser.process("e\u{0301}".as_bytes());
        let rows = screen_rows(parser.screen());
        assert_eq!(rows[0].cells[0].ch, 'e');
        assert_eq!(rows[0].cells[0].extra, vec!['\u{0301}']);
    }

    #[test]
    fn screen_rows_follow_the_scrollback_view() {
        let mut parser = vt100::Parser::new(2, 4, 100);
        parser.process(b"a\r\nb\r\nc\r\nd");
        parser.screen_mut().set_scrollback(2);
        let rows = screen_rows(parser.screen());
        assert_eq!(rows[0].cells[0].ch, 'a');
        assert_eq!(rows[1].cells[0].ch, 'b');
    }

    #[test]
    fn modes_track_decset_requests() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        parser.process(b"\x1b[?1002h\x1b[?1006h\x1b[?1049h\x1b[?1h");
        let modes = term_modes(parser.screen());
        assert_eq!(modes.mouse_mode, MouseMode::ButtonMotion);
        assert_eq!(modes.mouse_encoding, MouseEncoding::Sgr);
        assert!(modes.alt_screen);
        assert!(modes.app_cursor);
    }
}
