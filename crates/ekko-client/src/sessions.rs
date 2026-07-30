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
/// Resolves through the workspace's single resolver (`ekko-paths`), shared
/// with the server's `ekko-resurrection` manifest writer.
pub fn session_info_dir() -> PathBuf {
    ekko_paths::cache_root()
        .join(format!("wire_v{}", ekko_proto::WIRE_VERSION))
        .join("session_info")
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
    // Only real sockets name live sessions: the dir also holds PID files
    // (`<name>.pid`), which would decode into phantom session names.
    read_dir
        .flatten()
        .filter(|entry| ekko_proto::is_socket(&entry.path()))
        .map(|entry| ekko_proto::decode_session_name(&entry.file_name().to_string_lossy()))
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
