//! End-to-end tests of the Lua bridge: scripts register through the real
//! `RuntimeBuilder`, callbacks round-trip through the real `AppRuntime`
//! dispatch paths, and the guard rails (instruction budgets, buffered draw
//! ops) actually hold.

use std::path::Path;

use ekko_ext::{
    AppRuntime, ClientSnapshot, Color, CommandDispatch, DrawContext, EventKind, EventPayload,
    EventReturn, KeyIntercept, NoteKind, Rect, RuntimeBuilder, SessionEntry, SessionExitReason,
    SessionState, SurfaceSize, ThemePalette, UiAction,
};
use ekko_lua::LuaExtension;

fn runtime(source: &str) -> AppRuntime {
    RuntimeBuilder::new()
        .register_boxed_extension(Box::new(
            LuaExtension::from_source("test.lua", source).expect("script loads"),
        ))
        .build()
        .expect("runtime builds")
}

fn snapshot() -> ClientSnapshot {
    ClientSnapshot {
        session_name: "main".into(),
        mode: ClientSnapshot::NORMAL_MODE.into(),
        cols: 80,
        rows: 24,
        grid_cols: 80,
        grid_rows: 23,
        scrollback: 0,
        projects: Vec::new(),
        status_note: None,
        keybindings: vec![],
        now_ms: 12_345,
        hidden_surfaces: Vec::new(),
        theme: ThemePalette::fallback(),
    }
}

/// A recording DrawContext: every replayed op lands here.
#[derive(Default)]
struct Recorder {
    calls: Vec<String>,
}

impl DrawContext for Recorder {
    fn size(&self) -> (i32, i32) {
        (80, 1)
    }
    fn fill_rect(&mut self, rect: Rect, _fg: Color, _bg: Color) {
        self.calls.push(format!(
            "fill {} {} {} {}",
            rect.col, rect.row, rect.cols, rect.rows
        ));
    }
    fn set_cell(&mut self, col: i32, row: i32, _fg: Color, _bg: Color, text: &str, _u: bool) {
        self.calls.push(format!("cell {col} {row} {text}"));
    }
    fn put_text(&mut self, col: i32, row: i32, _max: i32, _fg: Color, _bg: Color, value: &str) {
        self.calls.push(format!("text {col} {row} {value}"));
    }
    fn put_text_bold(
        &mut self,
        col: i32,
        row: i32,
        _max: i32,
        _fg: Color,
        _bg: Color,
        value: &str,
    ) {
        self.calls.push(format!("bold {col} {row} {value}"));
    }
    fn put_text_styled(
        &mut self,
        col: i32,
        row: i32,
        _max: i32,
        value: &str,
        style: ekko_ext::TextStyle,
    ) {
        self.calls.push(format!(
            "styled {col} {row} {value} r={} b={}",
            style.reverse, style.bold
        ));
    }
    fn draw_box(&mut self, rect: Rect, _f: Color, _b: Color, _border: Color) {
        self.calls.push(format!("box {} {}", rect.cols, rect.rows));
    }
    fn render_scrollbar(
        &mut self,
        col: i32,
        row: i32,
        rows: i32,
        model: ekko_ext::ScrollbarModel,
        style: ekko_ext::ScrollbarStyle<'_>,
    ) {
        self.calls.push(format!(
            "scrollbar {col} {row} {rows} {}/{}@{} {}{}",
            model.visible_items,
            model.total_items,
            model.scroll_from_top,
            style.track_glyph,
            style.thumb_glyph,
        ));
    }
}

#[test]
fn manifest_comes_from_the_script() {
    let runtime = runtime(
        r#"
        return {
          id = "user.test",
          name = "test extension",
          version = "1.2.3",
          description = "a test",
          register = function(ekko) end,
        }
        "#,
    );
    let manifest = &runtime.manifests()[0];
    assert_eq!(manifest.id, "user.test");
    assert_eq!(manifest.name, "test extension");
    assert_eq!(manifest.version, "1.2.3");
}

#[test]
fn commands_round_trip_args_and_actions() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.cmd" }
        function ext.register(ekko)
          ekko.register_command({
            name = "greet",
            aliases = { "g" },
            description = "greet someone",
            handler = function(args)
              if args == "" then
                return "detach"
              end
              return {
                { set_status_note = { text = "hi " .. args, kind = "ok", ttl_ms = 1000 } },
                { switch_session = args },
              }
            end,
          })
        end
        return ext
        "#,
    );
    assert_eq!(
        runtime.invoke_command(":greet"),
        CommandDispatch::Invoked(vec![UiAction::Detach])
    );
    assert_eq!(
        runtime.invoke_command(":g bob"),
        CommandDispatch::Invoked(vec![
            UiAction::SetStatusNote {
                text: "hi bob".into(),
                kind: NoteKind::Ok,
                ttl_ms: 1000,
            },
            UiAction::SwitchSession { name: "bob".into() },
        ])
    );
}

#[test]
fn keybindings_match_and_see_the_snapshot() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.keys" }
        function ext.register(ekko)
          ekko.register_keybinding({
            chord = "ctrl+g",
            description = "go",
            handler = function(snapshot)
              return { { invoke_command = ":switch " .. snapshot.session_name } }
            end,
          })
        end
        return ext
        "#,
    );
    let spec = runtime
        .match_keybinding(&[0x07], None)
        .expect("ctrl+g registered");
    assert_eq!(spec.chord_text, "ctrl+g");
    let actions = (spec.handler)(&snapshot());
    assert_eq!(
        actions,
        vec![UiAction::InvokeCommand {
            line: ":switch main".into()
        }]
    );
}

