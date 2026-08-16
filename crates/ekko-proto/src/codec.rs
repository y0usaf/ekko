//! Hand-rolled, dependency-free wire encoding for the `ekko-proto` messages.
//!
//! This replaces `serde` + `bincode` while preserving the exact byte layout
//! bincode 1.3 with `DefaultOptions::new().with_fixint_encoding()`
//! (`allow_trailing_bytes`, little-endian) produced, which the golden test
//! pins. Rules:
//!
//! - All integers are little-endian fixed-width.
//! - `bool` = 1 byte (`0`/`1`).
//! - `char` = raw UTF-8 bytes (no length prefix).
//! - `String` / `PathBuf` / `Vec` / slices = a `u64`-sized byte/len prefix
//!   (`u64` little-endian), then the elements.
//! - `Option<T>` = 1 byte tag (`0` None, `1` Some), then `T`.
//! - enums = `u32` little-endian discriminant, then the payload.
//! - structs = fields serialized in declaration order, no framing.
//! - tuples / arrays = elements in sequence, no framing.
//!
//! The trait is implemented once per wire type in `msg.rs`.

use std::fmt;

/// Error decoding a wire message.
#[derive(Debug)]
pub enum DecodeError {
    Io(std::io::Error),
    /// Ran out of bytes before the frame was fully read.
    Truncated,
    /// A discriminant or tag had an unknown value.
    InvalidTag(&'static str),
    Mismatch(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Truncated => write!(f, "wire decode: truncated input"),
            Self::InvalidTag(tag) => write!(f, "wire decode: invalid {tag}"),
            Self::Mismatch(m) => write!(f, "wire decode: {m}"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Truncated | Self::InvalidTag(_) | Self::Mismatch(_) => None,
        }
    }
}

impl From<std::io::Error> for DecodeError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Serialize `T` to a `Vec<u8>` in the fixed wire layout.
pub fn encode<T: Wire>(value: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    value.write(&mut buf);
    buf
}

/// Deserialize `T` from a complete byte buffer.
pub fn decode<T: Wire>(bytes: &[u8]) -> Result<T, DecodeError> {
    let mut pos = 0usize;
    T::read(bytes, &mut pos)
}

/// A type that can be written to / read from the wire.
pub trait Wire: Sized {
    fn write(&self, out: &mut Vec<u8>);
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError>;
}

// ---- primitives ----------------------------------------------------------

impl Wire for u8 {
    fn write(&self, out: &mut Vec<u8>) {
        out.push(*self);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        let b = *bytes.get(*pos).ok_or(DecodeError::Truncated)?;
        *pos += 1;
        Ok(b)
    }
}

fn le16(n: u16) -> [u8; 2] {
    n.to_le_bytes()
}
fn le32(n: u32) -> [u8; 4] {
    n.to_le_bytes()
}
fn le64(n: u64) -> [u8; 8] {
    n.to_le_bytes()
}

impl Wire for u16 {
    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&le16(*self));
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        let s = bytes.get(*pos..*pos + 2).ok_or(DecodeError::Truncated)?;
        *pos += 2;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
}

impl Wire for u32 {
    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&le32(*self));
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        let s = bytes.get(*pos..*pos + 4).ok_or(DecodeError::Truncated)?;
        *pos += 4;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
}

impl Wire for u64 {
    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&le64(*self));
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        let s = bytes.get(*pos..*pos + 8).ok_or(DecodeError::Truncated)?;
        *pos += 8;
        Ok(u64::from_le_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]))
    }
}

impl Wire for i32 {
    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        let b = <[u8; 4]>::try_from(bytes.get(*pos..*pos + 4).ok_or(DecodeError::Truncated)?)
            .map_err(|_| DecodeError::Truncated)?;
        *pos += 4;
        Ok(i32::from_le_bytes(b))
    }
}

impl Wire for bool {
    fn write(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        match u8::read(bytes, pos)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::InvalidTag("bool")),
        }
    }
}

impl Wire for char {
    fn write(&self, out: &mut Vec<u8>) {
        let mut buf = [0u8; 4];
        out.extend_from_slice(self.encode_utf8(&mut buf).as_bytes());
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        let end = bytes
            .get(*pos..)
            .map(|_rest| {
                let len = utf8_len(bytes[*pos]).unwrap_or(1);
                *pos + len
            })
            .ok_or(DecodeError::Truncated)?;
        let s = std::str::from_utf8(bytes.get(*pos..end).ok_or(DecodeError::Truncated)?)
            .map_err(|_| DecodeError::Mismatch("invalid utf-8 char"))?;
        let mut it = s.chars();
        let c = it.next().ok_or(DecodeError::Mismatch("empty char"))?;
        if it.next().is_some() {
            return Err(DecodeError::Mismatch("multi-char char"));
        }
        *pos = end;
        Ok(c)
    }
}

fn utf8_len(b: u8) -> Option<usize> {
    match b {
        0x00..=0x7f => Some(1),
        0xc0..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf7 => Some(4),
        _ => None,
    }
}

impl Wire for String {
    fn write(&self, out: &mut Vec<u8>) {
        (self.len() as u64).write(out);
        out.extend_from_slice(self.as_bytes());
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        let len = u64::read(bytes, pos)? as usize;
        let s = bytes.get(*pos..*pos + len).ok_or(DecodeError::Truncated)?;
        *pos += len;
        String::from_utf8(s.to_vec()).map_err(|_| DecodeError::Mismatch("invalid utf-8 string"))
    }
}

