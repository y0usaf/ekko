//! Integration tests for the ekko session daemon: a real daemon (in-process,
//! on its own thread) talking to a real client connection over the actual
//! IPC socket. No mocks — this exercises `ekko_server::run` end to end.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ekko_proto::{
    ClientToServer, ExitReason, GridPayload, GridUpdate, ServerToClient, WIRE_VERSION,
    WorkspaceUpdate, read_msg, write_msg,
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
    _config_home: tempfile::TempDir,
    work_dir: tempfile::TempDir,
    session_name: String,
}

impl TestEnv {
    fn new(session_name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let socket_dir = tempfile::tempdir().expect("tempdir for EKKO_SOCKET_DIR");
        let cache_dir = tempfile::tempdir().expect("tempdir for EKKO_CACHE_DIR");
        let work_dir = tempfile::tempdir().expect("tempdir for shell cwd");
        let config_home = tempfile::tempdir().expect("tempdir for XDG_CONFIG_HOME");
        // SAFETY: serialized by `ENV_LOCK`, held until this `TestEnv` drops.
        unsafe {
            std::env::set_var("EKKO_SOCKET_DIR", socket_dir.path());
            std::env::set_var("EKKO_CACHE_DIR", cache_dir.path());
            // Split panes spawn the configured shell (`resolve_shell` falls
            // back to `$SHELL`): pin it so pane tests don't depend on the
            // host's interactive shell's rendering behavior.
            std::env::set_var("SHELL", "/bin/sh");
            // The daemon loads the real user config otherwise; geometry
            // assertions (pane sizes, splits) must not depend on the
            // host's `ui.pane_borders` or sidebar width.
            std::env::set_var("XDG_CONFIG_HOME", config_home.path());
        }
        Self {
            _guard: guard,
            _socket_dir: socket_dir,
            _cache_dir: cache_dir,
            _config_home: config_home,
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
            std::env::remove_var("SHELL");
            std::env::remove_var("XDG_CONFIG_HOME");
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
            terminal_colors: None,
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

/// Every pane grid carried by a workspace frame, as line strings per pane.
fn workspace_grid_lines(update: &WorkspaceUpdate) -> Vec<Vec<String>> {
    update
        .grids
        .iter()
        .map(|grid| grid_lines(&grid.update))
        .collect()
}

fn grid_contains(msg: &ServerToClient, needle: &str) -> bool {
    matches!(msg, ServerToClient::Workspace(update) if workspace_grid_lines(update).iter().flatten().any(|l| l.contains(needle)))
}

/// The grid of the frame's focused pane, for assertions that care about the
/// focused pane's view (scrollback, modes, size).
fn focused_grid(update: &WorkspaceUpdate) -> Option<&GridUpdate> {
    update
        .grids
        .iter()
        .find(|grid| grid.pane == update.focused)
        .map(|grid| &grid.update)
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
    // client is told via a workspace frame at the new canvas size.
    let mut client2 = TestClient::connect(&env.session_name);
    client2.attach_with_size(env.cwd(), false, 60, 20);
    assert!(
        client1
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Workspace(update)
                    if update.panes.len() == 1
                        && update.panes[0].rect.cols == 60
                        && update.panes[0].rect.rows == 20
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
                ServerToClient::Workspace(update)
                    if update.panes.len() == 1
                        && update.panes[0].rect.cols == 80
                        && update.panes[0].rect.rows == 24
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
fn activate_reports_undelivered_without_an_attached_client() {
    let env = TestEnv::new("t-activate-empty");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.send(&ClientToServer::Activate);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::ActivateResult { delivered: false }
            ))
            .is_some(),
        "expected ActivateResult {{ delivered: false }}"
    );

    kill_and_join(client, daemon, &env.session_name);
}

#[test]
fn activate_is_relayed_to_one_attached_client() {
    let env = TestEnv::new("t-activate");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut viewer1 = TestClient::connect(&env.session_name);
    viewer1.attach(env.cwd(), false);
    assert!(
        viewer1
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some(),
        "expected first viewer to attach"
    );

    let mut viewer2 = TestClient::connect(&env.session_name);
    viewer2.attach(env.cwd(), false);
    assert!(
        viewer2
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some(),
        "expected second viewer to attach"
    );

    let mut requester = TestClient::connect(&env.session_name);
    requester.send(&ClientToServer::Activate);
    assert!(
        requester
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::ActivateResult { delivered: true }
            ))
            .is_some(),
        "expected ActivateResult {{ delivered: true }}"
    );

