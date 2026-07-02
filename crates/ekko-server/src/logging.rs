//! Minimal file-backed `log` backend, plus the log file used to redirect
//! stdout/stderr when daemonizing. Both point at the same path:
//! `<cache_dir>/logs/<session_name>.log`.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};

use ekko_resurrection::cache_root;

fn log_dir() -> PathBuf {
    cache_root().join("logs")
}

fn log_path(session_name: &str) -> PathBuf {
    log_dir().join(format!("{session_name}.log"))
}

struct FileLogger {
    file: Mutex<File>,
}

impl Log for FileLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if record.level() > Level::Info && !cfg!(debug_assertions) {
            return;
        }
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(
                file,
                "[{secs}] {:>5} {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
            let _ = file.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

/// Open (creating parent dirs as needed) the log file for `session_name` and
/// install a process-wide `log` backend writing to it.
///
/// Safe to call more than once per process (e.g. from repeated in-process
/// test runs); a logger can only be installed once, so later calls just
/// leave the first one in place.
pub fn init(session_name: &str) -> anyhow::Result<PathBuf> {
    let path = open_new(session_name)?;
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let logger = Box::new(FileLogger {
        file: Mutex::new(file),
    });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(LevelFilter::Trace);
    }
    Ok(path)
}

/// Open a fresh handle on the same log file, suitable for handing to
/// `daemonize::Stdio::from` so the daemonized process's stdout/stderr land
/// in the same file as the structured log output.
pub fn open_redirect_file(session_name: &str) -> anyhow::Result<File> {
    let path = open_new(session_name)?;
    Ok(OpenOptions::new().create(true).append(true).open(path)?)
}

fn open_new(session_name: &str) -> anyhow::Result<PathBuf> {
    let dir = log_dir();
    fs::create_dir_all(&dir)?;
    Ok(log_path(session_name))
}
