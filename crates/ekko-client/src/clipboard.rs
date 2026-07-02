//! OSC 52 clipboard writes to the host terminal. The escape carries
//! base64-encoded data; a tiny local encoder keeps the dependency tree flat.

/// Build an OSC 52 sequence setting the system clipboard to `data`.
pub fn osc52_set_clipboard(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 4 / 3 + 16);
    out.extend_from_slice(b"\x1b]52;c;");
    out.extend_from_slice(base64_encode(data).as_bytes());
    out.extend_from_slice(b"\x07");
    out
}

/// Build an OSC 52 sequence from data that is already base64-encoded (as
/// delivered by the server's vt100 callback).
pub fn osc52_set_clipboard_base64(encoded: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(encoded.len() + 16);
    out.extend_from_slice(b"\x1b]52;c;");
    out.extend_from_slice(encoded);
    out.extend_from_slice(b"\x07");
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
        assert_eq!(osc52_set_clipboard(b"hi"), b"\x1b]52;c;aGk=\x07".to_vec());
        assert_eq!(
            osc52_set_clipboard_base64(b"aGk="),
            b"\x1b]52;c;aGk=\x07".to_vec()
        );
    }
}
