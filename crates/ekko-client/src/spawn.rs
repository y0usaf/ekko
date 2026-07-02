//! Spawns the per-session daemon (`ekko --server <name>`) and waits for its
//! socket to appear.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SPAWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Fork/exec `<current_exe> --server <session_name>`. The daemon
/// self-daemonizes (forks to the background and detaches), so this call
/// only needs to wait for the child to exit (it exits quickly once the
/// background server is up) — the actual liveness signal is the socket file
/// appearing, polled by [`wait_for_socket`].
pub fn spawn_daemon(session_name: &str) -> Result<()> {
    let exe = std::env::current_exe().context("resolving current executable")?;
    let status = std::process::Command::new(exe)
        .arg("--server")
        .arg(session_name)
        .status()
        .context("spawning session daemon")?;
    if !status.success() {
        bail!("session daemon exited with {status}");
    }
    Ok(())
}

/// Poll for `path` to appear, at [`POLL_INTERVAL`], up to [`SPAWN_TIMEOUT`].
pub fn wait_for_socket(path: &Path) -> Result<()> {
    let deadline = Instant::now() + SPAWN_TIMEOUT;
    while Instant::now() < deadline {
        if path.exists() {
            return Ok(());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    bail!(
        "timed out waiting for session socket to appear at {}",
        path.display()
    )
}
