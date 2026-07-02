//! Dedicated PTY-writer thread.
//!
//! Writing to the PTY happens on its own thread (separate from the reader)
//! because some programs deadlock if you write to their stdin while also
//! reading their stdout from the same thread's call stack (ported intent
//! from zellij's `pty_writer.rs`, extracted via grep rather than reading the
//! whole file).
//!
//! Two responsibilities:
//! - Buffer and drain writes (`Key`/`Paste` bytes), tolerating partial writes
//!   and `EAGAIN`, with a bounded pending-bytes cap so a stuck child can't
//!   grow memory without limit.
//! - Coalesce resizes: `TIOCSWINSZ` ioctls arriving faster than the terminal
//!   can usefully redraw are batched into at most one per debounce window,
//!   keeping only the most recent size (ported intent from zellij's
//!   `StartCachingResizes`/`ApplyCachedResizes`).

use std::collections::VecDeque;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError};

/// Maximum total bytes buffered for the PTY before we give up and drop the
/// queue (logging loudly). Matches the order of magnitude zellij uses for
/// its per-terminal write buffers.
const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;

/// Only apply one resize per this window, keeping the latest.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);

/// Poll interval while there's pending work (writes to drain or a resize
/// debounce to expire).
const DRAIN_TIMEOUT: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub enum PtyWriterInstruction {
    Write(Vec<u8>),
    Resize(u16, u16),
    Shutdown,
}

struct PendingWrite {
    bytes: Vec<u8>,
    offset: usize,
}

/// Runs until `Shutdown` is received or the channel disconnects. Intended to
/// be the body of a dedicated `pty-writer` thread.
pub fn run(rx: &Receiver<PtyWriterInstruction>, fd: RawFd) {
    let mut queue: VecDeque<PendingWrite> = VecDeque::new();
    let mut queued_bytes: usize = 0;
    let mut pending_resize: Option<(u16, u16)> = None;
    let mut resize_pending_since: Option<Instant> = None;

    loop {
        let has_pending = !queue.is_empty() || pending_resize.is_some();
        let event = if has_pending {
            match rx.recv_timeout(DRAIN_TIMEOUT) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(event) => Some(event),
                Err(_) => return,
            }
        };

        match event {
            Some(PtyWriterInstruction::Write(bytes)) => {
                if queued_bytes.saturating_add(bytes.len()) > MAX_PENDING_BYTES {
                    log::error!(
                        "pty-writer: dropping {} queued bytes, buffer exceeded {} byte cap",
                        queued_bytes,
                        MAX_PENDING_BYTES
                    );
                    queue.clear();
                    queued_bytes = 0;
                } else {
                    queued_bytes += bytes.len();
                    queue.push_back(PendingWrite { bytes, offset: 0 });
                }
            }
            Some(PtyWriterInstruction::Resize(cols, rows)) => {
                pending_resize = Some((cols, rows));
                resize_pending_since.get_or_insert_with(Instant::now);
            }
            Some(PtyWriterInstruction::Shutdown) => return,
            None => {}
        }

        if let Some(since) = resize_pending_since
            && since.elapsed() >= RESIZE_DEBOUNCE
        {
            if let Some((cols, rows)) = pending_resize.take()
                && let Err(e) = ekko_pty::resize(fd, cols, rows)
            {
                log::warn!("pty-writer: resize to {cols}x{rows} failed: {e}");
            }
            resize_pending_since = None;
        }

        drain_writes(fd, &mut queue, &mut queued_bytes);
    }
}

fn drain_writes(fd: RawFd, queue: &mut VecDeque<PendingWrite>, queued_bytes: &mut usize) {
    while let Some(front) = queue.front_mut() {
        let remaining = &front.bytes[front.offset..];
        if remaining.is_empty() {
            queue.pop_front();
            continue;
        }
        match ekko_pty::try_write_to_fd(fd, remaining) {
            Ok(0) => break, // EAGAIN: kernel buffer full, try again next loop
            Ok(n) => {
                *queued_bytes = queued_bytes.saturating_sub(n);
                front.offset += n;
                if front.offset >= front.bytes.len() {
                    queue.pop_front();
                }
            }
            Err(e) => {
                log::warn!("pty-writer: write error, dropping queued bytes: {e}");
                queue.clear();
                *queued_bytes = 0;
                break;
            }
        }
    }
}
