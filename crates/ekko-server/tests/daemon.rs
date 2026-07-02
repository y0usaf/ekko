//! Integration tests for the ekko session daemon: a real daemon (in-process,
//! on its own thread) talking to a real client connection over the actual
//! IPC socket. No mocks — this exercises `ekko_server::run` end to end.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ekko_proto::{
    ClientToServer, ExitReason, GridPayload, GridUpdate, ServerToClient, WIRE_VERSION, read_msg,
    write_msg,
};
use interprocess::local_socket::Stream as LocalSocketStream;
use interprocess::local_socket::traits::Stream as _;

/// `cargo test` runs tests on separate threads within one process, but
/// `EKKO_SOCKET_DIR`/`EKKO_CACHE_DIR` are process-global env vars. Serialize
/// tests in this file so each gets a hermetic, non-racing environment.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Holds the env-var lock and a pair of temp dirs for the duration of one
/// test's daemon + client interaction.
struct TestEnv {
    _guard: std::sync::MutexGuard<'static, ()>,
    _socket_dir: tempfile::TempDir,
    _cache_dir: tempfile::TempDir,
    work_dir: tempfile::TempDir,
    session_name: String,
}

impl TestEnv {
    fn new(session_name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let socket_dir = tempfile::tempdir().expect("tempdir for EKKO_SOCKET_DIR");
        let cache_dir = tempfile::tempdir().expect("tempdir for EKKO_CACHE_DIR");
        let work_dir = tempfile::tempdir().expect("tempdir for shell cwd");
        // SAFETY: serialized by `ENV_LOCK`, held until this `TestEnv` drops.
        unsafe {
            std::env::set_var("EKKO_SOCKET_DIR", socket_dir.path());
            std::env::set_var("EKKO_CACHE_DIR", cache_dir.path());
        }
        Self {
            _guard: guard,
            _socket_dir: socket_dir,
            _cache_dir: cache_dir,
            work_dir,
            session_name: session_name.to_string(),
        }
    }

    fn cwd(&self) -> PathBuf {
        self.work_dir.path().to_path_buf()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        // SAFETY: still serialized by `ENV_LOCK` (held until after this).
        unsafe {
            std::env::remove_var("EKKO_SOCKET_DIR");
            std::env::remove_var("EKKO_CACHE_DIR");
        }
    }
}

/// Runs the daemon for `session_name` on a background thread (`daemonize =
/// false`, i.e. no fork — just the hub loop on this thread).
fn spawn_daemon(session_name: String) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name(format!("test-daemon-{session_name}"))
        .spawn(move || {
            ekko_server::run(&session_name, false).expect("daemon run() returned an error");
        })
        .expect("spawn daemon thread")
}

fn wait_for_socket(session_name: &str) -> PathBuf {
    let path = ekko_proto::socket_path(session_name);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        if Instant::now() > deadline {
            panic!(
                "daemon did not bind its socket in time at {}",
                path.display()
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    path
}

/// A raw test client: a background reader thread decodes frames into a
/// channel so the test body can poll with timeouts without ever risking a
/// torn read on the underlying byte stream.
struct TestClient {
    send: LocalSocketStreamWriteHalf,
    rx: mpsc::Receiver<ServerToClient>,
}

// The `SendHalf` type from `interprocess` doesn't need naming beyond `Write`,
// but giving it an alias keeps `TestClient`'s field type readable.
type LocalSocketStreamWriteHalf =
    <LocalSocketStream as interprocess::local_socket::traits::Stream>::SendHalf;

impl TestClient {
    fn connect(session_name: &str) -> Self {
        let path = wait_for_socket(session_name);
        let stream = ekko_proto::ipc_connect(&path).expect("connect to daemon socket");
        let (mut recv_half, send_half) = stream.split();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            loop {
                match read_msg::<_, ServerToClient>(&mut recv_half) {
                    Ok(Some(msg)) => {
                        if tx.send(msg).is_err() {
                            return;
                        }
                    }
                    _ => return,
                }
            }
        });
        Self {
            send: send_half,
            rx,
        }
    }

    fn send(&mut self, msg: &ClientToServer) {
        write_msg(&mut self.send, msg).expect("write to daemon");
    }

    fn attach(&mut self, cwd: PathBuf, force: bool) {
        self.attach_with_size(cwd, force, 80, 24);
    }

    fn attach_with_size(&mut self, cwd: PathBuf, force: bool, cols: u16, rows: u16) {
        self.send(&ClientToServer::Attach {
            wire_version: WIRE_VERSION,
            cols,
            rows,
            cwd,
            shell: Some(PathBuf::from("/bin/sh")),
            force,
        });
    }

    /// Reads messages (blocking with a bounded per-read slice) until `pred`
    /// matches one, or the overall `timeout` elapses.
    fn wait_for(
        &self,
        timeout: Duration,
        mut pred: impl FnMut(&ServerToClient) -> bool,
    ) -> Option<ServerToClient> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match self
                .rx
                .recv_timeout(remaining.min(Duration::from_millis(200)))
            {
                Ok(msg) if pred(&msg) => return Some(msg),
                Ok(_) => continue,
                Err(_) => continue,
            }
        }
    }
}

