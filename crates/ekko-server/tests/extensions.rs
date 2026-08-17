//! Integration tests for the daemon's extension surface: a real daemon
//! (in-process, via `run_with_runtime`) with injected extensions, talking to
//! a real client over the actual IPC socket.
//!
//! Covers the Phase-6 guarantees:
//! - lifecycle hooks fire in the expected order,
//! - `BeforePtySpawn` overrides reach the spawned shell,
//! - a blocked handler cannot wedge the hub (never-block regression),
//! - the bare harness (zero extensions) works and writes no manifests,
//! - a `host = "server"` Lua script drives the same seam end-to-end,
//!
//! plus end-to-end input checks that need the same real daemon + PTY:
//! - cursor keys are re-encoded to match the child's DECCKM state.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ekko_ext::{
    AppRuntime, CommandDispatch, EventHandlerRegistration, EventKind, EventReturn, Extension,
    ExtensionHost, ExtensionManifest, NoteKind, RuntimeBuilder, UiAction,
};
use ekko_proto::{
    ClientToServer, ExitReason, GridPayload, GridUpdate, ServerToClient, WIRE_VERSION, read_msg,
    write_msg,
};
use std::os::unix::net::UnixStream as LocalSocketStream;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct TestEnv {
    _guard: std::sync::MutexGuard<'static, ()>,
    _socket_dir: ekko_tmp::TempDir,
    cache_dir: ekko_tmp::TempDir,
    work_dir: ekko_tmp::TempDir,
    session_name: String,
}

impl TestEnv {
    fn new(session_name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let socket_dir = ekko_tmp::tempdir().expect("tempdir for EKKO_SOCKET_DIR");
        let cache_dir = ekko_tmp::tempdir().expect("tempdir for EKKO_CACHE_DIR");
        let work_dir = ekko_tmp::tempdir().expect("tempdir for shell cwd");
        // SAFETY: serialized by `ENV_LOCK`, held until this `TestEnv` drops.
        unsafe {
            std::env::set_var("EKKO_SOCKET_DIR", socket_dir.path());
            std::env::set_var("EKKO_CACHE_DIR", cache_dir.path());
        }
        Self {
            _guard: guard,
            _socket_dir: socket_dir,
            cache_dir,
            work_dir,
            session_name: session_name.to_string(),
        }
    }

    fn cwd(&self) -> PathBuf {
        self.work_dir.path().to_path_buf()
    }

    fn manifest_path(&self) -> PathBuf {
        self.cache_dir
            .path()
            .join(format!("wire_v{WIRE_VERSION}"))
            .join("session_info")
            .join(&self.session_name)
            .join("manifest.json")
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

fn spawn_daemon_with(session_name: String, runtime: AppRuntime) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name(format!("test-daemon-{session_name}"))
        .spawn(move || {
            ekko_server::run_with_runtime(
                &session_name,
                false,
                ekko_config::Config::default(),
                runtime,
            )
            .expect("daemon run_with_runtime() returned an error");
        })
        .expect("spawn daemon thread")
}

fn wait_for_socket(session_name: &str) -> PathBuf {
    let path = ekko_proto::socket_path(session_name);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        if Instant::now() > deadline {
            panic!("daemon did not bind its socket in time");
        }
        thread::sleep(Duration::from_millis(10));
    }
    path
}

type SendHalf = LocalSocketStream;

struct TestClient {
    send: SendHalf,
    rx: mpsc::Receiver<ServerToClient>,
}