#[test]
fn surface_draw_ops_are_buffered_and_replayed() {
    let runtime = runtime(
        r##"
        local ext = { id = "user.surf" }
        function ext.register(ekko)
          ekko.register_surface({
            name = "lua-bar",
            dock = "bottom",
            size = 1,
            draw = function(ctx, snapshot)
              local cols, rows = ctx.size()
              ctx.fill_rect(0, 0, cols, rows, "text", "surface")
              ctx.put_text(2, 0, 20, "accent", "#102030", "hello " .. snapshot.session_name)
              ctx.put_text_bold(30, 0, 10, "text", "surface", "B")
              ctx.set_cell(0, 0, "text", "surface", "|", true)
              ctx.draw_box(4, 0, 10, 1, "text", "surface", "border")
            end,
          })
        end
        return ext
        "##,
    );
    let spec = runtime.surface("lua-bar").expect("surface registered");
    let mut recorder = Recorder::default();
    (spec.draw)(&mut recorder, &snapshot());
    assert_eq!(
        recorder.calls,
        vec![
            "fill 0 0 80 1",
            "styled 2 0 hello main r=false b=false",
            "styled 30 0 B r=false b=true",
            "cell 0 0 |",
            "box 10 1",
        ]
    );
}

#[test]
fn scrollbar_op_marshals_model_style_and_glyph_defaults() {
    let runtime = runtime(
        r##"
        local ext = { id = "user.sb" }
        function ext.register(ekko)
          ekko.register_surface({
            name = "lua-list",
            dock = "left",
            size = 20,
            draw = function(ctx, snapshot)
              ctx.render_scrollbar({
                col = 19, row = 0, rows = 10,
                visible = 10, total = 40, from_top = 5,
                fg = "border", bg = "surface", thumb_fg = "accent",
              })
              ctx.render_scrollbar({
                col = 19, row = 0, rows = 10,
                visible = 10, total = 40, from_top = 5,
                fg = "border", bg = "surface", thumb_fg = "accent",
                track = ".", thumb = "#",
              })
            end,
          })
        end
        return ext
        "##,
    );
    let spec = runtime.surface("lua-list").expect("surface registered");
    let mut recorder = Recorder::default();
    (spec.draw)(&mut recorder, &snapshot());
    assert_eq!(
        recorder.calls,
        vec![
            "scrollbar 19 0 10 10/40@5 │█",
            "scrollbar 19 0 10 10/40@5 .#"
        ]
    );
}

#[test]
fn surface_scaled_size_and_hide_below_are_marshaled() {
    let runtime = runtime(
        r##"
        local ext = { id = "user.sized" }
        function ext.register(ekko)
          ekko.register_surface({
            name = "scaled",
            dock = "left",
            size = { preferred = 30, min = 10, fraction = 3, min_remaining = 20 },
            hide_below = { cols = 64, rows = 4 },
            draw = function(ctx, snapshot) end,
          })
          ekko.register_surface({
            name = "scaled-defaults",
            dock = "left",
            size = { preferred = 12 },
            hide_below = { cols = 40 },
            draw = function(ctx, snapshot) end,
          })
        end
        return ext
        "##,
    );
    let spec = runtime.surface("scaled").expect("surface registered");
    assert_eq!(
        spec.size,
        SurfaceSize::Scaled {
            preferred: 30,
            min: 10,
            max_fraction_denom: 3,
            min_remaining: 20,
        }
    );
    assert_eq!(spec.hide_below, Some((64, 4)));

    // Omitted Scaled fields default to layout no-ops; omitted hide_below
    // fields default to 0 (no constraint on that axis).
    let spec = runtime
        .surface("scaled-defaults")
        .expect("surface registered");
    assert_eq!(
        spec.size,
        SurfaceSize::Scaled {
            preferred: 12,
            min: 0,
            max_fraction_denom: 1,
            min_remaining: 0,
        }
    );
    assert_eq!(spec.hide_below, Some((40, 0)));
}

#[test]
fn surface_scaled_size_without_preferred_is_a_registration_error() {
    let ext = LuaExtension::from_source(
        "nopref.lua",
        r#"
        local ext = { id = "user.nopref" }
        function ext.register(ekko)
          ekko.register_surface({
            name = "broken",
            dock = "left",
            size = { min = 10 },
            draw = function(ctx, snapshot) end,
          })
        end
        return ext
        "#,
    )
    .expect("script loads");
    assert!(
        RuntimeBuilder::new()
            .register_boxed_extension(Box::new(ext))
            .build()
            .is_err()
    );
}

#[test]
fn runaway_draw_hits_the_instruction_budget_and_draws_nothing() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.spin" }
        function ext.register(ekko)
          ekko.register_surface({
            name = "spinner",
            dock = "bottom",
            size = 1,
            draw = function(ctx, snapshot)
              ctx.put_text(0, 0, 10, "text", "surface", "before")
              while true do end
            end,
          })
        end
        return ext
        "#,
    );
    let spec = runtime.surface("spinner").expect("surface registered");
    let mut recorder = Recorder::default();
    // Must return (budget abort), and the buffered op from before the hang
    // must NOT reach the context.
    (spec.draw)(&mut recorder, &snapshot());
    assert!(recorder.calls.is_empty());
}

