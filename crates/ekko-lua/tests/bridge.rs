//! End-to-end tests of the Lua bridge: scripts register through the real
//! `RuntimeBuilder`, callbacks round-trip through the real `AppRuntime`
//! dispatch paths, and the guard rails (instruction budgets, buffered draw
//! ops) actually hold.

use ekko_ext::{
    AppRuntime, ClientSnapshot, Color, CommandDispatch, DrawContext, EventKind, EventPayload,
    EventReturn, KeyIntercept, NoteKind, Rect, RuntimeBuilder, ThemePalette, UiAction,
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
        _col: i32,
        _row: i32,
        _rows: i32,
        _model: ekko_ext::ScrollbarModel,
        _style: ekko_ext::ScrollbarStyle<'_>,
    ) {
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

    let extensions = ekko_lua::load_extensions(&dir);
    assert_eq!(extensions.len(), 1);
    assert_eq!(extensions[0].manifest().id, "user.good");

    std::fs::remove_dir_all(&dir).unwrap();
}
