//! Local session discovery: scan the socket directory (live sessions) and
//! the manifest directory (known/resurrectable sessions) without talking to
//! any per-session server. This is core mechanism (I/O); *grouping* the
//! result into projects is extension policy (the registered session
//! grouper).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ekko_ext::{SessionEntry, SessionState};
use ekko_proto::socket_dir;

/// Directory holding per-session manifests: `<cache_root>/wire_v<N>/session_info`.
/// Must resolve identically to the server's `resurrection::cache_root` —
/// `EKKO_CACHE_DIR` override first, then `$XDG_CACHE_HOME/ekko`, then `~/.cache/ekko`.
pub fn session_info_dir() -> PathBuf {
    cache_root()
        .join(format!("wire_v{}", ekko_proto::WIRE_VERSION))
        .join("session_info")
}

fn cache_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("EKKO_CACHE_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("ekko");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".cache").join("ekko")
}

/// Scan sockets + manifests and return the merged, deduplicated session list.
/// Never fails: any I/O or parse error just yields fewer entries.
pub fn scan_sessions() -> Vec<SessionEntry> {
    let mut entries: Vec<SessionEntry> = Vec::new();

    let alive_names = live_socket_names();

    if let Ok(read_dir) = std::fs::read_dir(session_info_dir()) {
        for dir_entry in read_dir.flatten() {
            let path = dir_entry.path();
            if !path.is_dir() {
                continue;
            }
            // Manifest dirs are encoded filenames (see
            // `ekko_proto::encode_session_name`); the JSON's `session_name`
            // field is authoritative, the decoded dir name a fallback.
            let dir_name =
                ekko_proto::decode_session_name(&dir_entry.file_name().to_string_lossy());
            let manifest_path = path.join("manifest.json");
            let Ok(content) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
                continue;
            };
            let name = value
                .get("session_name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or(dir_name);
            let cwd = value
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("~"));
            let created_at_secs = value
                .get("created_at_secs")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let state = if alive_names.contains(&name) {
                SessionState::Alive
            } else {
                SessionState::Gone
            };
            entries.push(SessionEntry {
                name,
                cwd,
                state,
                created_at_secs,
            });
        }
    }

    // Any live socket without a manifest still shows up (best-effort cwd).
    for name in &alive_names {
        if !entries.iter().any(|e| &e.name == name) {
            entries.push(SessionEntry {
                name: name.clone(),
                cwd: PathBuf::from("~"),
                state: SessionState::Alive,
                created_at_secs: now_secs(),
            });
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

fn live_socket_names() -> Vec<String> {
    let Ok(read_dir) = std::fs::read_dir(socket_dir()) else {
        return Vec::new();
    };
    read_dir
        .flatten()
        .filter(|entry| entry.path().is_file() || is_socket(&entry.path()))
        .map(|entry| ekko_proto::decode_session_name(&entry.file_name().to_string_lossy()))
        .collect()
}

#[cfg(unix)]
fn is_socket(path: &std::path::Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_socket())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_socket(_path: &std::path::Path) -> bool {
    false
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