#[test]
fn runaway_command_errors_instead_of_hanging() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.loop" }
        function ext.register(ekko)
          ekko.register_command({
            name = "hang",
            handler = function(args) while true do end end,
          })
        end
        return ext
        "#,
    );
    match runtime.invoke_command(":hang") {
        CommandDispatch::Failed(message) => {
            assert!(message.contains("instruction budget exceeded"), "{message}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn config_raises_the_instruction_budgets() {
    // WS-D acceptance: the budgets are `[lua]` config, not constants. One
    // loop heavy enough to blow both defaults (3M iterations is > the 2M
    // handler budget and >> the 200k draw budget) fails under
    // `Config::default()` and completes once the config raises the budgets
    // — pinning both that the defaults still bind and that the raise
    // actually reaches every callback path.
    let script = r#"
        local ext = { id = "user.heavy" }
        local function grind()
          local n = 0
          for i = 1, 3000000 do n = n + 1 end
          return n
        end
        function ext.register(ekko)
          ekko.register_command({
            name = "heavy",
            handler = function(args)
              return { { set_status_note = { text = "n=" .. grind(), kind = "ok", ttl_ms = 1000 } } }
            end,
          })
          ekko.register_surface({
            name = "heavy-bar", dock = "bottom", size = 1,
            draw = function(ctx, snapshot)
              ctx.put_text(0, 0, 20, "text", "surface", "n=" .. grind())
            end,
          })
        end
        return ext
    "#;

    let default_runtime = runtime(script);
    match default_runtime.invoke_command(":heavy") {
        CommandDispatch::Failed(message) => {
            assert!(message.contains("instruction budget exceeded"), "{message}");
        }
        other => panic!("expected Failed under default budget, got {other:?}"),
    }
    let mut recorder = Recorder::default();
    (default_runtime.surface("heavy-bar").unwrap().draw)(&mut recorder, &snapshot());
    assert!(recorder.calls.is_empty());

    let mut ext = LuaExtension::from_source("heavy.lua", script).expect("script loads");
    let mut config = ekko_config::Config::default();
    config.lua.handler_budget = 50_000_000;
    config.lua.draw_budget = 50_000_000;
    ext.set_config(&config);
    let raised_runtime = RuntimeBuilder::new()
        .register_boxed_extension(Box::new(ext))
        .build()
        .expect("runtime builds");
    assert_eq!(
        raised_runtime.invoke_command(":heavy"),
        CommandDispatch::Invoked(vec![UiAction::SetStatusNote {
            text: "n=3000000".into(),
            kind: NoteKind::Ok,
            ttl_ms: 1000,
        }])
    );
    let mut recorder = Recorder::default();
    (raised_runtime.surface("heavy-bar").unwrap().draw)(&mut recorder, &snapshot());
    assert_eq!(
        recorder.calls,
        vec!["styled 0 0 n=3000000 r=false b=false".to_string()]
    );
}

#[test]
fn surface_visibility_predicate_is_bridged() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.vis" }
        function ext.register(ekko)
          ekko.register_surface({
            name = "sometimes",
            dock = "top",
            size = 1,
            visible = function(snapshot) return snapshot.mode ~= "normal" end,
            draw = function(ctx, snapshot) end,
          })
        end
        return ext
        "#,
    );
    let normal = snapshot();
    assert!(runtime.visible_surfaces(&normal).is_empty());
    let mut command_mode = snapshot();
    command_mode.mode = "command".into();
    assert_eq!(runtime.visible_surfaces(&command_mode).len(), 1);
}

#[test]
fn subscriptions_observe_and_gate() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.guard" }
        function ext.register(ekko)
          ekko.subscribe("command_invoked", function(payload)
            if payload.name == "kill" then
              return { cancel = "protected by lua" }
            end
          end)
          ekko.subscribe("key_input", function(payload)
            if payload.bytes == "Z" then
              return { consume = true }
            end
            if payload.bytes == "a" then
              return { transform = "b" }
            end
          end)
        end
        return ext
        "#,
    );
    let cancels = runtime.dispatch_cancelable(
        EventKind::CommandInvoked,
        EventPayload::CommandInvoked {
            name: "kill".into(),
            raw_args: String::new(),
        },
    );
    assert_eq!(cancels, Some("protected by lua".into()));
    let returns = runtime.dispatch(
        EventKind::KeyInput,
        EventPayload::KeyInput {
            bytes: b"Z".to_vec(),
        },
    );
    assert!(matches!(
        returns[0],
        EventReturn::KeyIntercept(KeyIntercept::Consume)
    ));
    let returns = runtime.dispatch(
        EventKind::KeyInput,
        EventPayload::KeyInput {
            bytes: b"a".to_vec(),
        },
    );
    assert!(
        matches!(&returns[0], EventReturn::KeyIntercept(KeyIntercept::Transform(b)) if b == b"b")
    );
}

