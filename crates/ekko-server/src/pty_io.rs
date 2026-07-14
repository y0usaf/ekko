//! PTY reader thread: non-blocking reads from one pane's master fd,
//! forwarded to the hub as identity-tagged bytes for that pane's parser.

use std::io::ErrorKind;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crossbeam_channel::Sender;

use crate::hub::HubInstruction;
use crate::terminal_pane::PaneKey;

/// Maximum bytes in flight between one pane reader and the hub's parser.
/// Each pane owns a separate [`PtyBacklog`].
const MAX_BACKLOG_BYTES: usize = 4 * 1024 * 1024;

/// How long to wait for the hub to drain or for a non-blocking fd to become
/// readable before checking whether this pane was retired.
const BACKLOG_POLL: Duration = Duration::from_millis(1);

/// One pane's read-direction flow-control counter.
#[derive(Clone, Default)]
pub(crate) struct PtyBacklog {
    bytes: Arc<AtomicUsize>,
}

impl PtyBacklog {
    fn add(&self, bytes: usize) {
        self.bytes.fetch_add(bytes, Ordering::Release);
    }

    pub(crate) fn release(&self, bytes: usize) {
        // Saturation keeps cleanup/test seams safe if an event is discarded;
        // the production reader still reserves exactly once per event.
        let _ = self
            .bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_sub(bytes))
            });
    }

    fn is_at_cap(&self) -> bool {
        self.bytes.load(Ordering::Acquire) >= MAX_BACKLOG_BYTES
    }

    #[cfg(test)]
    pub(crate) fn reserve_for_test(&self, bytes: usize) {
        self.add(bytes);
    }
}

/// Runs as one pane's dedicated reader until retirement, EOF, or an
/// unrecoverable read error. `fd` must be non-blocking so retirement can join
/// this thread deterministically.
pub(crate) fn run(
    fd: RawFd,
    hub_tx: &Sender<HubInstruction>,
    backlog: &PtyBacklog,
    retired: &AtomicBool,
    pane: PaneKey,
) {
    let mut buf = [0u8; 64 * 1024];
    while !retired.load(Ordering::Acquire) {
        while backlog.is_at_cap() && !retired.load(Ordering::Acquire) {
            std::thread::sleep(BACKLOG_POLL);
        }
        if retired.load(Ordering::Acquire) {
            return;
        }

        match ekko_pty::read(fd, &mut buf) {
            Ok(0) => return,
            Ok(n) => {
                backlog.add(n);
                if hub_tx
                    .send(HubInstruction::PtyBytes {
                        pane,
                        bytes: buf[..n].to_vec(),
                    })
                    .is_err()
                {
                    backlog.release(n);
                    return;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(BACKLOG_POLL);
            }
            Err(e) => {
                log::debug!("pty-reader[{pane:?}]: read error, treating as EOF: {e}");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_backlog_cap_is_per_pane() {
        let first = PtyBacklog::default();
        let second = PtyBacklog::default();

        first.add(MAX_BACKLOG_BYTES);
        assert!(first.is_at_cap());
        assert!(!second.is_at_cap());

        second.add(MAX_BACKLOG_BYTES - 1);
        assert!(!second.is_at_cap());
        first.release(MAX_BACKLOG_BYTES);
        assert!(!first.is_at_cap());
    }
}