impl TestClient {
    fn connect(session_name: &str) -> Self {
        let path = wait_for_socket(session_name);
        let stream = ekko_proto::ipc_connect(&path).expect("connect to daemon socket");
        let mut recv_half = stream.try_clone().expect("clone test socket");
        let send_half = stream;
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

    fn attach(&mut self, cwd: PathBuf) {
        self.send(&ClientToServer::Attach {
            wire_version: WIRE_VERSION,
            cols: 80,
            rows: 24,
            cwd,
            shell: Some(PathBuf::from("/bin/sh")),
            force: false,
            terminal_colors: None,
        });
    }

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

fn grid_contains(msg: &ServerToClient, needle: &str) -> bool {
    let lines = |update: &GridUpdate| -> Vec<String> {
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
    };
    matches!(msg, ServerToClient::Workspace(update) if update.grids.iter().any(|grid| lines(&grid.update).iter().any(|l| l.contains(needle))))
}

// ── Test extensions ─────────────────────────────────────────────────────────

/// Records every dispatched `EventKind` into a shared vec.
struct RecordingExtension {
    fired: Arc<Mutex<Vec<EventKind>>>,
    kinds: Vec<EventKind>,
}

impl Extension for RecordingExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "test.recorder".into(),
            name: "recorder".into(),
            version: "0".into(),
            description: String::new(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> ekko_err::Result<()> {
        for kind in &self.kinds {
            let fired = self.fired.clone();
            let kind = *kind;
            host.subscribe(EventHandlerRegistration {
                event: kind,
                label: format!("test.recorder/{kind:?}"),
                handler: Arc::new(move |event| {
                    fired.lock().unwrap().push(event.kind);
                    Ok(None)
                }),
            })?;
        }
        Ok(())
    }
}

/// Overrides the spawn environment through `BeforePtySpawn`.
struct EnvOverrideExtension;

impl Extension for EnvOverrideExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "test.env-override".into(),
            name: "env override".into(),
            version: "0".into(),
            description: String::new(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> ekko_err::Result<()> {
        host.subscribe(EventHandlerRegistration {
            event: EventKind::BeforePtySpawn,
            label: "test.env-override/inject".into(),
            handler: Arc::new(|_| {
                Ok(Some(EventReturn::PtySpawnOverride {
                    shell: None,
                    cwd: None,
                    env: vec![("EKKO_TEST_MARKER".into(), "override-works".into())],
                }))
            }),
        })
    }
}

/// A handler that blocks far past its budget on every recorded kind.
struct SleeperExtension;

impl Extension for SleeperExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "test.sleeper".into(),
            name: "sleeper".into(),
            version: "0".into(),
            description: String::new(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> ekko_err::Result<()> {
        for kind in [EventKind::ClientAttached, EventKind::BeforePtySpawn] {
            host.subscribe(EventHandlerRegistration {
                event: kind,
                label: format!("test.sleeper/{kind:?}"),
                handler: Arc::new(|_| {
                    thread::sleep(Duration::from_secs(60));
                    Ok(None)
                }),
            })?;
        }
        Ok(())
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn lifecycle_hooks_fire_in_order() {
    let env = TestEnv::new("t-hooks");
    let fired = Arc::new(Mutex::new(Vec::new()));
    let runtime = RuntimeBuilder::new()
        .register_extension(RecordingExtension {
            fired: fired.clone(),
            kinds: vec![
                EventKind::BeforePtySpawn,
                EventKind::SessionCreated,
                EventKind::ClientAttached,
                EventKind::ClientDetached,
                EventKind::SessionExited,
            ],
        })
        .build()
        .unwrap();
    let daemon = spawn_daemon_with(env.session_name.clone(), runtime);

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd());
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );
    client.send(&ClientToServer::Detach);
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Exit(ExitReason::Detached)
            ))
            .is_some()
    );
    drop(client);

    let client2 = {
        let mut c = TestClient::connect(&env.session_name);
        c.attach(env.cwd());
        c.wait_for(Duration::from_secs(5), |m| {
            matches!(m, ServerToClient::Attached { .. })
        })
        .expect("reattach");
        c
    };
    kill_and_join(client2, daemon, &env.session_name);

    let fired = fired.lock().unwrap().clone();
    let expected = [
        EventKind::BeforePtySpawn,
        EventKind::SessionCreated,
        EventKind::ClientAttached,
        EventKind::ClientDetached,
        EventKind::ClientAttached,
        EventKind::SessionExited,
    ];
    assert_eq!(fired, expected, "unexpected hook sequence: {fired:?}");
}

#[test]
fn pty_spawn_override_reaches_the_shell() {
    let env = TestEnv::new("t-override");
    let runtime = RuntimeBuilder::new()
        .register_extension(EnvOverrideExtension)
        .build()
        .unwrap();
    let daemon = spawn_daemon_with(env.session_name.clone(), runtime);

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd());
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );
    client.send(&ClientToServer::Key(
        b"printf \"%s\" \"$EKKO_TEST_MARKER\"\n".to_vec(),
    ));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(
                m,
                "override-works"
            ))
            .is_some(),
        "expected the overridden environment variable in shell output"
    );

    kill_and_join(client, daemon, &env.session_name);
}