/// `PathBuf` encodes the same as a byte string (bincode serializes `PathBuf`
/// via `into_os_string`, which has the same len-prefixed layout on Unix).
impl Wire for std::path::PathBuf {
    fn write(&self, out: &mut Vec<u8>) {
        use std::os::unix::ffi::OsStrExt;
        let bytes = self.as_os_str().as_bytes();
        (bytes.len() as u64).write(out);
        out.extend_from_slice(bytes);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        use std::os::unix::ffi::OsStrExt;
        let len = u64::read(bytes, pos)? as usize;
        let s = bytes.get(*pos..*pos + len).ok_or(DecodeError::Truncated)?;
        *pos += len;
        let os = std::ffi::OsStr::from_bytes(s);
        Ok(std::path::PathBuf::from(os))
    }
}

// ---- collections ---------------------------------------------------------

fn write_len(len: usize, out: &mut Vec<u8>) {
    (len as u64).write(out);
}
fn read_len(bytes: &[u8], pos: &mut usize) -> Result<usize, DecodeError> {
    let len = u64::read(bytes, pos)?;
    // Guard against a prefix that would run past the buffer.
    if (*pos).saturating_add(len as usize) > bytes.len() {
        return Err(DecodeError::Truncated);
    }
    Ok(len as usize)
}

impl<T: Wire> Wire for Vec<T> {
    fn write(&self, out: &mut Vec<u8>) {
        write_len(self.len(), out);
        for item in self {
            item.write(out);
        }
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        let len = read_len(bytes, pos)?;
        let mut v = Vec::with_capacity(len);
        for _ in 0..len {
            v.push(T::read(bytes, pos)?);
        }
        Ok(v)
    }
}

impl<T: Wire> Wire for Option<T> {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Some(v) => {
                out.push(1);
                v.write(out);
            }
            None => out.push(0),
        }
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        match u8::read(bytes, pos)? {
            0 => Ok(None),
            1 => Ok(Some(T::read(bytes, pos)?)),
            _ => Err(DecodeError::InvalidTag("Option")),
        }
    }
}

impl<A: Wire, B: Wire> Wire for (A, B) {
    fn write(&self, out: &mut Vec<u8>) {
        self.0.write(out);
        self.1.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok((A::read(bytes, pos)?, B::read(bytes, pos)?))
    }
}

impl<A: Wire, B: Wire, C: Wire> Wire for (A, B, C) {
    fn write(&self, out: &mut Vec<u8>) {
        self.0.write(out);
        self.1.write(out);
        self.2.write(out);
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        Ok((
            A::read(bytes, pos)?,
            B::read(bytes, pos)?,
            C::read(bytes, pos)?,
        ))
    }
}

impl<T: Wire, const N: usize> Wire for [T; N] {
    fn write(&self, out: &mut Vec<u8>) {
        for item in self {
            item.write(out);
        }
    }
    fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, DecodeError> {
        // Build via a Vec then convert; `Self: Sized` array of T where
        // T: Wire is always constructible element-by-element.
        let mut v = Vec::with_capacity(N);
        for _ in 0..N {
            v.push(T::read(bytes, pos)?);
        }
        <[T; N]>::try_from(v).map_err(|_| DecodeError::Mismatch("array length"))
    }
}

// Macros to reduce boilerplate for the enum discriminant + struct impls.
#[macro_export]
macro_rules! impl_wire_enum {
    ($name:ident { $($variant:ident => $index:expr),* $(,)? }) => {
        impl $crate::codec::Wire for $name {
            fn write(&self, out: &mut Vec<u8>) {
                match self {
                    $(Self::$variant => { u32::write(&($index), out); }) *
                }
            }
            fn read(bytes: &[u8], pos: &mut usize) -> Result<Self, $crate::codec::DecodeError> {
                let d = u32::read(bytes, pos)?;
                match d {
                    $( $index => Ok(Self::$variant), )*
                    _ => Err($crate::codec::DecodeError::InvalidTag(stringify!($name))),
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_roundtrip() {
        assert_eq!(decode::<u32>(&encode(&42u32)).unwrap(), 42);
        assert!(decode::<bool>(&encode(&true)).unwrap());
        assert_eq!(
            decode::<String>(&encode(&"héllo".to_string())).unwrap(),
            "héllo"
        );
        assert_eq!(decode::<char>(&encode(&'界')).unwrap(), '界');
    }

    #[test]
    fn option_vec_array_roundtrip() {
        assert_eq!(
            decode::<Option<i32>>(&encode(&(None::<i32>))).unwrap(),
            None::<i32>
        );
        assert_eq!(
            decode::<Option<i32>>(&encode(&(Some(-7i32)))).unwrap(),
            Some(-7)
        );
        assert_eq!(
            decode::<Vec<u8>>(&encode(&vec![1u8, 2, 3])).unwrap(),
            vec![1u8, 2, 3]
        );
        let arr: [Option<(u8, u8, u8)>; 4] = [None, Some((1, 2, 3)), None, Some((4, 5, 6))];
        assert_eq!(
            decode::<[Option<(u8, u8, u8)>; 4]>(&encode(&arr)).unwrap(),
            arr,
        );
    }

    #[test]
    fn enum_discriminant_matches_bincode() {
        // bincode writes enum discriminants as a u32 LE; our wire width must
        // match so the golden byte layout stays identical.
        let mut buf = Vec::new();
        u32::write(&2, &mut buf);
        assert_eq!(buf, vec![2, 0, 0, 0]);
    }
}
