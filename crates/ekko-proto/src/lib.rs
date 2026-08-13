//! Wire protocol shared between the ekko client and server: versioned socket
//! paths, message types, and length-prefixed framing.

pub mod codec;
pub mod frame;
pub mod msg;
pub mod socket;

pub use codec::{DecodeError, Wire, decode, encode};

pub use frame::{FrameError, MAX_FRAME_SIZE, read_msg, write_msg};
pub use msg::*;
pub use socket::{
    IpcListener, WIRE_VERSION, decode_session_name, encode_session_name, ensure_socket_dir,
    ipc_bind, ipc_connect, is_socket, pid_path, socket_dir, socket_path,
};