/// The wasm-bridge acceptance for the daemon surface: a compiled `.wasm`
/// module (the shared cordis kernel, function set 2) mounts through
/// `ekko_ext::wasm`, its declared `(name, kind)` units are read back through
/// the same public registration API builtins use, and the runtime builds with
/// it (no privileged path). Since the kernel's set-6 host->guest dispatch
/// (W1 "dynamic dispatch — LANDED"), a declared command with a live guest
/// export is a REAL handler: it shows up in the registry and `invoke_command`
/// reaches the guest and applies its returned action.
#[test]
fn wasm_extension_declares_units_and_builds_runtime() {
    let cmd_res =
        r#"{"actions":[{"SetStatusNote":{"text":"c1 ready","kind":"Info","ttl_ms":1000}}]}"#;
    let esc = cmd_res.replace('\\', "\\\\").replace('"', "\\\"");
    let wasm_text = format!(
        r#"
        (module
          (import "host" "register_command" (func $reg (param i32 i32 i32 i32)))
          (import "host" "ctx_read" (func $read (param i32 i32)))
          (memory (export "memory") 2)
          (data (i32.const 0) "c1desc")
          (data (i32.const 64) "{esc}")
          (func (export "scratch") (result i32 i32) i32.const 256 i32.const 256)
          (func (export "mount")
            i32.const 0 i32.const 2 i32.const 2 i32.const 4 call $reg
            i32.const 0 i32.const 2 call $read)
          (func (export "on_change") (param i32 i32))
          (func (export "on_command") (param $p i32) (param $l i32) (result i32 i32)
            i32.const 64 i32.const {len})
        )
        "#,
        esc = esc,
        len = cmd_res.len()
    );
    let wasm = wat::parse_str(&wasm_text).expect("the wasm guest is valid");
    let ext = ekko_ext::wasm::WasmExtension::from_bytes(&wasm, "test.wasm-ext").unwrap();
    assert_eq!(
        ext.declared(),
        &[(
            "c1".to_string(),
            "command".to_string(),
            vec!["desc".to_string()]
        )],
        "the module's set-2 registration unit must be read back"
    );
    let runtime = RuntimeBuilder::new()
        .register_extension(ext)
        .build()
        .expect("runtime builds with the wasm extension");
    // The declared command now resolves to a LIVE handler (host->guest
    // dispatch). Assert the forward direction: it is in the registry.
    assert_eq!(runtime.command_infos().len(), 1);
    assert_eq!(runtime.command_infos()[0].name, "c1");
    assert!(
        runtime.command("c1").is_some(),
        "the declared command must resolve to a live spec"
    );
    // And the reverse direction: `invoke_command` reaches the guest's
    // on_command export and the host applies the returned action.
    assert_eq!(
        runtime.invoke_command(":c1"),
        CommandDispatch::Invoked(vec![UiAction::SetStatusNote {
            text: "c1 ready".into(),
            kind: NoteKind::Info,
            ttl_ms: 1000,
        }]),
        "the declared command's guest handler is reached and its action applied"
    );
}

