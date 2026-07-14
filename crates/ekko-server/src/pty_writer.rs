//! Dedicated per-pane PTY-writer thread.
//!
//! Writing happens separately from reading because some programs deadlock if
//! both directions share one call stack. Each writer owns its own bounded
//! pending-byte queue and coalesces resize floods to the latest dimensions.

use std::collections::VecDeque;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError};

/// Maximum bytes buffered for one pane's PTY.
const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;

/// Only apply one resize per this window, keeping the latest.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);

/// Poll interval while there's pending work.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub(crate) enum PtyWriterInstruction {
    Write(Vec<u8>),
    Resize(u16, u16),
    Shutdown,
}

struct PendingWrite {
    bytes: Vec<u8>,
    offset: usize,
}

#[derive(Default)]
struct PendingWrites {
    queue: VecDeque<PendingWrite>,
    queued_bytes: usize,
}

impl PendingWrites {
    fn push(&mut self, bytes: Vec<u8>) {
        if self.queued_bytes.saturating_add(bytes.len()) > MAX_PENDING_BYTES {
            log::error!(
                "pty-writer: dropping {} queued bytes, buffer exceeded {} byte cap",
                self.queued_bytes,
                MAX_PENDING_BYTES
            );
            self.queue.clear();
            self.queued_bytes = 0;
        } else {
            self.queued_bytes += bytes.len();
            self.queue.push_back(PendingWrite { bytes, offset: 0 });
        }
    }

    fn clear(&mut self) {
        self.queue.clear();
        self.queued_bytes = 0;
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Runs until `Shutdown`, retirement, or channel disconnection.
pub(crate) fn run(rx: &Receiver<PtyWriterInstruction>, fd: RawFd, retired: &AtomicBool) {
    let mut writes = PendingWrites::default();
    let mut pending_resize: Option<(u16, u16)> = None;
    let mut resize_pending_since: Option<Instant> = None;

    while !retired.load(Ordering::Acquire) {
        let has_pending = !writes.is_empty() || pending_resize.is_some();
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
            Some(PtyWriterInstruction::Write(bytes)) => writes.push(bytes),
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

        drain_writes(fd, &mut writes);
    }
}

fn drain_writes(fd: RawFd, writes: &mut PendingWrites) {
    while let Some(front) = writes.queue.front_mut() {
        let remaining = &front.bytes[front.offset..];
        if remaining.is_empty() {
            writes.queue.pop_front();
            continue;
        }
        match ekko_pty::try_write_to_fd(fd, remaining) {
            Ok(0) => break,
            Ok(n) => {
                writes.queued_bytes = writes.queued_bytes.saturating_sub(n);
                front.offset += n;
                if front.offset >= front.bytes.len() {
                    writes.queue.pop_front();
                }
            }
            Err(e) => {
                log::warn!("pty-writer: write error, dropping queued bytes: {e}");
                writes.clear();
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_write_cap_is_per_pane() {
        let mut first = PendingWrites::default();
        let mut second = PendingWrites::default();

        first.push(vec![0; MAX_PENDING_BYTES]);
        second.push(vec![1; 4]);
        first.push(vec![2]);

        assert_eq!(first.queued_bytes, 0, "overflow retires only this queue");
        assert_eq!(second.queued_bytes, 4, "another pane keeps its queue");
        assert_eq!(second.queue.len(), 1);
    }
}