    let got1 = viewer1
        .wait_for(Duration::from_secs(2), |m| {
            matches!(m, ServerToClient::Activate)
        })
        .is_some();
    let got2 = viewer2
        .wait_for(Duration::from_secs(2), |m| {
            matches!(m, ServerToClient::Activate)
        })
        .is_some();
    assert_eq!(
        u8::from(got1) + u8::from(got2),
        1,
        "expected exactly one attached client to receive Activate"
    );

    kill_and_join(requester, daemon, &env.session_name);
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
fn scrollback_search_and_dump_answer_over_the_wire() {
    let env = TestEnv::new("t-search");
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
        b"i=0; while [ $i -lt 30 ]; do echo hitme$i; i=$((i+1)); done\n".to_vec(),
    ));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "hitme29"))
            .is_some(),
        "expected the loop output to reach the screen"
    );

    // Search: every hitmeNN line matches, in absolute coordinates; the
    // reply names the focused pane and echoes the query.
    client.send(&ClientToServer::SearchScrollback {
        query: "hitme".to_string(),
    });
    let results = client.wait_for(Duration::from_secs(5), |m| {
        matches!(m, ServerToClient::SearchResults { .. })
    });
    let Some(ServerToClient::SearchResults {
        pane: _,
        query,
        matches,
    }) = results
    else {
        panic!("expected SearchResults");
    };
    assert_eq!(query, "hitme");
    // 30 hitme lines, plus however often the local sh build echoes the
    // command line (varies between hosts/sandboxes).
    assert!(matches.len() >= 30, "every hitme line: {matches:?}");
    assert!(
        matches.windows(2).all(|pair| pair[0].row <= pair[1].row),
        "matches arrive in row order"
    );

    // No matches: an empty result set, not an error.
    client.send(&ClientToServer::SearchScrollback {
        query: "no-such-text".to_string(),
    });
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::SearchResults { matches, .. } if matches.is_empty()
            ))
            .is_some(),
        "expected an empty SearchResults"
    );

    // Dump: the whole transcript comes back as text.
    client.send(&ClientToServer::DumpScrollback);
    let dump = client.wait_for(Duration::from_secs(5), |m| {
        matches!(m, ServerToClient::ScrollbackDump { .. })
    });
    let Some(ServerToClient::ScrollbackDump { text, .. }) = dump else {
        panic!("expected ScrollbackDump");
    };
    assert!(text.contains("\nhitme0\n"));
    assert!(text.contains("hitme29"));

    kill_and_join(client, daemon, &env.session_name);
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
                ServerToClient::Workspace(update)
                    if focused_grid(update).is_some_and(|grid| grid.scrollback == 5)
            ))
            .is_some(),
        "expected a grid update with the view scrolled back by 5"
    );

    client.send(&ClientToServer::ScrollReset);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Workspace(update)
                    if focused_grid(update).is_some_and(|grid| grid.scrollback == 0)
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
                ServerToClient::Workspace(update)
                    if focused_grid(update).is_some_and(|grid|
                        grid.modes.mouse_mode == ekko_proto::MouseMode::ButtonMotion
                            && grid.modes.mouse_encoding == ekko_proto::MouseEncoding::Sgr
                            && grid.modes.focus_reporting)
            ))
            .is_some(),
        "expected the child's mouse/focus mode requests on the wire"
    );

    kill_and_join(client, daemon, &env.session_name);
}