/// Sends `KillSession` and waits for the daemon thread to actually exit,
/// so tests never leak a running shell child or a spinning hub thread.
/// (No-op-safe even if the session never spawned a PTY.)
fn kill_and_join(mut client: TestClient, daemon: thread::JoinHandle<()>, session_name: &str) {
    client.send(&ClientToServer::KillSession(session_name.to_string()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !daemon.is_finished() {
        if Instant::now() > deadline {
            panic!("daemon thread did not exit after KillSession");
        }
        thread::sleep(Duration::from_millis(20));
    }
    daemon.join().expect("daemon thread panicked");
}

fn grid_lines(update: &GridUpdate) -> Vec<String> {
    match &update.payload {
        GridPayload::Full(rows) => rows
            .iter()
            .map(|row| row.cells.iter().map(|c| c.ch).collect::<String>())
            .collect(),
        GridPayload::Rows(rows) => rows
            .iter()
            .map(|(_, row)| row.cells.iter().map(|c| c.ch).collect::<String>())
            .collect(),
    }
}

fn grid_contains(msg: &ServerToClient, needle: &str) -> bool {
    matches!(msg, ServerToClient::Grid(update) if grid_lines(update).iter().any(|l| l.contains(needle)))
}

#[test]
fn attach_type_and_see_output() {
    let env = TestEnv::new("t-basic");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd(), false);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some(),
        "expected Attached reply"
    );

    client.send(&ClientToServer::Key(b"printf hello\n".to_vec()));

    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "hello"))
            .is_some(),
        "expected a grid update containing 'hello'"
    );

    kill_and_join(client, daemon, &env.session_name);
}

#[test]
fn detach_then_reattach_preserves_screen() {
    let env = TestEnv::new("t-detach");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd(), false);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );

    client.send(&ClientToServer::Key(b"printf hello\n".to_vec()));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "hello"))
            .is_some()
    );

    client.send(&ClientToServer::Detach);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Exit(ExitReason::Detached)
            ))
            .is_some(),
        "expected Exit(Detached)"
    );
    drop(client);

    // Reattach with a fresh connection; the PTY + vt100 state kept running
    // headless, so the screen content should still be there.
    let mut client2 = TestClient::connect(&env.session_name);
    client2.attach(env.cwd(), false);
    assert!(
        client2
            .wait_for(Duration::from_secs(5), |m| grid_contains(m, "hello"))
            .is_some(),
        "expected reattach to show preserved screen content"
    );

    kill_and_join(client2, daemon, &env.session_name);
}

#[test]
fn two_clients_share_a_session() {
    let env = TestEnv::new("t-multi");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client1 = TestClient::connect(&env.session_name);
    client1.attach(env.cwd(), false);
    assert!(
        client1
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );

    let mut client2 = TestClient::connect(&env.session_name);
    client2.attach(env.cwd(), false);
    assert!(
        client2
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some(),
        "expected the second attach to succeed"
    );

    // Input typed on the second client is visible to the first: input is
    // accepted from every attached client and output broadcast to all.
    client2.send(&ClientToServer::Key(b"printf hello\n".to_vec()));
    assert!(
        client1
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "hello"))
            .is_some(),
        "expected client1 to see output typed by client2"
    );

    kill_and_join(client1, daemon, &env.session_name);
}

#[test]
fn force_attach_kicks_other_clients() {
    let env = TestEnv::new("t-kick");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client1 = TestClient::connect(&env.session_name);
    client1.attach(env.cwd(), false);
    assert!(
        client1
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );

    let mut client2 = TestClient::connect(&env.session_name);
    client2.attach(env.cwd(), true);
    assert!(
        client2
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );
    assert!(
        client1
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Exit(ExitReason::Kicked)
            ))
            .is_some(),
        "expected client1 to be kicked by the forced attach"
    );

    kill_and_join(client2, daemon, &env.session_name);
}

