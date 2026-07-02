//! Interned text + packed cell representation (Priority 1 of the renderer
//! optimization plan). Each cell is 16 bytes of pure stack data with zero
//! per-cell heap allocations, turning the diff loop into a linear scan of
//! contiguous `u64` comparisons.

use std::collections::HashMap;

use crate::color::Color;

// ---------------------------------------------------------------------------
// PackedStyle
// ---------------------------------------------------------------------------

/// Style flags live in a dedicated `u32` — never steal bits from the ARGB
/// `Color`, whose alpha tag (`0x01`) and truecolor alpha (`0xFF`) would be
/// corrupted.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[repr(C)]
pub struct PackedStyle {
    pub fg: u32,
    pub bg: u32,
    pub flags: u32, // Only lowest 4 bits used
}

impl PackedStyle {
    pub const BOLD_MASK: u32 = 1 << 0;
    pub const UNDERLINE_MASK: u32 = 1 << 1;
    pub const REVERSE_MASK: u32 = 1 << 2;
    pub const CONTINUATION_MASK: u32 = 1 << 3;
    pub const ITALIC_MASK: u32 = 1 << 4;

    pub fn new(
        fg: Color,
        bg: Color,
        bold: bool,
        underline: bool,
        reverse: bool,
        continuation: bool,
    ) -> Self {
        let mut flags = 0u32;
        if bold {
            flags |= Self::BOLD_MASK;
        }
        if underline {
            flags |= Self::UNDERLINE_MASK;
        }
        if reverse {
            flags |= Self::REVERSE_MASK;
        }
        if continuation {
            flags |= Self::CONTINUATION_MASK;
        }
        Self {
            fg: fg.0,
            bg: bg.0,
            flags,
        }
    }

    /// The same style with the italic flag set (kept out of `new` so the
    /// many chrome callers stay unchanged; only grid content uses italics).
    #[must_use]
    pub fn with_italic(mut self, italic: bool) -> Self {
        if italic {
            self.flags |= Self::ITALIC_MASK;
        } else {
            self.flags &= !Self::ITALIC_MASK;
        }
        self
    }

    #[inline]
    pub fn bold(&self) -> bool {
        self.flags & Self::BOLD_MASK != 0
    }

    #[inline]
    pub fn underline(&self) -> bool {
        self.flags & Self::UNDERLINE_MASK != 0
    }

    #[inline]
    pub fn reverse(&self) -> bool {
        self.flags & Self::REVERSE_MASK != 0
    }

    #[inline]
    pub fn continuation(&self) -> bool {
        self.flags & Self::CONTINUATION_MASK != 0
    }

    #[inline]
    pub fn italic(&self) -> bool {
        self.flags & Self::ITALIC_MASK != 0
    }

    #[inline]
    pub fn fg(&self) -> Color {
        Color(self.fg)
    }

    #[inline]
    pub fn bg(&self) -> Color {
        Color(self.bg)
    }
}

// ---------------------------------------------------------------------------
// PackedText
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct PackedText(pub u32);

impl PackedText {
    const INTERNED_FLAG: u32 = 0x8000_0000;
    const MASK: u32 = 0x7FFF_FFFF;

    pub fn ascii(byte: u8) -> Self {
        Self(byte as u32)
    }

    pub fn interned(index: u32) -> Self {
        Self(index | Self::INTERNED_FLAG)
    }

    #[inline]
    pub fn is_interned(&self) -> bool {
        (self.0 & Self::INTERNED_FLAG) != 0
    }

    #[inline]
    pub fn index(&self) -> usize {
        (self.0 & Self::MASK) as usize
    }

    #[inline]
    pub fn ascii_byte(&self) -> u8 {
        (self.0 & Self::MASK) as u8
    }

    /// The single-ASCII fast path returns `Some(byte)`.
    pub fn as_ascii(&self) -> Option<u8> {
        if self.is_interned() {
            None
        } else {
            Some(self.ascii_byte())
        }
    }
}

// ---------------------------------------------------------------------------
// PackedCell
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct PackedCell {
    pub style: PackedStyle,
    pub text: PackedText,
}

// ---------------------------------------------------------------------------
// StringInterner
// ---------------------------------------------------------------------------

/// Circuit breaker: once we exceed this many interned strings we stop
/// growing the map and fall back to a side-table so streaming random unicode
/// noise can't exhaust memory (~2 MB cap).
const INTERNER_CAP: usize = 50_000;

#[derive(Clone, Default)]
pub struct StringInterner {
    map: HashMap<String, u32>,
    strings: Vec<String>,
    /// Side-table for overflow entries once `strings.len()` hits the cap.
    overflow: Vec<String>,
}

impl StringInterner {
    pub fn get_or_intern(&mut self, text: &str) -> PackedText {
        // Fast path: single ASCII codepoint — no allocation, no lookup.
        if text.len() == 1 && text.is_ascii() {
            return PackedText::ascii(text.as_bytes()[0]);
        }
        if let Some(&idx) = self.map.get(text) {
            return PackedText::interned(idx);
        }
        if self.strings.len() < INTERNER_CAP {
            let idx = self.strings.len() as u32;
            self.strings.push(text.to_string());
            self.map.insert(text.to_string(), idx);
            PackedText::interned(idx)
        } else {
            // Overflow: store in the side-table and encode the index
            // offset by the cap so resolve can find it.
            let idx = self.overflow.len() as u32;
            self.overflow.push(text.to_string());
            PackedText::interned(idx + INTERNER_CAP as u32)
        }
    }

    pub fn resolve(&self, text: PackedText) -> &str {
        if text.is_interned() {
            let idx = text.index();
            if idx < INTERNER_CAP {
                &self.strings[idx]
            } else {
                &self.overflow[idx - INTERNER_CAP]
            }
        } else {
            // SAFETY: PackedText::ascii is only produced for single ASCII
            // codepoints (get_or_intern checks `is_ascii()`), so the byte
            // is always < 128 and valid single-byte UTF-8.
            const ASCII_SINGLE: [&str; 128] = [
                "\x00", "\x01", "\x02", "\x03", "\x04", "\x05", "\x06", "\x07", "\x08", "\x09",
                "\x0A", "\x0B", "\x0C", "\x0D", "\x0E", "\x0F", "\x10", "\x11", "\x12", "\x13",
                "\x14", "\x15", "\x16", "\x17", "\x18", "\x19", "\x1A", "\x1B", "\x1C", "\x1D",
                "\x1E", "\x1F", " ", "!", "\"", "#", "$", "%", "&", "'", "(", ")", "*", "+", ",",
                "-", ".", "/", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", ":", ";", "<",
                "=", ">", "?", "@", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L",
                "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "[", "\\",
                "]", "^", "_", "`", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l",
                "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "{", "|",
                "}", "~", "\x7F",
            ];
            ASCII_SINGLE[(text.ascii_byte() & 0x7F) as usize]
        }
    }

    /// Number of interned (non-ascii) strings.
    pub fn len(&self) -> usize {
        self.strings.len() + self.overflow.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
