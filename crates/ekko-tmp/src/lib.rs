//! Minimal temporary-directory support for tests.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);
const MAX_ATTEMPTS: u64 = 32;

/// A uniquely-created temporary directory removed when dropped.
pub struct TempDir {
    path: PathBuf,
}

/// Create a temporary directory.
pub fn tempdir() -> io::Result<TempDir> {
    TempDir::new()
}

impl TempDir {
    /// Create a temporary directory, retrying atomically on collisions.
    pub fn new() -> io::Result<Self> {
        let root = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        for _ in 0..MAX_ATTEMPTS {
            let count = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!("ekko-{pid}-{nanos}-{count}"));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique temporary directory",
        ))
    }

    /// Return the temporary directory's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