#[test]
fn session_sizes_to_smallest_attached_client() {
    let env = TestEnv::new("t-sizes");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client1 = TestClient::connect(&env.session_name);
    client1.attach(env.cwd(), false);
    assert!(
        client1
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );

    // A smaller client joins: the session shrinks to fit it, and the larger
    // client is told via a grid update at the new size.
    let mut client2 = TestClient::connect(&env.session_name);
    client2.attach_with_size(env.cwd(), false, 60, 20);
    assert!(
        client1
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Grid(GridUpdate {
                    cols: 60,
                    rows: 20,
                    ..
                })
            ))
            .is_some(),
        "expected the session to shrink to the smallest attached client"
    );

    // The small client leaves: the session grows back.
    client2.send(&ClientToServer::Detach);
    assert!(
        client1
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Grid(GridUpdate {
                    cols: 80,
                    rows: 24,
                    ..
                })
            ))
            .is_some(),
        "expected the session to grow back after the small client detached"
    );

    kill_and_join(client1, daemon, &env.session_name);
}

#[test]
fn ping_gets_pong() {
    let env = TestEnv::new("t-ping");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.send(&ClientToServer::Ping);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Pong
            ))
            .is_some(),
        "expected Pong"
    );

    kill_and_join(client, daemon, &env.session_name);
}

#[test]
fn shell_exit_ends_session_and_daemon() {
    let env = TestEnv::new("t-exit");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd(), false);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );

    client.send(&ClientToServer::Key(b"exit\n".to_vec()));

    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| matches!(
                m,
                ServerToClient::Exit(ExitReason::SessionExited(_))
            ))
            .is_some(),
        "expected Exit(SessionExited)"
    );

    // The daemon should tear itself down after the shell exits: no lingering
    // hub thread, no socket file, and (per ekko-pty's own reaper guarantees,
    // exercised directly in crates/ekko-pty/src/lib.rs's tests) no zombie left
    // behind for the child shell process.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !daemon.is_finished() {
        if Instant::now() > deadline {
            panic!("daemon thread did not exit after the shell exited");
        }
        thread::sleep(Duration::from_millis(20));
    }
    daemon.join().expect("daemon thread panicked");

    assert!(
        !ekko_proto::socket_path(&env.session_name).exists(),
        "expected the session socket to be removed after shell exit"
    );
}

#[test]
fn scroll_messages_move_the_scrollback_view() {
    let env = TestEnv::new("t-scroll");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.attach_with_size(env.cwd(), false, 80, 6);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );

    // Fill more lines than the 6-row screen so history exists.
    client.send(&ClientToServer::Key(
        b"i=0; while [ $i -lt 30 ]; do echo line$i; i=$((i+1)); done\n".to_vec(),
    ));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "line29"))
            .is_some(),
        "expected the loop output to reach the screen"
    );

    client.send(&ClientToServer::Scroll { delta: 5 });
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Grid(update) if update.scrollback == 5
            ))
            .is_some(),
        "expected a grid update with the view scrolled back by 5"
    );

    client.send(&ClientToServer::ScrollReset);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Grid(update) if update.scrollback == 0
            ))
            .is_some(),
        "expected the view to return to the live screen"
    );

    kill_and_join(client, daemon, &env.session_name);
}

#[test]
fn paste_is_rewrapped_when_the_child_uses_bracketed_paste() {
    let env = TestEnv::new("t-paste");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd(), false);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );

    // Enable bracketed paste from the child's side, then run `cat -A` so the
    // markers the daemon adds become visible text on the screen.
    client.send(&ClientToServer::Key(
        b"printf '\\033[?2004h'; echo READY; cat -A\n".to_vec(),
    ));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "READY"))
            .is_some(),
        "expected the child to enable bracketed paste"
    );

    client.send(&ClientToServer::Paste(b"hi".to_vec()));
    client.send(&ClientToServer::Key(b"\n".to_vec()));
    // cat -A renders ESC as ^[, so the wrapped paste shows as ^[[200~hi^[[201~.
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "200~hi"))
            .is_some(),
        "expected the paste to arrive wrapped in bracketed-paste markers"
    );

    kill_and_join(client, daemon, &env.session_name);
}

#[test]
fn child_mouse_mode_requests_are_reported_to_the_client() {
    let env = TestEnv::new("t-modes");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd(), false);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );

    client.send(&ClientToServer::Key(
        b"printf '\\033[?1002h\\033[?1006h\\033[?1004h'; echo MODESET\n".to_vec(),
    ));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| matches!(
                m,
                ServerToClient::Grid(update)
                    if update.modes.mouse_mode == ekko_proto::MouseMode::ButtonMotion
                        && update.modes.mouse_encoding == ekko_proto::MouseEncoding::Sgr
                        && update.modes.focus_reporting
            ))
            .is_some(),
        "expected the child's mouse/focus mode requests on the wire"
    );

    kill_and_join(client, daemon, &env.session_name);
}
