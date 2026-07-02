//! Wire protocol shared between the ekko client and server: versioned socket
//! paths, message types, and length-prefixed framing.

pub mod frame;
pub mod msg;
pub mod socket;

pub use frame::{FrameError, MAX_FRAME_SIZE, read_msg, write_msg};
pub use msg::*;
pub use socket::{
    WIRE_VERSION, decode_session_name, encode_session_name, ensure_socket_dir, ipc_bind,
    ipc_connect, socket_dir, socket_path,
};