#[test]
fn every_event_payload_marshals_to_lua() {
    // `payload_table`'s match is exhaustive, so a new `EventPayload` variant
    // fails compilation until it renders; this pins the table shape each
    // existing variant presents to scripts. One handler subscribed to every
    // canonical event name echoes the payload back as a sorted `key=value`
    // line through the `notice` return.
    let names: String = EventKind::ALL
        .iter()
        .map(|kind| format!("\"{}\", ", kind.name()))
        .collect();
    let source = r#"
        local ext = { id = "user.echo" }
        function ext.register(ekko)
          local function describe(payload)
            local keys = {}
            for key in pairs(payload) do keys[#keys + 1] = key end
            table.sort(keys)
            local parts = {}
            for _, key in ipairs(keys) do
              parts[#parts + 1] = key .. "=" .. tostring(payload[key])
            end
            return table.concat(parts, " ")
          end
          for _, name in ipairs({ EVENT_NAMES }) do
            ekko.subscribe(name, function(payload)
              return { notice = { message = describe(payload) } }
            end)
          end
        end
        return ext
        "#
    .replace("EVENT_NAMES", &names);
    let runtime = runtime(&source);

    let cases: Vec<(EventKind, EventPayload, &str)> = vec![
        (EventKind::ClientReady, EventPayload::Empty, ""),
        (
            EventKind::SessionAttached,
            EventPayload::SessionAttached {
                session_name: "main".into(),
                wire_version: 3,
            },
            "session_name=main wire_version=3",
        ),
        (
            EventKind::BeforeSessionDetach,
            EventPayload::BeforeSessionDetach {
                session_name: "main".into(),
            },
            "session_name=main",
        ),
        (
            EventKind::BeforeSessionSwitch,
            EventPayload::SessionSwitch {
                from: "a".into(),
                to: "b".into(),
            },
            "from=a to=b",
        ),
        (
            EventKind::GridUpdated,
            EventPayload::GridUpdated {
                epoch: 7,
                cols: 80,
                rows: 24,
            },
            "cols=80 epoch=7 rows=24",
        ),
        (
            EventKind::Resize,
            EventPayload::Resize {
                cols: 120,
                rows: 40,
            },
            "cols=120 rows=40",
        ),
        (
            EventKind::Tick,
            EventPayload::Tick { now_ms: 99 },
            "now_ms=99",
        ),
        (
            EventKind::KeyInput,
            EventPayload::KeyInput {
                bytes: b"x".to_vec(),
            },
            "bytes=x",
        ),
        (
            EventKind::ModeChanged,
            EventPayload::ModeChanged {
                from: "normal".into(),
                to: "scroll".into(),
            },
            "from=normal to=scroll",
        ),
        (
            EventKind::CommandInvoked,
            EventPayload::CommandInvoked {
                name: "split".into(),
                raw_args: "-h".into(),
            },
            "name=split raw_args=-h",
        ),
        (
            EventKind::BeforePtySpawn,
            EventPayload::PtySpawn {
                session_name: "main".into(),
                shell: "/bin/sh".into(),
                cwd: "/tmp".into(),
                cols: 80,
                rows: 24,
            },
            "cols=80 cwd=/tmp rows=24 session_name=main shell=/bin/sh",
        ),
        (
            EventKind::SessionCreated,
            EventPayload::SessionCreated {
                session_name: "main".into(),
                shell: "/bin/sh".into(),
                cwd: "/tmp".into(),
            },
            "cwd=/tmp session_name=main shell=/bin/sh",
        ),
        (
            EventKind::ClientAttached,
            EventPayload::ClientAttached {
                session_name: "main".into(),
                client_id: 7,
                cols: 80,
                rows: 24,
            },
            "client_id=7 cols=80 rows=24 session_name=main",
        ),
        (
            EventKind::ClientDetached,
            EventPayload::ClientDetached {
                session_name: "main".into(),
                client_id: 7,
            },
            "client_id=7 session_name=main",
        ),
        (
            EventKind::SessionExited,
            EventPayload::SessionExited {
                session_name: "main".into(),
                exit_code: Some(0),
                reason: SessionExitReason::ShellExited,
            },
            "exit_code=0 reason=shell_exited session_name=main",
        ),
        // A `None` exit code is an absent key, not an error or a 0.
        (
            EventKind::SessionExited,
            EventPayload::SessionExited {
                session_name: "main".into(),
                exit_code: None,
                reason: SessionExitReason::Killed,
            },
            "reason=killed session_name=main",
        ),
        (
            EventKind::PtyResized,
            EventPayload::PtyResized {
                session_name: "main".into(),
                cols: 80,
                rows: 24,
            },
            "cols=80 rows=24 session_name=main",
        ),
        (
            EventKind::Heartbeat,
            EventPayload::Heartbeat {
                session_name: "main".into(),
            },
            "session_name=main",
        ),
        (
            EventKind::Bell,
            EventPayload::Bell {
                session_name: "main".into(),
            },
            "session_name=main",
        ),
    ];
    for (kind, payload, expected) in cases {
        let returns = runtime.dispatch(kind, payload);
        let [EventReturn::EmitNotice { message, .. }] = returns.as_slice() else {
            panic!("expected one notice for {kind:?}, got {returns:?}");
        };
        assert_eq!(message, expected, "payload table for {kind:?}");
    }
}

#[test]
fn spawn_override_returns_marshal_shell_cwd_and_env() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.spawnhook" }
        function ext.register(ekko)
          ekko.subscribe("before_pty_spawn", function(payload)
            return {
              spawn_override = {
                shell = "/bin/zsh",
                cwd = "/tmp",
                env = { EKKO_HOOKED = payload.session_name },
              },
            }
          end)
        end
        return ext
        "#,
    );
    let returns = runtime.dispatch(
        EventKind::BeforePtySpawn,
        EventPayload::PtySpawn {
            session_name: "main".into(),
            shell: "/bin/sh".into(),
            cwd: "/home".into(),
            cols: 80,
            rows: 24,
        },
    );
    let [EventReturn::PtySpawnOverride { shell, cwd, env }] = returns.as_slice() else {
        panic!("expected a spawn override, got {returns:?}");
    };
    assert_eq!(shell.as_deref(), Some(Path::new("/bin/zsh")));
    assert_eq!(cwd.as_deref(), Some(Path::new("/tmp")));
    assert_eq!(env, &[("EKKO_HOOKED".to_string(), "main".to_string())]);
}