#[test]
fn panes_split_focus_and_close_over_the_wire() {
    let env = TestEnv::new("t-panes");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd(), false);
    let first = client
        .wait_for(
            Duration::from_secs(10),
            |m| matches!(m, ServerToClient::Workspace(update) if update.panes.len() == 1),
        )
        .and_then(|m| match m {
            ServerToClient::Workspace(update) => Some(update),
            _ => None,
        })
        .expect("expected the initial one-pane workspace");
    let initial = first.panes[0].id;
    assert_eq!(first.focused, initial);
    assert_eq!(first.panes[0].rect.cols, 80);

    // Split right: a second pane appears to the right, focused, at half the
    // canvas; the sibling keeps the left half.
    client.send(&ClientToServer::SplitPane {
        direction: ekko_proto::SplitDirection::Right,
    });
    let split = client
        .wait_for(
            Duration::from_secs(10),
            |m| matches!(m, ServerToClient::Workspace(update) if update.panes.len() == 2),
        )
        .and_then(|m| match m {
            ServerToClient::Workspace(update) => Some(update),
            _ => None,
        })
        .expect("expected a two-pane workspace after the split");
    let child = split.focused;
    assert_ne!(child, initial);
    let left = split.panes.iter().find(|p| p.id == initial).unwrap();
    let right = split.panes.iter().find(|p| p.id == child).unwrap();
    assert_eq!((left.rect.x, left.rect.cols), (0, 40));
    assert_eq!((right.rect.x, right.rect.cols), (40, 40));

    // Output typed into the focused (right) pane lands in its grid only.
    client.send(&ClientToServer::Key(b"printf rightpane\n".to_vec()));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| matches!(
                m,
                ServerToClient::Workspace(update)
                    if update.grids.iter().any(|grid| grid.pane == child
                        && grid_lines(&grid.update).iter().any(|l| l.contains("rightpane")))
            ))
            .is_some(),
        "expected the right pane's output in its own grid"
    );

    // Focus moves back left directionally, then right by id.
    client.send(&ClientToServer::FocusDirection {
        direction: ekko_proto::Direction::Left,
    });
    let mut seen: Vec<String> = Vec::new();
    let moved = client.wait_for(Duration::from_secs(10), |m| {
        if let ServerToClient::Workspace(update) = m {
            seen.push(format!(
                "epoch={} focused={} panes={:?} grids={}",
                update.epoch,
                update.focused,
                update.panes.iter().map(|p| p.id).collect::<Vec<_>>(),
                update.grids.len()
            ));
        }
        matches!(m, ServerToClient::Workspace(update) if update.focused == initial)
    });
    assert!(
        moved.is_some(),
        "expected directional focus back to the left pane; frames: {seen:?}"
    );
    client.send(&ClientToServer::FocusPane { pane: child });
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| matches!(
                m,
                ServerToClient::Workspace(update) if update.focused == child
            ))
            .is_some(),
        "expected focus-by-id back to the right pane"
    );

    // Closing the focused (right) pane removes it and expands the sibling.
    client.send(&ClientToServer::CloseFocusedPane);
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| matches!(
                m,
                ServerToClient::Workspace(update)
                    if update.panes.len() == 1
                        && update.panes[0].id == initial
                        && update.panes[0].rect.cols == 80
                        && update.focused == initial
            ))
            .is_some(),
        "expected the sibling to absorb the closed pane's space"
    );

    kill_and_join(client, daemon, &env.session_name);
}

// ── P5 end-to-end pane acceptance ───────────────────────────────────────────

/// Waits for a workspace frame matching `pred` and returns it.
fn wait_workspace(
    client: &TestClient,
    timeout: Duration,
    mut pred: impl FnMut(&WorkspaceUpdate) -> bool,
) -> Option<WorkspaceUpdate> {
    client
        .wait_for(timeout, move |m| match m {
            ServerToClient::Workspace(update) => pred(update),
            _ => false,
        })
        .and_then(|m| match m {
            ServerToClient::Workspace(update) => Some(update),
            _ => None,
        })
}

#[test]
fn pane_child_exit_expands_sibling_and_last_exit_ends_session() {
    let env = TestEnv::new("t-p5-exit");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd(), false);
    assert!(wait_workspace(&client, Duration::from_secs(10), |u| u.panes.len() == 1).is_some());
    client.send(&ClientToServer::SplitPane {
        direction: ekko_proto::SplitDirection::Right,
    });
    let split = wait_workspace(&client, Duration::from_secs(10), |u| u.panes.len() == 2)
        .expect("two panes after the split");
    let child = split.focused;
    let initial = split.panes.iter().find(|p| p.id != child).unwrap().id;

    // The focused (right) shell exits: its pane is removed and the sibling
    // absorbs the full canvas; the session lives on.
    client.send(&ClientToServer::Key(b"exit\n".to_vec()));
    assert!(
        wait_workspace(&client, Duration::from_secs(10), |u| {
            u.panes.len() == 1 && u.panes[0].id == initial && u.panes[0].rect.cols == 80
        })
        .is_some(),
        "expected the sibling to expand after the child shell exited"
    );

    // The last shell exits: the session ends as the single PTY does today.
    client.send(&ClientToServer::Key(b"exit\n".to_vec()));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| matches!(
                m,
                ServerToClient::Exit(ExitReason::SessionExited(_))
            ))
            .is_some(),
        "expected Exit(SessionExited) after the last pane's shell exited"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !daemon.is_finished() {
        if Instant::now() > deadline {
            panic!("daemon thread did not exit after the last pane exited");
        }
        thread::sleep(Duration::from_millis(20));
    }
    daemon.join().expect("daemon thread panicked");
}

