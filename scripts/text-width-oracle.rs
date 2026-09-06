extern crate unicode_width;

use unicode_width::UnicodeWidthChar;

fn main() {
    for cp in 0..=0x10ffff {
        if (0xd800..=0xdfff).contains(&cp) {
            continue;
        }
        let character = char::from_u32(cp).unwrap();
        println!("{}", character.width().unwrap_or(0));
    }
}