#[test]
fn budget_blowout_on_before_pty_spawn_degrades_to_no_override() {
    // A runaway `BeforePtySpawn` gate hits the instruction budget (pure Lua
    // looping, so the hook aborts it long before the dispatch timeout). The
    // dispatcher logs and skips it — the spawn proceeds without an override
    // rather than failing. The well-behaved gate registered *after* it in
    // the same script pins that the budget abort released the script's Lua
    // state cleanly.
    let runtime = runtime(
        r#"
        local ext = { id = "user.blowout" }
        function ext.register(ekko)
          ekko.subscribe("before_pty_spawn", function(payload)
            while true do end
          end)
          ekko.subscribe("before_pty_spawn", function(payload)
            return { spawn_override = { cwd = "/srv" } }
          end)
        end
        return ext
        "#,
    );
    let returns = runtime.dispatch(
        EventKind::BeforePtySpawn,
        EventPayload::PtySpawn {
            session_name: "main".into(),
            shell: "/bin/sh".into(),
            cwd: "/home".into(),
            cols: 80,
            rows: 24,
        },
    );
    let [EventReturn::PtySpawnOverride { shell, cwd, env }] = returns.as_slice() else {
        panic!("expected only the well-behaved override, got {returns:?}");
    };
    assert_eq!(shell, &None);
    assert_eq!(cwd.as_deref(), Some(Path::new("/srv")));
    assert!(env.is_empty());
}

#[test]
fn abandoned_lua_lock_is_skipped_by_timeout_and_spares_other_scripts() {
    // The one hole the instruction budget cannot cover: a single C call
    // (here: pathological pattern backtracking) never yields to the hook,
    // so it outlives the dispatch timeout and the detached thread keeps the
    // script's Lua lock. Subsequent callbacks into the same script must
    // fail cleanly — blocked on the lock, timed out, logged, skipped — and
    // other scripts (separate Lua states, separate locks) must be
    // completely unaffected. This is the daemon's containment story for a
    // wedged server script degrading exactly one extension, never the hub.
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let sentinel = std::env::temp_dir().join(format!("ekko-lua-stall-{}", std::process::id()));
    let _ = std::fs::remove_file(&sentinel);
    let stuck = r#"
        local ext = { id = "user.stuck" }
        function ext.register(ekko)
          ekko.register_command({
            name = "stall",
            handler = function(args)
              local f = assert(io.open("SENTINEL", "w"))
              f:write("locked")
              f:close()
              -- One C call the instruction hook cannot interrupt; holds
              -- this script's lock far past every dispatch timeout.
              string.find(string.rep("a", 300), "a-a-a-a-ab")
            end,
          })
          ekko.subscribe("bell", function(payload)
            return { notice = { message = "stuck answered" } }
          end)
        end
        return ext
        "#
    .replace("SENTINEL", &sentinel.display().to_string());
    let healthy = r#"
        local ext = { id = "user.healthy" }
        function ext.register(ekko)
          ekko.subscribe("bell", function(payload)
            return { notice = { message = "healthy answered" } }
          end)
        end
        return ext
        "#;
    let runtime = Arc::new(
        RuntimeBuilder::new()
            .register_boxed_extension(Box::new(
                LuaExtension::from_source("stuck.lua", &stuck).expect("script loads"),
            ))
            .register_boxed_extension(Box::new(
                LuaExtension::from_source("healthy.lua", healthy).expect("script loads"),
            ))
            .build()
            .expect("runtime builds"),
    );

    // Wedge user.stuck on its own thread; the sentinel file is written
    // under the lock, so once it exists the lock is held.
    {
        let runtime = runtime.clone();
        std::thread::spawn(move || {
            let _ = runtime.invoke_command(":stall");
        });
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !sentinel.exists() {
        assert!(Instant::now() < deadline, "stall handler never started");
        std::thread::sleep(Duration::from_millis(10));
    }

    let start = Instant::now();
    let returns = runtime.dispatch_labeled(
        EventKind::Bell,
        EventPayload::Bell {
            session_name: "main".into(),
        },
    );
    // user.stuck's bell handler blocked on the abandoned lock and was timed
    // out; user.healthy answered normally; the dispatcher never wedged.
    assert_eq!(returns.len(), 1, "got {returns:?}");
    assert_eq!(returns[0].0, "user.healthy:bell");
    assert!(matches!(
        &returns[0].1,
        EventReturn::EmitNotice { message, .. } if message == "healthy answered"
    ));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "dispatch must time the blocked handler out, not wait for the lock"
    );
    let _ = std::fs::remove_file(&sentinel);
}

#[test]
fn overlays_carry_lua_state_and_payloads() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.overlay" }
        function ext.register(ekko)
          ekko.register_overlay({
            name = "counter",
            description = "counts keys",
            build_payload = function(registries)
              return { label = "commands: " .. #registries.commands }
            end,
            init = function(payload)
              return { label = payload.label, count = 0 }
            end,
            render = function(ctx, state, snapshot)
              ctx.put_text(0, 0, 40, "text", "surface", state.label .. " count " .. state.count)
            end,
            handle_key = function(state, bytes)
              if bytes == "q" then return "close" end
              state.count = state.count + 1
            end,
          })
          ekko.register_command({ name = "noop", handler = function(args) end })
        end
        return ext
        "#,
    );
    let spec = runtime.overlay("counter").expect("overlay registered");

    let payload = spec.build_payload.as_ref().expect("payload builder")(&runtime.registry_view());
    let mut state = (spec.init_state)(Some(payload));

    // Two counted keys, then render must reflect the mutated Lua state.
    assert_eq!(
        (spec.handle_key)(&mut state, b"x"),
        ekko_ext::OverlayOutcome::None
    );
    assert_eq!(
        (spec.handle_key)(&mut state, b"y"),
        ekko_ext::OverlayOutcome::None
    );
    let mut recorder = Recorder::default();
    (spec.render)(&mut recorder, &mut state, &snapshot());
    assert_eq!(
        recorder.calls,
        vec!["styled 0 0 commands: 1 count 2 r=false b=false"]
    );

    assert_eq!(
        (spec.handle_key)(&mut state, b"q"),
        ekko_ext::OverlayOutcome::Close
    );
}