#[test]
fn detach_reattach_preserves_panes_and_clients_keep_independent_focus() {
    let env = TestEnv::new("t-p5-detach");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client1 = TestClient::connect(&env.session_name);
    client1.attach(env.cwd(), false);
    assert!(wait_workspace(&client1, Duration::from_secs(10), |u| u.panes.len() == 1).is_some());
    client1.send(&ClientToServer::SplitPane {
        direction: ekko_proto::SplitDirection::Right,
    });
    let split = wait_workspace(&client1, Duration::from_secs(10), |u| u.panes.len() == 2)
        .expect("two panes after the split");
    let child = split.focused;
    let initial = split.panes.iter().find(|p| p.id != child).unwrap().id;
    client1.send(&ClientToServer::Key(b"printf preserved\n".to_vec()));
    assert!(
        client1
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "preserved"))
            .is_some()
    );

    client1.send(&ClientToServer::Detach);
    assert!(
        client1
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Exit(ExitReason::Detached)
            ))
            .is_some()
    );
    drop(client1);

    // Reattach: both children, the topology, and the output survive.
    let mut client2 = TestClient::connect(&env.session_name);
    client2.attach(env.cwd(), false);
    let restored = wait_workspace(&client2, Duration::from_secs(10), |u| {
        u.panes.len() == 2
            && u.grids.iter().any(|g| {
                g.pane == child
                    && grid_lines(&g.update)
                        .iter()
                        .any(|l| l.contains("preserved"))
            })
    });
    assert!(
        restored.is_some(),
        "reattach must preserve both panes and their content"
    );

    // Two simultaneous clients focus different panes; input routes per
    // client, never cross-redirected.
    let mut client1 = TestClient::connect(&env.session_name);
    client1.attach(env.cwd(), false);
    assert!(wait_workspace(&client1, Duration::from_secs(10), |u| u.panes.len() == 2).is_some());
    client1.send(&ClientToServer::FocusPane { pane: initial });
    client2.send(&ClientToServer::FocusPane { pane: child });
    assert!(
        wait_workspace(&client1, Duration::from_secs(10), |u| u.focused == initial).is_some(),
        "client1 focused the left pane"
    );
    assert!(
        wait_workspace(&client2, Duration::from_secs(10), |u| u.focused == child).is_some(),
        "client2 focused the right pane"
    );

    client1.send(&ClientToServer::Key(b"printf leftin\n".to_vec()));
    client2.send(&ClientToServer::Key(b"printf rightin\n".to_vec()));
    assert!(
        client1
            .wait_for(Duration::from_secs(10), |m| matches!(
                m,
                ServerToClient::Workspace(u)
                    if u.grids.iter().any(|g| g.pane == initial
                        && grid_lines(&g.update).iter().any(|l| l.contains("leftin")))
            ))
            .is_some(),
        "client1's input landed in its own focused pane"
    );
    let mut seen2: Vec<String> = Vec::new();
    let landed = client2.wait_for(Duration::from_secs(10), |m| {
        if let ServerToClient::Workspace(u) = m {
            seen2.push(format!(
                "focused={} panes={:?} grids={:?}",
                u.focused,
                u.panes.iter().map(|p| p.id).collect::<Vec<_>>(),
                u.grids
                    .iter()
                    .map(|g| (g.pane, grid_lines(&g.update).join("|")))
                    .collect::<Vec<_>>()
            ));
        }
        matches!(m, ServerToClient::Workspace(u)
            if u.grids.iter().any(|g| g.pane == child
                && grid_lines(&g.update).iter().any(|l| l.contains("rightin"))))
    });
    assert!(
        landed.is_some(),
        "client2's input landed in its own focused pane; frames: {seen2:?}"
    );

    kill_and_join(client1, daemon, &env.session_name);
}

