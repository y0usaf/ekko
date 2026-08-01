#![cfg(not(feature = "builtins"))]

//! Runtime acceptance coverage for the feature-free (bare) harness.  This is
//! deliberately a real hub and a real IPC socket, rather than invoking the
//! client internals through mocks.

use std::path::PathBuf;
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use ekko_proto::{
    ClientToServer, ExitReason, GridPayload, ServerToClient, WIRE_VERSION, read_msg, write_msg,
};
use std::os::unix::net::UnixStream as LocalSocketStream;

static ENV_LOCK: Mutex<()> = Mutex::new(());
type SendHalf = LocalSocketStream;

struct Env {
    _lock: std::sync::MutexGuard<'static, ()>,
    _socket: tempfile::TempDir,
    _cache: tempfile::TempDir,
    _config: tempfile::TempDir,
    cwd: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let socket = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        // The lock makes process-global configuration safe for cargo's test
        // threads, and the temporary config prevents host configuration from
        // changing the canvas geometry.
        unsafe {
            std::env::set_var("EKKO_SOCKET_DIR", socket.path());
            std::env::set_var("EKKO_CACHE_DIR", cache.path());
            std::env::set_var("XDG_CONFIG_HOME", config.path());
            std::env::set_var("SHELL", "/bin/sh");
        }
        Self {
            _lock: lock,
            _socket: socket,
            _cache: cache,
            _config: config,
            cwd,
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("EKKO_SOCKET_DIR");
            std::env::remove_var("EKKO_CACHE_DIR");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("SHELL");
        }
    }
}

struct Client {
    send: SendHalf,
    rx: mpsc::Receiver<ServerToClient>,
}

impl Client {
    fn connect(name: &str) -> Self {
        let path = ekko_proto::socket_path(name);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "daemon did not create its socket"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let stream = ekko_proto::ipc_connect(&path).unwrap();
        let mut recv = stream.try_clone().expect("clone test socket");
        let send = stream;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(Some(msg)) = read_msg::<_, ServerToClient>(&mut recv) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        Self { send, rx }
    }

    fn send(&mut self, msg: ClientToServer) {
        write_msg(&mut self.send, &msg).unwrap();
    }

    fn wait(
        &self,
        timeout: Duration,
        mut predicate: impl FnMut(&ServerToClient) -> bool,
    ) -> Option<ServerToClient> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(msg) if predicate(&msg) => return Some(msg),
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            }
        }
        None
    }
}

fn grid_has_full_canvas(msg: &ServerToClient, cols: u16, rows: u16) -> bool {
    let ServerToClient::Workspace(update) = msg else {
        return false;
    };
    update.grids.iter().any(|grid| {
        grid.update.cols == cols
            && grid.update.rows == rows
            && match &grid.update.payload {
                GridPayload::Full(lines) => {
                    lines.len() == rows as usize
                        && lines.iter().all(|line| line.cells.len() == cols as usize)
                }
                GridPayload::Rows(_) => false,
            }
    })
}

/// Proves all four bare-harness claims: a live session starts, attach returns
/// an attached full-canvas grid, raw input traverses the PTY and is rendered,
/// and the rendered screen survives detach followed by reattach.
#[test]
fn bare_harness_attach_input_and_detach_reattach() {
    let env = Env::new();
    let name = "bare-runtime";
    let daemon = thread::spawn({
        let name = name.to_string();
        move || ekko_server::run(&name, false).expect("daemon")
    });

    let mut client = Client::connect(name);
    client.send(ClientToServer::Attach {
        wire_version: WIRE_VERSION,
        cols: 80,
        rows: 24,
        cwd: env.cwd.path().to_path_buf(),
        shell: Some(PathBuf::from("/bin/sh")),
        force: false,
        terminal_colors: None,
    });
    assert!(
        client
            .wait(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );
    assert!(
        client
            .wait(Duration::from_secs(5), |m| grid_has_full_canvas(m, 80, 24))
            .is_some(),
        "attach must provide a full 80x24 grid"
    );

    client.send(ClientToServer::Key(b"printf bare-ok\\n".to_vec()));
    assert!(client.wait(Duration::from_secs(10), |m| {
        matches!(m, ServerToClient::Workspace(update) if update.grids.iter().any(|g| match &g.update.payload {
            GridPayload::Full(rows) => rows.iter().any(|r| r.cells.iter().map(|c| c.ch).collect::<String>().contains("bare-ok")),
            GridPayload::Rows(rows) => rows.iter().any(|(_, r)| r.cells.iter().map(|c| c.ch).collect::<String>().contains("bare-ok")),
        }))
    }).is_some(), "raw PTY input must return as rendered output");

    client.send(ClientToServer::Detach);
    assert!(
        client
            .wait(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Exit(ExitReason::Detached)
            ))
            .is_some()
    );
    drop(client);

    let mut reattached = Client::connect(name);
    reattached.send(ClientToServer::Attach {
        wire_version: WIRE_VERSION,
        cols: 80,
        rows: 24,
        cwd: env.cwd.path().to_path_buf(),
        shell: Some(PathBuf::from("/bin/sh")),
        force: false,
        terminal_colors: None,
    });
    assert!(reattached.wait(Duration::from_secs(10), |m| {
        matches!(m, ServerToClient::Workspace(update) if update.grids.iter().any(|g| match &g.update.payload {
            GridPayload::Full(rows) => rows.iter().any(|r| r.cells.iter().map(|c| c.ch).collect::<String>().contains("bare-ok")),
            GridPayload::Rows(rows) => rows.iter().any(|(_, r)| r.cells.iter().map(|c| c.ch).collect::<String>().contains("bare-ok")),
        }))
    }).is_some(), "screen content must survive reattach");

    reattached.send(ClientToServer::KillSession(name.to_string()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !daemon.is_finished() {
        assert!(Instant::now() < deadline, "daemon did not stop");
        thread::sleep(Duration::from_millis(20));
    }
    daemon.join().unwrap();
}
