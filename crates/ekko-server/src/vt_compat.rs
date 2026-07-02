//! Compensation for escape sequences the `vt100` crate does not dispatch.
//!
//! `vt100` 0.16 has no arm for the HVP final (`CSI Ps ; Ps f`), so programs
//! that position with HVP instead of CUP (`H`) — btop hardcodes it — degrade
//! into an unpositioned text stream. HVP and CUP are defined identically
//! (both honor origin mode the same way), so the fix is a byte-for-byte
//! rewrite of the final `f` to `H` before the parser sees it.
//!
//! The rewriter is a persistent state machine, not a regex: PTY reads can
//! split a sequence across chunks, and a literal `f` inside an OSC/DCS
//! string or plain text must never be touched. Only a CSI with no
//! intermediates and no private parameter markers is eligible.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    /// After ESC (including any escape intermediates 0x20-0x2f).
    Escape,
    /// Inside CSI. `plain` stays true only while every byte seen is a
    /// digit/`;`/`:` parameter — the only shape HVP can take.
    Csi { plain: bool },
    /// Inside an OSC string (ESC ] ... BEL/ST).
    Osc,
    /// Inside a DCS/SOS/PM/APC string (ESC P/X/^/_ ... ST).
    OtherString,
}

/// Rewrites HVP finals to CUP in a PTY byte stream, preserving state across
/// chunks. One instance per PTY stream; reset it (via `Default`) whenever
/// the stream restarts.
#[derive(Debug, Default)]
pub(crate) struct HvpToCup {
    state: State,
}

impl HvpToCup {
    pub(crate) fn rewrite_in_place(&mut self, bytes: &mut [u8]) {
        for byte in bytes {
            self.state = match self.state {
                State::Ground => match byte {
                    0x1b => State::Escape,
                    _ => State::Ground,
                },
                State::Escape => match byte {
                    b'[' => State::Csi { plain: true },
                    b']' => State::Osc,
                    b'P' | b'X' | b'^' | b'_' => State::OtherString,
                    0x1b => State::Escape,
                    0x20..=0x2f => State::Escape, // escape intermediates
                    _ => State::Ground,           // single-char escape (ESC 7, ESC M, ...)
                },
                State::Csi { plain } => match byte {
                    // Parameter bytes; `<=>?` mark a private sequence.
                    0x30..=0x3f => State::Csi {
                        plain: plain && !matches!(byte, 0x3c..=0x3f),
                    },
                    // Intermediates disqualify the plain-HVP shape.
                    0x20..=0x2f => State::Csi { plain: false },
                    0x40..=0x7e => {
                        if plain && *byte == b'f' {
                            *byte = b'H';
                        }
                        State::Ground
                    }
                    0x1b => State::Escape,
                    0x18 | 0x1a => State::Ground, // CAN/SUB abort
                    // Other C0 controls execute without ending the CSI.
                    _ => State::Csi { plain },
                },
                State::Osc => match byte {
                    0x07 => State::Ground, // BEL terminator
                    0x1b => State::Escape, // ST is ESC \; Escape handles the \
                    0x18 | 0x1a => State::Ground,
                    _ => State::Osc,
                },
                State::OtherString => match byte {
                    0x1b => State::Escape,
                    0x18 | 0x1a => State::Ground,
                    _ => State::OtherString,
                },
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(input: &[u8]) -> Vec<u8> {
        let mut bytes = input.to_vec();
        HvpToCup::default().rewrite_in_place(&mut bytes);
        bytes
    }

    #[test]
    fn rewrites_hvp_to_cup() {
        assert_eq!(rewrite(b"\x1b[5;10fX"), b"\x1b[5;10HX");
        assert_eq!(rewrite(b"\x1b[f"), b"\x1b[H");
    }

    #[test]
    fn leaves_plain_text_f_alone() {
        assert_eq!(rewrite(b"before f after"), b"before f after");
    }

    #[test]
    fn leaves_other_csi_finals_alone() {
        assert_eq!(
            rewrite(b"\x1b[1;1H\x1b[38;5;10m"),
            b"\x1b[1;1H\x1b[38;5;10m"
        );
    }

    #[test]
    fn private_and_intermediate_sequences_are_not_hvp() {
        assert_eq!(rewrite(b"\x1b[?25f"), b"\x1b[?25f");
        assert_eq!(rewrite(b"\x1b[1 f"), b"\x1b[1 f");
    }

    #[test]
    fn osc_payload_is_untouched() {
        assert_eq!(
            rewrite(b"\x1b]0;a f title\x07\x1b[1;1f"),
            b"\x1b]0;a f title\x07\x1b[1;1H"
        );
        assert_eq!(
            rewrite(b"\x1b]0;a f title\x1b\\\x1b[2;2f"),
            b"\x1b]0;a f title\x1b\\\x1b[2;2H"
        );
    }

    #[test]
    fn dcs_payload_is_untouched() {
        assert_eq!(
            rewrite(b"\x1bPq f f f\x1b\\\x1b[3;4f"),
            b"\x1bPq f f f\x1b\\\x1b[3;4H"
        );
    }

    #[test]
    fn sequences_split_across_chunks_still_rewrite() {
        let mut filter = HvpToCup::default();
        let mut a = b"\x1b[12;".to_vec();
        let mut b = b"34f".to_vec();
        filter.rewrite_in_place(&mut a);
        filter.rewrite_in_place(&mut b);
        assert_eq!(a, b"\x1b[12;");
        assert_eq!(b, b"34H");
    }

    #[test]
    fn esc_split_across_chunks_still_rewrites() {
        let mut filter = HvpToCup::default();
        let mut a = b"text\x1b".to_vec();
        let mut b = b"[7;8f".to_vec();
        filter.rewrite_in_place(&mut a);
        filter.rewrite_in_place(&mut b);
        assert_eq!(b, b"[7;8H");
    }

    #[test]
    fn esc_inside_csi_restarts_the_sequence() {
        assert_eq!(rewrite(b"\x1b[1;2\x1b[3;4f"), b"\x1b[1;2\x1b[3;4H");
    }
}
