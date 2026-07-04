//! End-to-end tests of the Lua bridge: scripts register through the real
//! `RuntimeBuilder`, callbacks round-trip through the real `AppRuntime`
//! dispatch paths, and the guard rails (instruction budgets, buffered draw
//! ops) actually hold.

use ekko_ext::{
    AppRuntime, ClientSnapshot, Color, CommandDispatch, DrawContext, EventKind, EventPayload,
    EventReturn, KeyIntercept, NoteKind, Rect, RuntimeBuilder, SessionEntry, SessionState,
    SurfaceSize, ThemePalette, UiAction,
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
            palette = { text = "#aabbcc", accent = "#010203" },
          })
        end
        return ext
        "##,
    );
    let theme = runtime.theme("lua-dark").expect("theme registered");
    assert_eq!(theme.palette.text, Color::rgb(0xaa, 0xbb, 0xcc));
    assert_eq!(theme.palette.accent, Color::rgb(0x01, 0x02, 0x03));
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

    let extensions = ekko_lua::load_extensions(&dir, ekko_lua::HostKind::Client);
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
        ekko_lua::load_extensions(&dir, host)
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
