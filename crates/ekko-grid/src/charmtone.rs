//! Charmtone palette constants.
//!
//! Verbatim hex values from
//! `github.com/charmbracelet/x/exp/charmtone` (`charmtone.go`). Kept as
//! plain `Color` constants so theme tables can be expressed declaratively.

use crate::color::Color;

// Reds / oranges
pub const CUMIN: Color = Color::rgb(0xBF, 0x97, 0x6F);
pub const TANG: Color = Color::rgb(0xFF, 0x98, 0x5A);
pub const YAM: Color = Color::rgb(0xFF, 0xB5, 0x87);
pub const PAPRIKA: Color = Color::rgb(0xD3, 0x6C, 0x64);
pub const BENGAL: Color = Color::rgb(0xFF, 0x6E, 0x63);
pub const UNI: Color = Color::rgb(0xFF, 0x93, 0x7D);
pub const SRIRACHA: Color = Color::rgb(0xEB, 0x42, 0x68);
pub const CORAL: Color = Color::rgb(0xFF, 0x57, 0x7D);
pub const SALMON: Color = Color::rgb(0xFF, 0x7F, 0x90);
pub const CHILI: Color = Color::rgb(0xE2, 0x30, 0x80);
pub const CHERRY: Color = Color::rgb(0xFF, 0x38, 0x8B);

// Pinks / magentas
pub const TUNA: Color = Color::rgb(0xFF, 0x6D, 0xAA);
pub const MACARON: Color = Color::rgb(0xE9, 0x40, 0xB0);
pub const PONY: Color = Color::rgb(0xFF, 0x4F, 0xBF);
pub const CHEEKY: Color = Color::rgb(0xFF, 0x79, 0xD0);
pub const FLAMINGO: Color = Color::rgb(0xF9, 0x47, 0xE3);
pub const DOLLY: Color = Color::rgb(0xFF, 0x60, 0xFF);
pub const BLUSH: Color = Color::rgb(0xFF, 0x84, 0xFF);

// Purples
pub const URCHIN: Color = Color::rgb(0xC3, 0x37, 0xE0);
pub const MOCHI: Color = Color::rgb(0xEB, 0x5D, 0xFF);
pub const LILAC: Color = Color::rgb(0xF3, 0x79, 0xFF);
pub const PRINCE: Color = Color::rgb(0x9C, 0x35, 0xE1);
pub const VIOLET: Color = Color::rgb(0xC2, 0x59, 0xFF);
pub const MAUVE: Color = Color::rgb(0xD4, 0x6E, 0xFF);
pub const GRAPE: Color = Color::rgb(0x71, 0x34, 0xDD);
pub const PLUM: Color = Color::rgb(0x99, 0x53, 0xFF);
pub const ORCHID: Color = Color::rgb(0xAD, 0x6E, 0xFF);
pub const JELLY: Color = Color::rgb(0x4A, 0x30, 0xD9);
pub const CHARPLE: Color = Color::rgb(0x6B, 0x50, 0xFF);
pub const HAZY: Color = Color::rgb(0x8B, 0x75, 0xFF);

// Blues
pub const OX: Color = Color::rgb(0x33, 0x31, 0xB2);
pub const SAPPHIRE: Color = Color::rgb(0x49, 0x49, 0xFF);
pub const GUPPY: Color = Color::rgb(0x72, 0x72, 0xFF);
pub const OCEANIA: Color = Color::rgb(0x2B, 0x55, 0xB3);
pub const THUNDER: Color = Color::rgb(0x47, 0x76, 0xFF);
pub const ANCHOVY: Color = Color::rgb(0x71, 0x9A, 0xFC);
pub const DAMSON: Color = Color::rgb(0x00, 0x7A, 0xB8);
pub const MALIBU: Color = Color::rgb(0x00, 0xA4, 0xFF);
pub const SARDINE: Color = Color::rgb(0x4F, 0xBE, 0xFE);

// Greens / cyans
pub const ZINC: Color = Color::rgb(0x10, 0xB1, 0xAE);
pub const TURTLE: Color = Color::rgb(0x0A, 0xDC, 0xD9);
pub const LICHEN: Color = Color::rgb(0x5C, 0xDF, 0xEA);
pub const GUAC: Color = Color::rgb(0x12, 0xC7, 0x8F);
pub const JULEP: Color = Color::rgb(0x00, 0xFF, 0xB2);
pub const BOK: Color = Color::rgb(0x68, 0xFF, 0xD6);

// Yellows
pub const MUSTARD: Color = Color::rgb(0xF5, 0xEF, 0x34);
pub const CITRON: Color = Color::rgb(0xE8, 0xFF, 0x27);
pub const ZEST: Color = Color::rgb(0xE8, 0xFE, 0x96);

// Neutrals (dark → light)
pub const PEPPER: Color = Color::rgb(0x20, 0x1F, 0x26);
pub const BBQ: Color = Color::rgb(0x2D, 0x2C, 0x35);
pub const CHARCOAL: Color = Color::rgb(0x3A, 0x39, 0x43);
pub const IRON: Color = Color::rgb(0x4D, 0x4C, 0x57);
pub const OYSTER: Color = Color::rgb(0x60, 0x5F, 0x6B);
pub const SQUID: Color = Color::rgb(0x85, 0x83, 0x92);
pub const SMOKE: Color = Color::rgb(0xBF, 0xBC, 0xC8);
pub const ASH: Color = Color::rgb(0xDF, 0xDB, 0xDD);
pub const SALT: Color = Color::rgb(0xF1, 0xEF, 0xEF);
pub const BUTTER: Color = Color::rgb(0xFF, 0xFA, 0xF1);
