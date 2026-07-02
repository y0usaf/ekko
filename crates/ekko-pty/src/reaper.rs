//! Per-child reaper thread, ported from zellij's `handle_command_exit`
//! (`zellij-server/src/os_input_output_unix.rs`).
//!
//! Watches a spawned child until it exits, forwarding SIGINT/SIGTERM through
//! to it and escalating to SIGKILL if it doesn't respond. Guarantees the
//! child is fully waited on (no zombie left behind) before returning,
//! regardless of which exit path is taken.

use std::process::Child;
use std::thread;
use std::time::Duration;

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

/// Poll interval while waiting for the child to exit.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Number of polite SIGTERM attempts before escalating to SIGKILL.
const SIGTERM_ATTEMPTS: u32 = 3;

/// Blocks the calling thread until `child` exits, returning its exit code
/// (`None` if it was killed by a signal). Always reaps the child before
/// returning.
pub(crate) fn reap_child(mut child: Child) -> Option<i32> {
    let pid = Pid::from_raw(child.id() as i32);

    let mut signals = match Signals::new([SIGINT, SIGTERM]) {
        Ok(signals) => signals,
        Err(e) => {
            log::error!(
                "reaper: failed to install signal handler for pid {pid}: {e}; \
                 falling back to a plain blocking wait"
            );
            return child.wait().ok().and_then(|status| status.code());
        }
    };

    let mut should_exit = false;
    let mut attempts_left = SIGTERM_ATTEMPTS;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code(),
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(e) => {
                log::error!("reaper: error waiting for pid {pid}: {e}");
                return None;
            }
        }

        if !should_exit {
            for signal in signals.pending() {
                if signal == SIGINT || signal == SIGTERM {
                    should_exit = true;
                }
            }
        } else if attempts_left > 0 {
            attempts_left -= 1;
            // Ask nicely first.
            let _ = kill(pid, Some(Signal::SIGTERM));
        } else {
            // When I say whoa, I mean WHOA! Send SIGKILL and block until the
            // kernel confirms the exit so we never leave a zombie behind.
            let _ = child.kill();
            return child.wait().ok().and_then(|status| status.code());
        }
    }
}
