//! Length-prefixed bincode framing for messages sent over the IPC socket.
//!
//! Each frame is a little-endian `u32` byte length followed by that many
//! bincode-serialized bytes. Frames larger than [`MAX_FRAME_SIZE`] are
//! rejected on both the write and read side.

use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

/// Maximum allowed frame payload size (16 MiB), guarding against a corrupt or
/// malicious length prefix causing an unbounded allocation.
pub const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

/// Errors that can occur while writing or reading a framed message.
#[derive(Debug, Error)]
pub enum FrameError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("frame of {size} bytes exceeds the maximum of {max} bytes")]
    FrameTooLarge { size: u32, max: u32 },
    #[error("bincode encode/decode error: {0}")]
    Bincode(#[from] bincode::Error),
    /// The connection was closed in the middle of a frame (after the length
    /// prefix or a partial payload had already been read). Distinguishable
    /// from a clean EOF, which is reported by returning `Ok(None)` from
    /// [`read_msg`].
    #[error("connection closed mid-frame (truncated)")]
    Truncated,
}

/// Write `msg` to `writer` as a single length-prefixed frame.
pub fn write_msg<W: Write, T: Serialize>(writer: &mut W, msg: &T) -> Result<(), FrameError> {
    let bytes = bincode::serialize(msg)?;
    if bytes.len() > MAX_FRAME_SIZE as usize {
        return Err(FrameError::FrameTooLarge {
            size: bytes.len() as u32,
            max: MAX_FRAME_SIZE,
        });
    }
    let len = bytes.len() as u32;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

/// Read one length-prefixed frame from `reader` and deserialize it as `T`.
///
/// Returns `Ok(None)` on a clean EOF (the peer closed the connection between
/// frames, with zero bytes of the next frame read). Returns
/// `Err(FrameError::Truncated)` if the connection closes partway through a
/// frame, which indicates a corrupted stream rather than an orderly
/// disconnect.
pub fn read_msg<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<Option<T>, FrameError> {
    let mut len_buf = [0u8; 4];
    if !read_fully_or_eof(reader, &mut len_buf)? {
        return Ok(None);
    }
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_SIZE {
        return Err(FrameError::FrameTooLarge {
            size: len,
            max: MAX_FRAME_SIZE,
        });
    }
    let mut buf = vec![0u8; len as usize];
    if !read_fully_or_eof(reader, &mut buf)? {
        return Err(FrameError::Truncated);
    }
    let msg = bincode::deserialize(&buf)?;
    Ok(Some(msg))
}

/// Fill `buf` completely from `reader`. Returns `Ok(true)` if `buf` was
/// filled, `Ok(false)` if the reader hit EOF before any bytes were read
/// (a clean disconnect). Returns `Err(FrameError::Truncated)` if EOF is hit
/// after some but not all bytes were read.
fn read_fully_or_eof<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<bool, FrameError> {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..]) {
            Ok(0) => {
                return if total == 0 {
                    Ok(false)
                } else {
                    Err(FrameError::Truncated)
                };
            }
            Ok(n) => total += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::*;
    use std::io::Cursor;

    fn roundtrip<T>(msg: T)
    where
        T: Serialize + DeserializeOwned + std::fmt::Debug + PartialEq,
    {
        let mut buf = Vec::new();
        write_msg(&mut buf, &msg).expect("write");
        let mut cursor = Cursor::new(buf);
        let decoded: Option<T> = read_msg(&mut cursor).expect("read");
        assert_eq!(decoded, Some(msg));
    }

    #[test]
    fn roundtrip_client_to_server_variants() {
        roundtrip(ClientToServer::Attach {
            wire_version: 1,
            cols: 80,
            rows: 24,
            cwd: "/home/user".into(),
            shell: Some("/bin/bash".into()),
            force: false,
            terminal_colors: Some(TerminalColors {
                background: (0x1e, 0x1e, 0x2e),
                foreground: (0xcd, 0xd6, 0xf4),
                palette: {
                    let mut palette = [None; 16];
                    palette[1] = Some((0xf3, 0x8b, 0xa8));
                    palette
                },
            }),
        });
        roundtrip(ClientToServer::Detach);
        roundtrip(ClientToServer::Resize {
            cols: 120,
            rows: 40,
        });
        roundtrip(ClientToServer::Key(vec![0x1b, b'[', b'A']));
        roundtrip(ClientToServer::Paste(b"hello world".to_vec()));
        roundtrip(ClientToServer::Scroll { delta: -3 });
        roundtrip(ClientToServer::ScrollReset);
        roundtrip(ClientToServer::KillCurrentSession);
        roundtrip(ClientToServer::KillSession("main".into()));
        roundtrip(ClientToServer::Ping);
    }

    #[test]
    fn roundtrip_server_to_client_variants() {
        roundtrip(ServerToClient::Attached {
            session_name: "main".into(),
            wire_version: 1,
        });
        roundtrip(ServerToClient::AttachRejected(
            AttachRejectReason::WrongWireVersion,
        ));
        roundtrip(ServerToClient::AttachRejected(
            AttachRejectReason::SpawnFailed("boom".into()),
        ));
        roundtrip(ServerToClient::Grid(GridUpdate {
            epoch: 42,
            cols: 80,
            rows: 24,
            cursor: Some(CursorState {
                row: 1,
                col: 2,
                visible: true,
                shape: 6,
            }),
            modes: TermModes {
                alt_screen: true,
                app_cursor: true,
                mouse_mode: MouseMode::ButtonMotion,
                mouse_encoding: MouseEncoding::Sgr,
                focus_reporting: true,
            },
            scrollback: 0,
            payload: GridPayload::Full(vec![GridRow {
                cells: vec![GridCell {
                    ch: 'x',
                    extra: vec!['\u{0301}'],
                    fg: WireColor::Default,
                    bg: WireColor::Rgb(1, 2, 3),
                    attrs: GridCell::BOLD | GridCell::UNDERLINE,
                }],
            }]),
        }));
        roundtrip(ServerToClient::Grid(GridUpdate {
            epoch: 43,
            cols: 80,
            rows: 24,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 120,
            payload: GridPayload::Rows(vec![(
                3,
                GridRow {
                    cells: vec![GridCell {
                        ch: ' ',
                        extra: Vec::new(),
                        fg: WireColor::Indexed(5),
                        bg: WireColor::Default,
                        attrs: 0,
                    }],
                },
            )]),
        }));
        roundtrip(ServerToClient::Bell);
        roundtrip(ServerToClient::Exit(ExitReason::Normal));
        roundtrip(ServerToClient::Exit(ExitReason::Detached));
        roundtrip(ServerToClient::Exit(ExitReason::Kicked));
        roundtrip(ServerToClient::Exit(ExitReason::SessionExited(Some(0))));
        roundtrip(ServerToClient::Exit(ExitReason::ServerError("oops".into())));
        roundtrip(ServerToClient::Pong);
        roundtrip(ServerToClient::Title("vim ~/notes.md".into()));
        roundtrip(ServerToClient::ClipboardCopy(b"aGVsbG8=".to_vec()));
    }

    #[test]
    fn rejects_oversized_frame_on_read() {
        let mut buf = Vec::new();
        let bad_len = MAX_FRAME_SIZE + 1;
        buf.extend_from_slice(&bad_len.to_le_bytes());
        let mut cursor = Cursor::new(buf);
        let result: Result<Option<ClientToServer>, FrameError> = read_msg(&mut cursor);
        assert!(matches!(result, Err(FrameError::FrameTooLarge { .. })));
    }

    #[test]
    fn rejects_oversized_frame_on_write() {
        // Construct a message whose serialized form exceeds MAX_FRAME_SIZE.
        let huge = ClientToServer::Paste(vec![0u8; MAX_FRAME_SIZE as usize + 1]);
        let mut buf = Vec::new();
        let result = write_msg(&mut buf, &huge);
        assert!(matches!(result, Err(FrameError::FrameTooLarge { .. })));
    }

    #[test]
    fn clean_eof_returns_none() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let result: Option<ClientToServer> = read_msg(&mut cursor).expect("read should not error");
        assert!(result.is_none());
    }

    #[test]
    fn truncated_frame_is_an_error_not_none() {
        // Write a valid length prefix but only part of the payload.
        let mut buf = Vec::new();
        let len: u32 = 100;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&[0u8; 10]); // far short of 100 bytes
        let mut cursor = Cursor::new(buf);
        let result: Result<Option<ClientToServer>, FrameError> = read_msg(&mut cursor);
        assert!(matches!(result, Err(FrameError::Truncated)));
    }

    #[test]
    fn truncated_length_prefix_is_an_error_not_none() {
        // Only 2 of the 4 length-prefix bytes are present.
        let buf = vec![0x01u8, 0x02];
        let mut cursor = Cursor::new(buf);
        let result: Result<Option<ClientToServer>, FrameError> = read_msg(&mut cursor);
        assert!(matches!(result, Err(FrameError::Truncated)));
    }
}