#[test]
fn resize_reaches_every_pane_and_a_flood_converges() {
    let env = TestEnv::new("t-p5-resize");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd(), false);
    assert!(wait_workspace(&client, Duration::from_secs(10), |u| u.panes.len() == 1).is_some());
    client.send(&ClientToServer::SplitPane {
        direction: ekko_proto::SplitDirection::Right,
    });
    assert!(wait_workspace(&client, Duration::from_secs(10), |u| u.panes.len() == 2).is_some());

    // One resize: every pane gets its resolved dimensions.
    client.send(&ClientToServer::Resize { cols: 60, rows: 20 });
    assert!(
        wait_workspace(&client, Duration::from_secs(10), |u| {
            u.panes.len() == 2
                && u.panes
                    .iter()
                    .all(|p| p.rect.cols == 30 && p.rect.rows == 20)
                && u.grids
                    .iter()
                    .all(|g| g.update.cols == 30 && g.update.rows == 20)
        })
        .is_some(),
        "expected both panes at 30x20 after the resize"
    );

    // A resize/split flood stays bounded and converges to the latest
    // geometry: final state must exactly match the last request (76x24
    // canvas, right column split in two rows of 12).
    for cols in [70, 64, 90, 76] {
        client.send(&ClientToServer::Resize { cols, rows: 24 });
    }
    client.send(&ClientToServer::SplitPane {
        direction: ekko_proto::SplitDirection::Down,
    });
    let settled = wait_workspace(&client, Duration::from_secs(10), |u| {
        u.panes.len() == 3
            && u.panes
                .iter()
                .all(|p| (p.rect.rows == 24 || p.rect.rows == 12) && p.rect.x + p.rect.cols <= 76)
    });
    assert!(
        settled.is_some(),
        "expected the flood to converge to the final 76x24 canvas with three panes"
    );
    let settled = settled.unwrap();
    let rows: Vec<u16> = {
        let mut rows: Vec<u16> = settled.panes.iter().map(|p| p.rect.rows).collect();
        rows.sort_unstable();
        rows
    };
    assert_eq!(rows, vec![12, 12, 24], "final geometry exactly settled");

    kill_and_join(client, daemon, &env.session_name);
}

#[test]
fn pane_modes_and_title_stay_pane_local() {
    let env = TestEnv::new("t-p5-modes");
    let daemon = spawn_daemon(env.session_name.clone());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd(), false);
    assert!(wait_workspace(&client, Duration::from_secs(10), |u| u.panes.len() == 1).is_some());
    client.send(&ClientToServer::SplitPane {
        direction: ekko_proto::SplitDirection::Right,
    });
    let split = wait_workspace(&client, Duration::from_secs(10), |u| u.panes.len() == 2)
        .expect("two panes after the split");
    let child = split.focused;
    let initial = split.panes.iter().find(|p| p.id != child).unwrap().id;

    // The right pane's child asks for mouse + focus reporting and sets a
    // title: modes ride only its own grid, the title goes out as a message
    // (this client is its focused viewer) and lands in its metadata.
    client.send(&ClientToServer::Key(
        b"printf '\\033[?1002h\\033[?1006h\\033]2;right-title\\007'; echo MODEREADY\n".to_vec(),
    ));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| matches!(
                m,
                ServerToClient::Title(t) if t == "right-title"
            ))
            .is_some(),
        "the focused pane's title is forwarded to its viewer"
    );
    assert!(
        wait_workspace(&client, Duration::from_secs(10), |u| {
            let right = u.grids.iter().find(|g| g.pane == child);
            let left = u.grids.iter().find(|g| g.pane == initial);
            right.is_some_and(|g| g.update.modes.mouse_mode == ekko_proto::MouseMode::ButtonMotion)
                && u.panes
                    .iter()
                    .any(|p| p.id == child && p.title.as_deref() == Some("right-title"))
                && left.is_none_or(|g| g.update.modes.mouse_mode == ekko_proto::MouseMode::None)
        })
        .is_some(),
        "mouse modes and title must stay scoped to the requesting pane"
    );

    kill_and_join(client, daemon, &env.session_name);
}