/// Once a declared command has a live on_command export, a guest that does NOT
/// provide the export is the failure direction: the command is still
/// registered (registry matches) but its handler dispatch fails LOUDLY
/// (CommandDispatch::Failed), not silently inert.
#[test]
fn wasm_command_without_dispatch_export_fails_loudly() {
    let wasm = wat::parse_str(
        r#"
        (module
          (import "host" "register_command" (func $reg (param i32 i32 i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "c2")
          (func (export "scratch") (result i32 i32) i32.const 256 i32.const 256)
          (func (export "mount")
            i32.const 0 i32.const 2 i32.const 0 i32.const 0 call $reg)
          (func (export "on_change") (param i32 i32))
        )
        "#,
    )
    .unwrap();
    let ext = ekko_ext::wasm::WasmExtension::from_bytes(&wasm, "test-nodispatch").unwrap();
    let runtime = RuntimeBuilder::new()
        .register_extension(ext)
        .build()
        .expect("runtime builds");
    assert_eq!(
        runtime.command_infos().len(),
        1,
        "command 'c2' is registered"
    );
    assert!(
        matches!(runtime.invoke_command(":c2"), CommandDispatch::Failed(_)),
        "a registered command with no on_command export must fail loudly (no silent no-op)"
    );
}

#[test]
fn env_file_builtin_injects_project_environment() {
    let env = TestEnv::new("t-env-file");
    std::fs::write(
        env.cwd().join(".ekko-env"),
        "# project env\nEKKO_TEST_MARKER=env-file-works\n",
    )
    .unwrap();
    let mut builder = RuntimeBuilder::new();
    for extension in ekko_builtins::server_extensions() {
        builder = builder.register_boxed_extension(extension);
    }
    let daemon = spawn_daemon_with(env.session_name.clone(), builder.build().unwrap());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd());
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );
    client.send(&ClientToServer::Key(
        b"printf \"%s\" \"$EKKO_TEST_MARKER\"\n".to_vec(),
    ));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(
                m,
                "env-file-works"
            ))
            .is_some(),
        "expected the .ekko-env variable in shell output"
    );

    // The resurrection builtin is registered too: the manifest must exist.
    assert!(
        env.manifest_path().exists(),
        "expected the resurrection builtin to write a manifest"
    );

    kill_and_join(client, daemon, &env.session_name);

    // KillSession -> SessionExited(Killed) -> the builtin deletes the manifest.
    assert!(
        !env.manifest_path().exists(),
        "expected the resurrection builtin to delete the manifest on kill"
    );
}

#[test]
fn blocked_handler_does_not_wedge_the_hub() {
    let env = TestEnv::new("t-timeout");
    let runtime = RuntimeBuilder::new()
        .register_extension(SleeperExtension)
        .build()
        .unwrap();
    let daemon = spawn_daemon_with(env.session_name.clone(), runtime);

    let started = Instant::now();
    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd());
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some(),
        "attach must complete despite a 60s-sleeping handler"
    );
    client.send(&ClientToServer::Key(b"printf hi-there\n".to_vec()));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "hi-there"))
            .is_some()
    );
    // Gate budget is 500ms, lifecycle 2s: the whole flow must finish in a
    // few seconds, nowhere near the handler's 60s sleep.
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "hub stalled on a blocked handler"
    );

    kill_and_join(client, daemon, &env.session_name);
}

#[test]
fn cursor_keys_are_reencoded_for_the_childs_decckm_state() {
    let env = TestEnv::new("t-decckm");
    let daemon = spawn_daemon_with(env.session_name.clone(), AppRuntime::empty());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd());
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );

    // The child enables application cursor keys (DECCKM), then echoes its
    // stdin with escapes made visible — the role cmus/vim/less play.
    client.send(&ClientToServer::Key(
        b"printf '\\033[?1h'; cat -v\n".to_vec(),
    ));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| {
                matches!(m, ServerToClient::Workspace(update)
                    if update.grids.iter().any(|grid| grid.update.modes.app_cursor))
            })
            .is_some(),
        "the child's DECCKM set must reach the server parser"
    );

    // The host terminal sends the CSI form; the child must receive the SS3
    // form its terminfo declares for application mode (kcuu1=\EOA).
    client.send(&ClientToServer::Key(b"\x1b[A\n".to_vec()));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "^[OA"))
            .is_some(),
        "expected the arrow re-encoded as SS3 (^[OA) in the child's echo"
    );

    kill_and_join(client, daemon, &env.session_name);
}

#[test]
fn bare_harness_works_and_writes_no_manifest() {
    let env = TestEnv::new("t-bare");
    let daemon = spawn_daemon_with(env.session_name.clone(), AppRuntime::empty());

    let mut client = TestClient::connect(&env.session_name);
    client.attach(env.cwd());
    assert!(
        client
            .wait_for(Duration::from_secs(5), |m| matches!(
                m,
                ServerToClient::Attached { .. }
            ))
            .is_some()
    );
    client.send(&ClientToServer::Key(b"printf bare-ok\n".to_vec()));
    assert!(
        client
            .wait_for(Duration::from_secs(10), |m| grid_contains(m, "bare-ok"))
            .is_some(),
        "bare harness must still attach and pass keys through"
    );

    assert!(
        !env.manifest_path().exists(),
        "bare harness must not write manifests (resurrection is a builtin)"
    );

    kill_and_join(client, daemon, &env.session_name);
}
