//! Low-level, partial-write-tolerant I/O helpers for PTY master file
//! descriptors. Ported from zellij's `try_write_to_fd`
//! (`zellij-server/src/os_input_output_unix.rs`).

use std::io;
use std::os::fd::{BorrowedFd, RawFd};

use nix::fcntl::{FcntlArg, OFlag, fcntl};

use crate::PtyError;

/// Try to write as many bytes from `buf` as possible to `fd`.
///
/// Loops on successful short writes and `EINTR` to drain as much as the
/// kernel will accept. If `fd` is non-blocking and the kernel buffer fills
/// up (`EAGAIN`), stops and returns how many bytes were written so far
/// (which may be 0) rather than erroring — the caller is expected to
/// re-queue any unwritten remainder. If `fd` is blocking, this simply writes
/// the whole buffer.
pub fn try_write_to_fd(fd: RawFd, buf: &[u8]) -> Result<usize, PtyError> {
    // SAFETY: `fd` is borrowed for the duration of this call only; the
    // caller retains ownership of the underlying file descriptor.
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    let mut written = 0;
    while written < buf.len() {
        match nix::unistd::write(borrowed, &buf[written..]) {
            Ok(0) => break, // fd returned 0 on a non-empty buf; treat like EAGAIN
            Ok(n) => written += n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(nix::errno::Errno::EAGAIN) => break,
            Err(e) => return Err(PtyError::Nix(e)),
        }
    }
    Ok(written)
}

/// Read from `fd` into `buf`, retrying on `EINTR`. Blocks or not depending
/// on whether `fd` currently has `O_NONBLOCK` set.
pub fn read(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        // SAFETY: `buf` is a valid, writable slice for its full length, and
        // `fd` is a valid file descriptor owned by the caller.
        let ret = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        return Ok(ret as usize);
    }
}

/// Set or clear `O_NONBLOCK` on `fd`.
///
/// The server wraps the PTY master fd in a `tokio::io::unix::AsyncFd` for
/// async reads, which requires the fd to be non-blocking; this helper lets
/// callers flip that on (or off, for plain synchronous use) without ekko-pty
/// needing to know about tokio at all.
pub fn set_nonblocking(fd: RawFd, nonblocking: bool) -> Result<(), PtyError> {
    let flags = fcntl(fd, FcntlArg::F_GETFL).map_err(PtyError::Nix)?;
    let mut oflags = OFlag::from_bits_truncate(flags);
    oflags.set(OFlag::O_NONBLOCK, nonblocking);
    fcntl(fd, FcntlArg::F_SETFL(oflags)).map_err(PtyError::Nix)?;
    Ok(())
}