#[test]
fn overlays_can_attach_to_a_mode() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.attached" }
        function ext.register(ekko)
          ekko.register_overlay({
            name = "session-panel",
            description = "session list, tied to the leader panel",
            attach_mode = "leader",
            init = function() return {} end,
            render = function(ctx, state, snapshot) end,
            handle_key = function(state, bytes) end,
          })
        end
        return ext
        "#,
    );
    let spec = runtime
        .overlay("session-panel")
        .expect("overlay registered");
    assert_eq!(spec.attach_mode.as_deref(), Some("leader"));
    assert_eq!(
        runtime
            .overlay_attached_to("leader")
            .map(|o| o.name.as_str()),
        Some("session-panel")
    );
}

#[test]
fn mode_outcome_dialect_round_trips() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.dialect" }
        function ext.register(ekko)
          ekko.register_mode({
            name = "dialect",
            on_key = function(state, bytes, snapshot)
              if bytes == "n" then return nil end
              if bytes == "e" then return "exit" end
              if bytes == "s" then return { scroll = -1 } end
              if bytes == "a" then return { { scroll = 1 }, "detach" } end
              if bytes == "x" then return { "exit", { switch_session = "other" } } end
              if bytes == "X" then return { "exit" } end
              if bytes == "u" then return "not-an-outcome" end
              if bytes == "b" then return 42 end
            end,
          })
        end
        return ext
        "#,
    );
    let spec = runtime.mode("dialect").expect("mode registered");
    let cases: Vec<(&[u8], ekko_ext::ModeOutcome)> = vec![
        (b"n", ekko_ext::ModeOutcome::Continue),
        (b"e", ekko_ext::ModeOutcome::Exit),
        (
            b"s",
            ekko_ext::ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: -1 }]),
        ),
        (
            b"a",
            ekko_ext::ModeOutcome::ContinueWith(vec![
                UiAction::Scroll { delta: 1 },
                UiAction::Detach,
            ]),
        ),
        (
            b"x",
            ekko_ext::ModeOutcome::ExitWith(vec![UiAction::SwitchSession {
                name: "other".into(),
            }]),
        ),
        (b"X", ekko_ext::ModeOutcome::Exit),
        // Unrecognized returns degrade to Continue, never trap or exit.
        (b"u", ekko_ext::ModeOutcome::Continue),
        (b"b", ekko_ext::ModeOutcome::Continue),
    ];
    for (bytes, expected) in cases {
        let mut state = (spec.init_state)();
        assert_eq!(
            (spec.on_key)(&mut state, bytes, &snapshot()),
            expected,
            "bytes {bytes:?}"
        );
    }
}

#[test]
fn modes_carry_lua_state_and_render_a_cursor() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.jump" }
        function ext.register(ekko)
          ekko.register_mode({
            name = "jump",
            init = function() return { input = "" } end,
            on_key = function(state, bytes, snapshot)
              if bytes == "\27" then return "exit" end
              state.input = state.input .. bytes
            end,
            render = function(ctx, state, snapshot)
              if state.input == "" then return nil end
              ctx.put_text(0, 0, 40, "text", "surface", "jump: " .. state.input)
              return { row = 0, col = 6 + #state.input }
            end,
          })
        end
        return ext
        "#,
    );
    let spec = runtime.mode("jump").expect("mode registered");
    let render = spec.render.as_ref().expect("mode has a render fn");
    let mut state = (spec.init_state)();

    // Empty state: nothing drawn, no cursor.
    let mut recorder = Recorder::default();
    assert_eq!(render(&mut recorder, &state, &snapshot()), None);
    assert!(recorder.calls.is_empty());

    // Two keys mutate the registry-held Lua state in place.
    assert_eq!(
        (spec.on_key)(&mut state, b"a", &snapshot()),
        ekko_ext::ModeOutcome::Continue
    );
    assert_eq!(
        (spec.on_key)(&mut state, b"b", &snapshot()),
        ekko_ext::ModeOutcome::Continue
    );
    let mut recorder = Recorder::default();
    assert_eq!(render(&mut recorder, &state, &snapshot()), Some((0, 8)));
    assert_eq!(recorder.calls, vec!["styled 0 0 jump: ab r=false b=false"]);

    assert_eq!(
        (spec.on_key)(&mut state, b"\x1b", &snapshot()),
        ekko_ext::ModeOutcome::Exit
    );
}

