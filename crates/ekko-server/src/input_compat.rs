//! Input-direction compensation: cursor-key encoding (DECCKM).
//!
//! The host terminal encodes arrows/Home/End according to *its own*
//! cursor-key mode, but the child decides which form it understands via
//! DECCKM (`CSI ? 1 h/l`). Full-screen children (cmus, vim, less) enable
//! application cursor keys, and with `TERM=xterm-256color` ncurses only
//! recognizes the SS3 forms its terminfo declares (`kcuu1=\EOA`, ...). The
//! host is almost never in that mode, so its CSI arrows must be rewritten
//! to SS3 — and symmetrically, SS3 arrows from an application-mode host
//! must fall back to CSI for a child that never asked. tmux and zellij
//! both perform this translation at the pane boundary (zellij:
//! `TerminalPane::adjust_input_to_terminal`, gated on `cursor_key_mode`).
//!
//! It runs in the hub because the pane's parser is the authoritative mode
//! source: the client's `TermModes` copy lags by a frame, and a child that
//! sets DECCKM then immediately reads keys would race it.
//!
//! Only the six unmodified single-final sequences (`A B C D` arrows, `H F`
//! Home/End) are eligible, in exact CSI (`ESC [ X`) or SS3 (`ESC O X`)
//! form. Modified variants (`ESC [ 1;5A`) are mode-independent, and SS3
//! `P`-`S` (F1-F4) are sent that way regardless of DECCKM, so both stay
//! untouched. Because the finals are terminating bytes, `ESC [ X` cannot
//! be a prefix of a longer well-formed sequence — matching the 3-byte
//! window is unambiguous, no CSI state machine needed (unlike the HVP
//! rewrite in `vt_compat`, whose final can follow parameter bytes).

/// Finals whose encoding is governed by DECCKM.
fn is_cursor_key_final(byte: u8) -> bool {
    matches!(byte, b'A'..=b'D' | b'H' | b'F')
}

/// Rewrite every unmodified cursor-key sequence in `bytes` to the form the
/// child expects: SS3 when `app_cursor` is set, CSI when it is not. The
/// rewrite is byte-for-byte (`[` <-> `O`), hence in place.
///
/// `bytes` is one client `Key` message: whole input tokens, possibly
/// several coalesced. A sequence split across messages (possible only at
/// machine rate, never from a human keypress) passes through untranslated,
/// matching the client tokenizer's own truncation policy.
pub(crate) fn rewrite_cursor_keys(bytes: &mut [u8], app_cursor: bool) {
    let introducer = if app_cursor { b'O' } else { b'[' };
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == 0x1b
            && (bytes[i + 1] == b'[' || bytes[i + 1] == b'O')
            && is_cursor_key_final(bytes[i + 2])
        {
            bytes[i + 1] = introducer;
            i += 3;
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(input: &[u8], app_cursor: bool) -> Vec<u8> {
        let mut bytes = input.to_vec();
        rewrite_cursor_keys(&mut bytes, app_cursor);
        bytes
    }

    #[test]
    fn app_cursor_rewrites_csi_to_ss3() {
        assert_eq!(rewrite(b"\x1b[A", true), b"\x1bOA");
        assert_eq!(rewrite(b"\x1b[B", true), b"\x1bOB");
        assert_eq!(rewrite(b"\x1b[C", true), b"\x1bOC");
        assert_eq!(rewrite(b"\x1b[D", true), b"\x1bOD");
        assert_eq!(rewrite(b"\x1b[H", true), b"\x1bOH");
        assert_eq!(rewrite(b"\x1b[F", true), b"\x1bOF");
    }

    #[test]
    fn normal_mode_rewrites_ss3_to_csi() {
        assert_eq!(rewrite(b"\x1bOA", false), b"\x1b[A");
        assert_eq!(rewrite(b"\x1bOF", false), b"\x1b[F");
    }

    #[test]
    fn already_matching_forms_are_stable() {
        assert_eq!(rewrite(b"\x1bOA", true), b"\x1bOA");
        assert_eq!(rewrite(b"\x1b[A", false), b"\x1b[A");
    }

    #[test]
    fn coalesced_autorepeat_rewrites_every_sequence() {
        assert_eq!(rewrite(b"\x1b[A\x1b[A\x1b[B", true), b"\x1bOA\x1bOA\x1bOB");
    }

    #[test]
    fn modified_arrows_are_mode_independent() {
        assert_eq!(rewrite(b"\x1b[1;5A", true), b"\x1b[1;5A");
        assert_eq!(rewrite(b"\x1b[1;3D", true), b"\x1b[1;3D");
    }

    #[test]
    fn ss3_function_keys_are_not_cursor_keys() {
        // F1-F4 are SS3 P-S in both DECCKM states.
        assert_eq!(rewrite(b"\x1bOP", false), b"\x1bOP");
        assert_eq!(rewrite(b"\x1bOS", false), b"\x1bOS");
    }

    #[test]
    fn mouse_and_focus_reports_are_untouched() {
        assert_eq!(rewrite(b"\x1b[<0;5;10M", true), b"\x1b[<0;5;10M");
        assert_eq!(
            rewrite(&[0x1b, b'[', b'M', 32, 33, 34], true),
            [0x1b, b'[', b'M', 32, 33, 34]
        );
        assert_eq!(rewrite(b"\x1b[I", true), b"\x1b[I");
        assert_eq!(rewrite(b"\x1b[O", true), b"\x1b[O");
    }

    #[test]
    fn plain_text_and_short_buffers_are_untouched() {
        assert_eq!(rewrite(b"OA [A", true), b"OA [A");
        assert_eq!(rewrite(b"\x1b[", true), b"\x1b[");
        assert_eq!(rewrite(b"\x1b", true), b"\x1b");
        assert_eq!(rewrite(b"", true), b"");
    }

    #[test]
    fn arrow_after_text_in_one_message_rewrites() {
        assert_eq!(rewrite(b"ls\x1b[A", true), b"ls\x1bOA");
    }
}
