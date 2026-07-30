//! ekko-paths: the single owner of ekko's on-disk locations.
//!
//! Every XDG/env resolution in the workspace funnels through this crate so
//! the client, daemon, and CLI always agree on where things live:
//!
//! - **cache root**: `EKKO_CACHE_DIR` → `$XDG_CACHE_HOME/ekko` → `~/.cache/ekko`
//! - **config dir**: `$XDG_CONFIG_HOME/ekko` → `~/.config/ekko`
//! - **socket dir**: `EKKO_SOCKET_DIR` → `$XDG_RUNTIME_DIR/ekko/wire_v<N>` →
//!   `$TMPDIR/ekko-<uid>/wire_v<N>` (owned here as `runtime_root`; the
//!   versioned leaf is appended by `ekko-proto`, which owns `WIRE_VERSION`)
//!
//! All functions are pure resolution — they never touch the filesystem.

use std::path::PathBuf;

/// Root cache directory for ekko (logs, resurrection manifests).
///
/// `EKKO_CACHE_DIR` wins (tests use it for a hermetic location), then
/// `$XDG_CACHE_HOME/ekko`, then `~/.cache/ekko`.
pub fn cache_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("EKKO_CACHE_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.cache_dir().join("ekko"))
        .unwrap_or_else(|| PathBuf::from(".cache/ekko"))
}

/// Daemon log directory: `<cache_root>/logs`.
pub fn log_dir() -> PathBuf {
    cache_root().join("logs")
}

/// Config directory: `$XDG_CONFIG_HOME/ekko` (or `~/.config/ekko`).
pub fn config_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("ekko"))
        .unwrap_or_else(|| PathBuf::from(".config/ekko"))
}

/// Root runtime directory for unix sockets, before the `wire_v<N>` leaf
/// `ekko-proto` appends: `EKKO_SOCKET_DIR` (test override, returned as-is),
/// then `$XDG_RUNTIME_DIR/ekko`, then `$TMPDIR/ekko-<uid>` (`/tmp` fallback).
pub fn runtime_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("EKKO_SOCKET_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR")
        && !runtime_dir.is_empty()
    {
        return PathBuf::from(runtime_dir).join("ekko");
    }
    let tmp_dir = std::env::var_os("TMPDIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    tmp_dir.join(format!("ekko-{}", current_uid()))
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid() takes no arguments and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_are_stable_per_process() {
        assert_eq!(cache_root(), cache_root());
        assert_eq!(config_dir(), config_dir());
        assert_eq!(runtime_root(), runtime_root());
    }

    #[test]
    fn log_dir_lives_under_cache_root() {
        assert!(log_dir().starts_with(cache_root()));
    }
}