#[test]
fn default_mode_state_is_a_table_and_broken_handlers_exit() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.modes" }
        function ext.register(ekko)
          -- No init: the bridge hands on_key a fresh empty table.
          ekko.register_mode({
            name = "counting",
            on_key = function(state, bytes, snapshot)
              state.n = (state.n or 0) + 1
              if state.n >= 2 then return "exit" end
            end,
          })
          ekko.register_mode({
            name = "broken",
            on_key = function(state, bytes, snapshot) error("boom") end,
          })
          ekko.register_mode({
            name = "runaway",
            on_key = function(state, bytes, snapshot) while true do end end,
          })
        end
        return ext
        "#,
    );
    let spec = runtime.mode("counting").expect("mode registered");
    let mut state = (spec.init_state)();
    assert_eq!(
        (spec.on_key)(&mut state, b"x", &snapshot()),
        ekko_ext::ModeOutcome::Continue
    );
    assert_eq!(
        (spec.on_key)(&mut state, b"x", &snapshot()),
        ekko_ext::ModeOutcome::Exit
    );

    // A broken or runaway mode must not trap input: both bail out.
    for name in ["broken", "runaway"] {
        let spec = runtime.mode(name).expect("mode registered");
        let mut state = (spec.init_state)();
        assert_eq!(
            (spec.on_key)(&mut state, b"x", &snapshot()),
            ekko_ext::ModeOutcome::Exit,
            "mode {name}"
        );
    }
}

/// The A-acceptance proof for modes: `examples/scroll-mode.lua` re-registers
/// the scroll-mode builtin's key policy, byte for byte. Drives the example
/// through the same cases as the builtin's own tests.
#[test]
fn scroll_mode_example_matches_the_builtin_policy() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/scroll-mode.lua"
    );
    let runtime = RuntimeBuilder::new()
        .register_boxed_extension(Box::new(
            LuaExtension::from_file(std::path::Path::new(path)).expect("example loads"),
        ))
        .build()
        .expect("runtime builds");
    let spec = runtime.mode("scroll").expect("scroll mode registered");
    // snapshot() has grid_rows = 23: half page 11, full page 23.
    let cases: Vec<(&[u8], ekko_ext::ModeOutcome)> = vec![
        (
            b"k",
            ekko_ext::ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: 1 }]),
        ),
        (
            b"j",
            ekko_ext::ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: -1 }]),
        ),
        (
            b"u",
            ekko_ext::ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: 11 }]),
        ),
        (
            b"\x1b[6~",
            ekko_ext::ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: -23 }]),
        ),
        (
            b"g",
            ekko_ext::ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: i32::MAX }]),
        ),
        (
            b"G",
            ekko_ext::ModeOutcome::ContinueWith(vec![UiAction::ScrollToBottom]),
        ),
        (
            b"\x1b[<64;10;5M",
            ekko_ext::ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: 3 }]),
        ),
        (
            b"\x1b[<65;10;5M",
            ekko_ext::ModeOutcome::ContinueWith(vec![UiAction::Scroll { delta: -3 }]),
        ),
        (
            b"q",
            ekko_ext::ModeOutcome::ExitWith(vec![UiAction::ScrollToBottom]),
        ),
        (
            b"\x1b",
            ekko_ext::ModeOutcome::ExitWith(vec![UiAction::ScrollToBottom]),
        ),
        (b"x", ekko_ext::ModeOutcome::Continue),
    ];
    for (bytes, expected) in cases {
        let mut state = (spec.init_state)();
        assert_eq!(
            (spec.on_key)(&mut state, bytes, &snapshot()),
            expected,
            "bytes {bytes:?}"
        );
    }
}

#[test]
fn themes_register_with_hex_overrides() {
    let runtime = runtime(
        r##"
        local ext = { id = "user.theme" }
        function ext.register(ekko)
          ekko.register_theme({
            name = "lua-dark",
            palette = {
              text = "#aabbcc",
              accent = "#010203",
              selection_fg = "#f0f1f2",
              selection_bg = "#303132",
            },
          })
        end
        return ext
        "##,
    );
    let theme = runtime.theme("lua-dark").expect("theme registered");
    assert_eq!(theme.palette.text, Color::rgb(0xaa, 0xbb, 0xcc));
    assert_eq!(theme.palette.accent, Color::rgb(0x01, 0x02, 0x03));
    assert_eq!(theme.palette.selection_fg, Color::rgb(0xf0, 0xf1, 0xf2));
    assert_eq!(theme.palette.selection_bg, Color::rgb(0x30, 0x31, 0x32));
    // Untouched fields keep the fallback.
    assert_eq!(theme.palette.muted, ThemePalette::fallback().muted);
}

#[test]
fn spinners_register_as_pure_data() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.spin" }
        function ext.register(ekko)
          ekko.register_spinner({ name = "dots", frames = { "a", "b", "c" }, interval_ms = 100 })
          ekko.register_spinner({ name = "blink", frames = { "*" } })
        end
        return ext
        "#,
    );
    let spinner = runtime.spinner("dots").expect("spinner registered");
    assert_eq!(*spinner.frames, vec!["a", "b", "c"]);
    assert_eq!(spinner.interval_ms, 100);
    assert_eq!(spinner.frame_at(0), "a");
    assert_eq!(spinner.frame_at(150), "b");
    assert_eq!(spinner.frame_at(300), "a");
    // interval_ms defaults when omitted.
    assert_eq!(
        runtime.spinner("blink").expect("registered").interval_ms,
        80
    );

    // A frameless spinner is a registration error, not a blank animation.
    let empty = LuaExtension::from_source(
        "empty.lua",
        r#"
        local ext = { id = "user.empty" }
        function ext.register(ekko)
          ekko.register_spinner({ name = "void", frames = {} })
        end
        return ext
        "#,
    )
    .expect("script loads");
    assert!(
        RuntimeBuilder::new()
            .register_boxed_extension(Box::new(empty))
            .build()
            .is_err()
    );
}

