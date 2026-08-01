//! Low-level, partial-write-tolerant I/O helpers for PTY master file
//! descriptors. Ported from zellij's `try_write_to_fd`
//! (`zellij-server/src/os_input_output_unix.rs`).

use std::io;
use std::os::fd::RawFd;

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
    let mut written = 0;
    while written < buf.len() {
        // SAFETY: fd and slice are valid for this call.
        let ret = unsafe { libc::write(fd, buf[written..].as_ptr().cast(), buf.len() - written) };
        if ret == 0 {
            break;
        }
        if ret > 0 {
            written += ret as usize;
            continue;
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EAGAIN) => break,
            _ => return Err(PtyError::Io(error)),
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
/// Configure whether reads and writes on the PTY master should block. The
/// non-blocking mode lets the synchronous event loop poll the descriptor
/// without stalling other work.
pub fn set_nonblocking(fd: RawFd, nonblocking: bool) -> Result<(), PtyError> {
    // SAFETY: fcntl operates on the caller-owned descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(PtyError::Io(io::Error::last_os_error()));
    }
    let new_flags = if nonblocking {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };
    if unsafe { libc::fcntl(fd, libc::F_SETFL, new_flags) } == -1 {
        return Err(PtyError::Io(io::Error::last_os_error()));
    }
    Ok(())
}
