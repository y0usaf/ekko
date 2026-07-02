//! On-disk session manifests, used by `ekko ls` to show detached/resurrectable
//! sessions after their daemon has exited or crashed.
//!
//! Manifests live at `<cache_dir>/wire_v<N>/session_info/<session>/manifest.json`.
//! `<cache_dir>` defaults to `~/.cache/ekko` (via `directories::BaseDirs`) and can
//! be overridden with `EKKO_CACHE_DIR` so tests get a hermetic location.
//!
//! This is a plain I/O library, deliberately host-agnostic: the daemon-side
//! *policy* of when manifests are written lives in
//! `ekko-builtins::resurrection` (an extension subscribing to session
//! lifecycle events), while the read/list side is called directly by
//! `ekko ls`, which runs with no daemon or extension runtime at all.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use ekko_proto::{SessionStatus, SessionSummary};
use serde::{Deserialize, Serialize};

/// Manifests whose last activity is older than this are pruned by
/// [`list_sessions`], as long as their session isn't currently alive.
const MAX_MANIFEST_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub session_name: String,
    pub cwd: PathBuf,
    pub shell: PathBuf,
    pub created_at_secs: u64,
    pub last_active_secs: u64,
    pub status: SessionStatus,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Root cache directory for ekko (honors `EKKO_CACHE_DIR`).
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

fn session_info_root() -> PathBuf {
    cache_root()
        .join(format!("wire_v{}", ekko_proto::WIRE_VERSION))
        .join("session_info")
}

fn manifest_dir(session_name: &str) -> PathBuf {
    session_info_root().join(ekko_proto::encode_session_name(session_name))
}

fn manifest_path(session_name: &str) -> PathBuf {
    manifest_dir(session_name).join("manifest.json")
}

/// Create (or overwrite) the manifest for a freshly spawned session.
pub fn create(
    session_name: &str,
    cwd: &std::path::Path,
    shell: &std::path::Path,
) -> anyhow::Result<()> {
    let now = now_secs();
    let manifest = Manifest {
        session_name: session_name.to_string(),
        cwd: cwd.to_path_buf(),
        shell: shell.to_path_buf(),
        created_at_secs: now,
        last_active_secs: now,
        status: SessionStatus::Running,
    };
    write(&manifest)
}

fn write(manifest: &Manifest) -> anyhow::Result<()> {
    let dir = manifest_dir(&manifest.session_name);
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(manifest)?;
    fs::write(manifest_path(&manifest.session_name), json)?;
    Ok(())
}

pub fn read(session_name: &str) -> Option<Manifest> {
    let bytes = fs::read(manifest_path(session_name)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Bump `last_active_secs` without changing status. Called periodically by
/// the heartbeat thread and is a no-op if no manifest exists yet.
pub fn touch(session_name: &str) -> anyhow::Result<()> {
    if let Some(mut manifest) = read(session_name) {
        manifest.last_active_secs = now_secs();
        write(&manifest)?;
    }
    Ok(())
}

/// Update the status field (e.g. to `Exited` or `Crashed`), keeping the rest
/// of the manifest as-is.
pub fn set_status(session_name: &str, status: SessionStatus) -> anyhow::Result<()> {
    if let Some(mut manifest) = read(session_name) {
        manifest.status = status;
        manifest.last_active_secs = now_secs();
        write(&manifest)?;
    }
    Ok(())
}

/// Remove a session's manifest entirely (called on explicit `KillSession`).
pub fn delete(session_name: &str) {
    let _ = fs::remove_dir_all(manifest_dir(session_name));
}

/// List all known sessions: live ones (a socket is bound) plus resurrectable
/// ones (a manifest exists but no socket). Prunes manifests older than
/// [`MAX_MANIFEST_AGE`] that aren't currently alive.
pub fn list_sessions() -> anyhow::Result<Vec<SessionSummary>> {
    prune_stale_manifests();

    let mut names = std::collections::BTreeSet::new();
    if let Ok(entries) = fs::read_dir(ekko_proto::socket_dir()) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                names.insert(ekko_proto::decode_session_name(name));
            }
        }
    }
    if let Ok(entries) = fs::read_dir(session_info_root()) {
        for entry in entries.flatten() {
            if entry.path().is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                names.insert(ekko_proto::decode_session_name(name));
            }
        }
    }

    let mut summaries = Vec::with_capacity(names.len());
    for name in names {
        let alive = ekko_proto::socket_path(&name).exists();
        let summary = match read(&name) {
            Some(manifest) => SessionSummary {
                name: name.clone(),
                cwd: manifest.cwd,
                attached: false,
                alive,
                created_at_secs: manifest.created_at_secs,
                status: if alive {
                    SessionStatus::Running
                } else {
                    manifest.status
                },
            },
            None => SessionSummary {
                name: name.clone(),
                cwd: PathBuf::new(),
                attached: false,
                alive,
                created_at_secs: 0,
                status: SessionStatus::Running,
            },
        };
        summaries.push(summary);
    }
    Ok(summaries)
}

fn prune_stale_manifests() {
    let root = session_info_root();
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    let now = now_secs();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = entry
            .file_name()
            .to_str()
            .map(ekko_proto::decode_session_name)
        else {
            continue;
        };
        if ekko_proto::socket_path(&name).exists() {
            continue; // still alive, never prune
        }
        let Some(manifest) = read(&name) else {
            continue;
        };
        let age = now.saturating_sub(manifest.last_active_secs);
        if age > MAX_MANIFEST_AGE.as_secs() {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `cargo test` runs tests in this module on separate threads within the
    /// same process; since `EKKO_SOCKET_DIR`/`EKKO_CACHE_DIR` are process-global,
    /// mutating them without a lock races between tests. Hold this for the
    /// full duration of the env vars being set.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_dirs<F: FnOnce()>(f: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let socket_dir = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by `ENV_LOCK` above, so no other thread in this
        // process observes a torn or concurrently-mutated environment.
        unsafe {
            std::env::set_var("EKKO_SOCKET_DIR", socket_dir.path());
            std::env::set_var("EKKO_CACHE_DIR", cache_dir.path());
        }
        f();
        unsafe {
            std::env::remove_var("EKKO_SOCKET_DIR");
            std::env::remove_var("EKKO_CACHE_DIR");
        }
    }

    #[test]
    fn create_read_touch_roundtrip() {
        with_temp_dirs(|| {
            create(
                "s1",
                std::path::Path::new("/tmp"),
                std::path::Path::new("/bin/sh"),
            )
            .unwrap();
            let manifest = read("s1").expect("manifest should exist");
            assert_eq!(manifest.session_name, "s1");
            assert_eq!(manifest.status, SessionStatus::Running);

            touch("s1").unwrap();
            let manifest = read("s1").unwrap();
            assert!(manifest.last_active_secs > 0);

            set_status("s1", SessionStatus::Exited).unwrap();
            assert_eq!(read("s1").unwrap().status, SessionStatus::Exited);

            delete("s1");
            assert!(read("s1").is_none());
        });
    }

    #[test]
    fn list_sessions_reports_resurrectable_when_manifest_but_no_socket() {
        with_temp_dirs(|| {
            create(
                "s2",
                std::path::Path::new("/tmp"),
                std::path::Path::new("/bin/sh"),
            )
            .unwrap();
            set_status("s2", SessionStatus::Exited).unwrap();
            let sessions = list_sessions().unwrap();
            let s2 = sessions.iter().find(|s| s.name == "s2").unwrap();
            assert!(!s2.alive);
            assert_eq!(s2.status, SessionStatus::Exited);
        });
    }
}