fn session_entries() -> Vec<SessionEntry> {
    let entry = |name: &str, cwd: &str| SessionEntry {
        name: name.into(),
        cwd: cwd.into(),
        state: SessionState::Alive,
        created_at_secs: 7,
    };
    vec![
        entry("api", "/work/api"),
        entry("web", "/work/web"),
        entry("scratch", "/tmp"),
    ]
}

#[test]
fn session_grouper_rehydrates_by_name_and_keeps_unclaimed_sessions() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.grouper" }
        function ext.register(ekko)
          ekko.register_session_grouper({
            name = "by-root",
            group = function(sessions)
              local work = {}
              for _, s in ipairs(sessions) do
                if s.cwd:sub(1, 6) == "/work/" and s.state == "alive" then
                  table.insert(work, s.name)
                end
              end
              return {
                { name = "work", sessions = work },
                -- Fabricated names and repeat claims are dropped, so this
                -- group rehydrates empty and disappears.
                { name = "ghost", sessions = { "no-such-session", "api" } },
              }
            end,
          })
        end
        return ext
        "#,
    );
    let spec = runtime.session_grouper().expect("grouper registered");
    assert_eq!(spec.name, "by-root");
    let groups = (spec.group)(session_entries());
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].name, "work");
    assert_eq!(
        groups[0]
            .sessions
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec!["api", "web"]
    );
    // "scratch" was never claimed: it must not vanish from the sidebar.
    assert_eq!(groups[1].name, "ungrouped");
    assert_eq!(groups[1].sessions[0].name, "scratch");
    assert_eq!(groups[1].sessions[0].created_at_secs, 7);
}

#[test]
fn broken_session_groupers_degrade_to_the_flat_fallback() {
    let runtime = runtime(
        r#"
        local ext = { id = "user.badgroup" }
        function ext.register(ekko)
          ekko.register_session_grouper({
            name = "broken",
            group = function(sessions) error("boom") end,
          })
        end
        return ext
        "#,
    );
    let spec = runtime.session_grouper().expect("grouper registered");
    let groups = (spec.group)(session_entries());
    // Exactly the no-grouper shape: one flat, name-sorted "sessions" group.
    assert_eq!(groups, ekko_ext::fallback_group(session_entries()));
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "sessions");
    assert_eq!(
        groups[0]
            .sessions
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec!["api", "scratch", "web"]
    );
}

#[test]
fn broken_scripts_fail_to_load_but_duplicates_fail_the_build() {
    assert!(LuaExtension::from_source("bad.lua", "this is not lua").is_err());
    assert!(
        LuaExtension::from_source("no-id.lua", "return { register = function() end }").is_err()
    );

    // Duplicate command names across a lua script and another extension are
    // a hard build error, same as native-vs-native.
    let script = r#"
        local ext = { id = "user.dupe" }
        function ext.register(ekko)
          ekko.register_command({ name = "same", handler = function(args) end })
          ekko.register_command({ name = "same", handler = function(args) end })
        end
        return ext
    "#;
    let result = RuntimeBuilder::new()
        .register_boxed_extension(Box::new(
            LuaExtension::from_source("dupe.lua", script).unwrap(),
        ))
        .build();
    assert!(result.is_err());
}

#[test]
fn load_extensions_skips_broken_files_and_loads_good_ones() {
    let dir = std::env::temp_dir().join(format!("ekko-lua-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("good.lua"),
        r#"return { id = "user.good", register = function(ekko) end }"#,
    )
    .unwrap();
    std::fs::write(dir.join("broken.lua"), "not lua at all").unwrap();
    std::fs::write(dir.join("ignored.txt"), "not a script").unwrap();

    let extensions = ekko_lua::load_extensions(
        &dir,
        ekko_lua::HostKind::Client,
        &ekko_config::Config::default(),
    );
    assert_eq!(extensions.len(), 1);
    assert_eq!(extensions[0].manifest().id, "user.good");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn host_declaration_filters_where_a_script_loads() {
    let dir = std::env::temp_dir().join(format!("ekko-lua-host-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("client.lua"),
        r#"return { id = "user.client", register = function(ekko) end }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("server.lua"),
        r#"return { id = "user.server", host = "server", register = function(ekko) end }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("shared.lua"),
        r#"return { id = "user.shared", host = "both", register = function(ekko) end }"#,
    )
    .unwrap();

    let ids = |host| -> Vec<String> {
        ekko_lua::load_extensions(&dir, host, &ekko_config::Config::default())
            .iter()
            .map(|e| e.manifest().id)
            .collect()
    };
    // No `host` field defaults to client-only; "both" loads everywhere.
    assert_eq!(
        ids(ekko_lua::HostKind::Client),
        ["user.client", "user.shared"]
    );
    assert_eq!(
        ids(ekko_lua::HostKind::Server),
        ["user.server", "user.shared"]
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unknown_host_declarations_are_load_errors() {
    let bad_value = r#"return { id = "user.x", host = "daemon", register = function(ekko) end }"#;
    let err = LuaExtension::from_source("bad-host.lua", bad_value)
        .map(|_| ())
        .unwrap_err();
    assert!(err.to_string().contains("unknown host 'daemon'"), "{err}");

    let bad_type = r#"return { id = "user.x", host = true, register = function(ekko) end }"#;
    let err = LuaExtension::from_source("bad-host-type.lua", bad_type)
        .map(|_| ())
        .unwrap_err();
    assert!(err.to_string().contains("must be a string"), "{err}");
}
