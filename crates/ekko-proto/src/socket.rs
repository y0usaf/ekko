//! Versioned unix-domain socket paths and IPC bind/connect helpers.
//!
//! Ported from `zellij-utils/src/consts.rs`. Sockets live under
//! `$XDG_RUNTIME_DIR/ekko/wire_v<N>` (falling back to `$TMPDIR/ekko-<uid>/wire_v<N>`
//! when `XDG_RUNTIME_DIR` isn't set), where `N` is [`WIRE_VERSION`]. Bumping
//! the wire version moves sockets to a new directory so old and new binaries
//! never collide on the same path.

use std::io;
use std::path::{Path, PathBuf};

use interprocess::local_socket::{
    GenericFilePath, Listener, ListenerOptions, Stream as LocalSocketStream, prelude::*,
};

/// Wire protocol version. Bump whenever [`crate::msg`] changes in a way that
/// breaks compatibility between client and server binaries.
///
/// v2: appended `ServerToClient::Notice` (bincode enum encoding is
/// positional, so appends still require a bump).
///
/// v3: removed `AttachRejectReason::AlreadyAttached` — sessions accept
/// multiple simultaneous clients.
///
/// v4: `GridUpdate` gained `modes`/`scrollback`, `GridCell` gained `extra`
/// (grapheme clusters), `CursorState` gained `shape`; added
/// `ClientToServer::{Scroll, ScrollReset}` and
/// `ServerToClient::{Title, ClipboardCopy}`.
///
/// v5: pruned never-produced surface — removed `ClientToServer::ListSessions`,
/// `ServerToClient::Sessions`, `AttachRejectReason::SessionNotFound`, and the
/// `ClientCommand` wrapper (its only live variant is now
/// `ClientToServer::KillCurrentSession`); `Attach` lost `session_name` /
/// `create_if_missing` (the daemon is per-session; creation is client-side).
///
/// v6: `Attach` gained `terminal_colors` — the client's OSC 10/11/4 probe
/// result, so the server can answer the child's color queries (OSC 10/11/4
/// with `?`) on the host terminal's behalf.
///
/// v7: added `ClientToServer::Activate` and
/// `ServerToClient::{Activate, ActivateResult}` for `ekko activate`.
/// v8: `ServerToClient::Workspace` pane projection + split/focus/close requests.
/// v9: `WorkspaceUpdate.border_style` (`PaneBorderStyle`).
pub const WIRE_VERSION: u32 = 9;

fn wire_dir_name() -> String {
    format!("wire_v{WIRE_VERSION}")
}

fn current_uid() -> u32 {
    // SAFETY: getuid() takes no arguments and cannot fail.
    unsafe { libc::getuid() }
}

/// Directory that holds all ekko unix-domain sockets for the current user and
/// wire version.
///
/// Honors `EKKO_SOCKET_DIR` as an override (used by tests to get a hermetic,
/// per-test socket directory instead of the shared per-user runtime dir).
pub fn socket_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("EKKO_SOCKET_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR")
        && !runtime_dir.is_empty()
    {
        return PathBuf::from(runtime_dir)
            .join("ekko")
            .join(wire_dir_name());
    }
    let tmp_dir = std::env::var_os("TMPDIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    tmp_dir
        .join(format!("ekko-{}", current_uid()))
        .join(wire_dir_name())
}

/// Encode a session name into a single filename component. Session names
/// are user-visible strings (the stock namer produces e.g.
/// "~/Dev/ekko blue-orangutan") but double as socket and manifest
/// filenames, so `/` and `%` are percent-encoded on the way to disk. Names
/// without those characters encode to themselves, keeping existing socket
/// and manifest paths stable.
pub fn encode_session_name(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '/' => encoded.push_str("%2F"),
            '%' => encoded.push_str("%25"),
            _ => encoded.push(c),
        }
    }
    encoded
}

/// Inverse of [`encode_session_name`]: decode a directory-entry filename
/// back into the session name. Unrecognized `%` sequences pass through
/// untouched, so filenames that were never encoded survive a round trip.
pub fn decode_session_name(file_name: &str) -> String {
    let mut decoded = String::with_capacity(file_name.len());
    let mut chars = file_name.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            decoded.push(c);
            continue;
        }
        let mut lookahead = chars.clone();
        match (lookahead.next(), lookahead.next()) {
            (Some('2'), Some('F' | 'f')) => {
                decoded.push('/');
                chars = lookahead;
            }
            (Some('2'), Some('5')) => {
                decoded.push('%');
                chars = lookahead;
            }
            _ => decoded.push(c),
        }
    }
    decoded
}

/// Full path to the session socket for `session_name`.
pub fn socket_path(session_name: &str) -> PathBuf {
    socket_dir().join(encode_session_name(session_name))
}

/// Set the unix permission bits on `path`.
#[cfg(unix)]
pub fn set_permissions(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
}

/// Create the socket directory (0700) if it doesn't already exist.
pub fn ensure_socket_dir() -> io::Result<PathBuf> {
    let dir = socket_dir();
    std::fs::create_dir_all(&dir)?;
    set_permissions(&dir, 0o700)?;
    Ok(dir)
}

/// Connect to an existing session socket.
pub fn ipc_connect(path: &Path) -> io::Result<LocalSocketStream> {
    let fs_name = path.to_fs_name::<GenericFilePath>()?;
    LocalSocketStream::connect(fs_name)
}

/// Bind a new session socket, creating the socket directory as needed and
/// setting the sticky bit on the resulting socket file.
///
/// Per the XDG base directory spec, files under `XDG_RUNTIME_DIR` should
/// either have their access time updated periodically or have the sticky bit
/// set, or they may be cleaned up by the OS. Not all platforms allow setting
/// the sticky bit on a socket file, so failure to do so is ignored.
pub fn ipc_bind(path: &Path) -> io::Result<Listener> {
    ensure_socket_dir()?;
    drop(std::fs::remove_file(path));
    let fs_name = path.to_fs_name::<GenericFilePath>()?;
    let listener = ListenerOptions::new().name(fs_name).create_sync()?;
    drop(set_permissions(path, 0o1700));
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_is_stable_and_versioned() {
        let a = socket_path("my-session");
        let b = socket_path("my-session");
        assert_eq!(a, b);
        assert!(a.to_string_lossy().contains(&wire_dir_name()));
        assert_eq!(a.file_name().unwrap(), "my-session");
    }

    #[test]
    fn socket_dir_is_versioned() {
        let dir = socket_dir();
        assert!(dir.ends_with(wire_dir_name()));
    }

    #[test]
    fn different_sessions_have_different_paths() {
        assert_ne!(socket_path("a"), socket_path("b"));
    }

    #[test]
    fn plain_names_encode_to_themselves() {
        assert_eq!(encode_session_name("main"), "main");
        assert_eq!(
            encode_session_name("~/Dev ekko"),
            "~/Dev ekko".replace('/', "%2F")
        );
    }

    #[test]
    fn name_encoding_round_trips() {
        for name in [
            "main",
            "~/Dev/ekko blue-orangutan",
            "100% legit/name",
            "%2F literal",
            "trailing%",
        ] {
            assert_eq!(decode_session_name(&encode_session_name(name)), name);
        }
    }

    #[test]
    fn encoded_names_are_single_path_components() {
        let path = socket_path("~/Dev/ekko blue-orangutan");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "~%2FDev%2Fekko blue-orangutan"
        );
    }

    #[test]
    fn unencoded_percent_sequences_pass_through_decode() {
        assert_eq!(decode_session_name("100%done"), "100%done");
        assert_eq!(decode_session_name("%"), "%");
    }
}
