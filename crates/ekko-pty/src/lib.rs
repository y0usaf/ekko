//! PTY spawning, process reaping, and low-level PTY I/O for ekko.
//!
//! Ported from zellij's unix PTY backend
//! (`zellij-server/src/os_input_output_unix.rs`).

mod io;
mod reaper;
mod spawn;

pub use io::{read, set_nonblocking, try_write_to_fd};
pub use spawn::{PtyCommand, PtyError, PtyHandle, WinSize, force_kill, kill, resize, spawn_pty};

pub use nix::unistd::Pid;

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::termios;
    use std::io::Read as _;
    use std::os::fd::AsRawFd;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Spawn `/bin/sh -c 'printf hi'`, read its output from the master until
    /// EOF, and confirm the reaper thread fires `on_exit` with exit code 0
    /// and leaves no zombie behind.
    #[test]
    fn spawn_reads_output_and_reaps_without_zombie() {
        let (tx, rx) = mpsc::channel();
        let cmd = PtyCommand::new("/bin/sh").arg("-c").arg("printf hi");
        let handle = spawn_pty(
            cmd,
            WinSize { cols: 80, rows: 24 },
            Box::new(move |code| {
                let _ = tx.send(code);
            }),
        )
        .expect("spawn_pty failed");

        let pid = handle.child_pid;
        let mut master = std::fs::File::from(handle.master_fd);
        let mut output = Vec::new();
        // The pty master reports EOF once the shell exits and the last fd
        // referencing the slave side (duped by login_tty) is closed.
        let _ = master.read_to_end(&mut output);
        assert_eq!(String::from_utf8_lossy(&output), "hi");

        let exit_code = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("on_exit callback did not fire");
        assert_eq!(exit_code, Some(0));

        // No zombie: signal 0 should now fail with ESRCH since the child has
        // been fully waited on.
        let err = nix::sys::signal::kill(pid, None).expect_err("child should be fully reaped");
        assert_eq!(err, nix::errno::Errno::ESRCH);
    }

    /// Ported from zellij's `try_write_to_fd_returns_partial_on_full_buffer`
    /// (`zellij-server/src/os_input_output_unix.rs`): verify that
    /// `try_write_to_fd` returns a partial byte count rather than an error
    /// when a non-blocking master fd's buffer fills up.
    #[test]
    fn try_write_to_fd_partial_write_on_full_buffer() {
        let pty = nix::pty::openpty(None::<&nix::pty::Winsize>, None::<&termios::Termios>)
            .expect("openpty failed");
        let master_fd = pty.master.as_raw_fd();

        let mut attrs = termios::tcgetattr(&pty.slave).expect("tcgetattr failed");
        termios::cfmakeraw(&mut attrs);
        termios::tcsetattr(&pty.slave, termios::SetArg::TCSANOW, &attrs).expect("tcsetattr failed");

        set_nonblocking(master_fd, true).expect("set_nonblocking failed");

        let mut slave_file = std::fs::File::from(pty.slave);

        // Fill most of the buffer, leaving some space.
        let chunk = vec![0x42u8; 1024];
        let mut total_filled = 0;
        loop {
            match try_write_to_fd(master_fd, &chunk) {
                Ok(0) => break,
                Ok(n) => total_filled += n,
                Err(e) => panic!("unexpected error filling buffer: {e}"),
            }
        }
        assert!(
            total_filled > 0,
            "should have written some bytes to fill buffer"
        );

        // Read a small amount from the slave to free partial space.
        let mut drain = vec![0u8; 512];
        let drained = slave_file.read(&mut drain).expect("slave read failed");
        assert!(drained > 0, "should have drained some bytes");

        // Now write more than the freed space — should get a partial write.
        let size = 128 * 1024;
        let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
        let written =
            try_write_to_fd(master_fd, &data).expect("try_write_to_fd should not error on EAGAIN");

        assert!(
            written > 0 && written < size,
            "expected partial write, got {written}/{size}",
        );

        let _ = pty.master; // keep the master fd alive until here
    }

    /// Ported from zellij's `try_write_to_fd_returns_zero_on_stuck_pty`:
    /// once the buffer is completely full, further writes return `Ok(0)`
    /// rather than an error.
    #[test]
    fn try_write_to_fd_returns_zero_on_stuck_pty() {
        let pty = nix::pty::openpty(None::<&nix::pty::Winsize>, None::<&termios::Termios>)
            .expect("openpty failed");
        let master_fd = pty.master.as_raw_fd();

        let mut attrs = termios::tcgetattr(&pty.slave).expect("tcgetattr failed");
        termios::cfmakeraw(&mut attrs);
        termios::tcsetattr(&pty.slave, termios::SetArg::TCSANOW, &attrs).expect("tcsetattr failed");

        set_nonblocking(master_fd, true).expect("set_nonblocking failed");

        let fill = vec![0x42u8; 1024];
        loop {
            match try_write_to_fd(master_fd, &fill) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(e) => panic!("unexpected error filling buffer: {e}"),
            }
        }

        let written = try_write_to_fd(master_fd, &[0x01, 0x02, 0x03])
            .expect("try_write_to_fd should not error on EAGAIN");
        assert_eq!(written, 0, "expected zero bytes written on full buffer");

        let _ = pty.slave; // keep the slave side alive until here
    }

    #[test]
    fn resize_does_not_error_on_live_pty() {
        let pty = nix::pty::openpty(None::<&nix::pty::Winsize>, None::<&termios::Termios>)
            .expect("openpty failed");
        let master_fd = pty.master.as_raw_fd();
        resize(master_fd, 100, 40).expect("resize should not fail on a live pty");
        let _ = (pty.master, pty.slave);
    }
}
