//! PTY reader thread: blocking reads from the master fd, forwarded to the
//! hub as raw bytes for the vt100 parser to consume.
//!
//! A single fd's worth of I/O doesn't need an async runtime — a plain thread
//! blocked in `read(2)` is simpler and exactly as fast.

use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crossbeam_channel::Sender;

use crate::hub::HubInstruction;

/// Maximum bytes in flight between this reader and the hub's parser. The
/// mirror of `pty_writer`'s cap on the opposite direction: while the hub is
/// behind, the reader stalls, the kernel PTY buffer fills, and the flooding
/// child blocks in `write(2)` — bounded memory instead of an unbounded
/// instruction queue.
const MAX_BACKLOG_BYTES: usize = 4 * 1024 * 1024;

/// How long to wait for the hub to drain before re-checking the backlog.
const BACKLOG_POLL: Duration = Duration::from_millis(1);

/// Blocks reading from `fd` until EOF (the shell exited and the last fd
/// referencing the PTY slave was closed) or an unrecoverable read error.
/// Runs as the body of a dedicated `pty-reader` thread. `backlog` counts
/// bytes sent but not yet parsed; the hub decrements it.
pub fn run(fd: RawFd, hub_tx: &Sender<HubInstruction>, backlog: &Arc<AtomicUsize>) {
    let mut buf = [0u8; 64 * 1024];
    loop {
        match ekko_pty::read(fd, &mut buf) {
            Ok(0) => return, // EOF
            Ok(n) => {
                while backlog.load(Ordering::Acquire) > MAX_BACKLOG_BYTES {
                    std::thread::sleep(BACKLOG_POLL);
                }
                backlog.fetch_add(n, Ordering::Release);
                if hub_tx
                    .send(HubInstruction::PtyBytes(buf[..n].to_vec()))
                    .is_err()
                {
                    return; // hub gone; nothing left to do
                }
            }
            Err(e) => {
                log::debug!("pty-reader: read error, treating as EOF: {e}");
                return;
            }
        }
    }
}
