//! `init.lua` as the settings source: round-trip into `ekko_config::Config`,
//! precedence over a coexisting `config.toml`, the instruction budget on
//! evaluation, hard errors for broken files, and the read-only `ekko.config`
//! table scripts see.

use std::path::PathBuf;

use ekko_ext::{CommandDispatch, NoteKind, RuntimeBuilder, UiAction};
use ekko_lua::LuaExtension;

/// A fresh per-test temp directory (recreated, so reruns don't see stale
/// files from a crashed prior run).
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ekko-lua-config-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn load_source(tag: &str, source: &str) -> anyhow::Result<ekko_config::Config> {
    let dir = temp_dir(tag);
    std::fs::write(dir.join("init.lua"), source).unwrap();
    let result = ekko_lua::load_config(&dir.join("init.lua"));
    std::fs::remove_dir_all(&dir).unwrap();
    result
}

#[test]
fn init_lua_round_trips_every_section() {
    let config = load_source(
        "full",
        r#"
        return {
          general = { default_shell = "/bin/zsh", scrollback_lines = 500 },
          ui = { sidebar_width = 28, pane_borders = "compact" },
          keybinds = { detach = "ctrl+q", session_next = { "ctrl+j", "ctrl+down" } },
          extensions = { disabled = { "ekko-builtins.sidebar" } },
          lua = { draw_budget = 500000, handler_budget = 4000000 },
        }
        "#,
    )
    .unwrap();
    assert_eq!(config.general.default_shell, "/bin/zsh");
    assert_eq!(config.general.scrollback_lines, 500);
    assert_eq!(config.sidebar_width(), 28);
    assert_eq!(config.ui.pane_borders, ekko_proto::PaneBorderStyle::Compact);
    assert_eq!(
        config.bindings_for("detach", &["ctrl+d"]),
        vec!["ctrl+q".to_string()]
    );
    assert_eq!(
        config.bindings_for("session_next", &[]),
        vec!["ctrl+j".to_string(), "ctrl+down".to_string()]
    );
    assert_eq!(config.extensions.disabled, vec!["ekko-builtins.sidebar"]);
    assert_eq!(config.lua.draw_budget, 500_000);
    assert_eq!(config.lua.handler_budget, 4_000_000);
}

#[test]
fn missing_sections_default_and_nonsense_is_normalized() {
    // Same normalize() pass the TOML path runs: 0 scrollback / 0 budget
    // (which would abort every callback on its first instruction) → default.
    let config = load_source(
        "normalize",
        "return { general = { scrollback_lines = 0 }, lua = { handler_budget = 0 } }",
    )
    .unwrap();
    assert_eq!(
        config.general.scrollback_lines,
        ekko_config::SCROLLBACK_LINES_DEFAULT
    );
    assert_eq!(config.sidebar_width(), ekko_config::SIDEBAR_WIDTH_DEFAULT);
    assert_eq!(
        config.lua.handler_budget,
        ekko_config::LUA_HANDLER_BUDGET_DEFAULT
    );
    assert_eq!(config.lua.draw_budget, ekko_config::LUA_DRAW_BUDGET_DEFAULT);
}

#[test]
fn unknown_keys_warn_but_do_not_fail() {
    // Config files outlive binaries: a key this build doesn't know is a
    // logged warning, and the rest of the table still applies.
    let config = load_source(
        "unknown",
        r#"return { uii = { sidebar_width = 99 }, ui = { sidebar_width = 28 } }"#,
    )
    .unwrap();
    assert_eq!(config.sidebar_width(), 28);
}

#[test]
fn broken_init_lua_is_a_hard_error() {
    assert!(load_source("syntax", "return {").is_err());
    assert!(load_source("not-a-table", "return 42").is_err());
    let err = load_source("runaway", "while true do end").unwrap_err();
    assert!(
        format!("{err:#}").contains("instruction budget exceeded"),
        "{err:#}"
    );
}

#[test]
fn cascade_prefers_init_lua_over_config_toml() {
    let dir = temp_dir("cascade");
    std::fs::write(dir.join("config.toml"), "[ui]\nsidebar_width = 50\n").unwrap();
    std::fs::write(
        dir.join("init.lua"),
        "return { ui = { sidebar_width = 28 } }",
    )
    .unwrap();
    let config = ekko_lua::load_config_cascade_in(&dir).unwrap();
    assert_eq!(config.sidebar_width(), 28);

    // Without init.lua the TOML applies; with neither, defaults.
    std::fs::remove_file(dir.join("init.lua")).unwrap();
    let config = ekko_lua::load_config_cascade_in(&dir).unwrap();
    assert_eq!(config.sidebar_width(), 50);
    std::fs::remove_file(dir.join("config.toml")).unwrap();
    let config = ekko_lua::load_config_cascade_in(&dir).unwrap();
    assert_eq!(config.sidebar_width(), ekko_config::SIDEBAR_WIDTH_DEFAULT);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn broken_init_lua_does_not_fall_through_to_toml() {
    // Silently ignoring the user's config is worse than refusing to start:
    // a broken init.lua must error even though a valid config.toml coexists.
    let dir = temp_dir("no-fallthrough");
    std::fs::write(dir.join("config.toml"), "[ui]\nsidebar_width = 50\n").unwrap();
    std::fs::write(dir.join("init.lua"), "return {").unwrap();
    assert!(ekko_lua::load_config_cascade_in(&dir).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The C-acceptance end-to-end: with no TOML anywhere, `init.lua` disables
/// the scroll-mode builtin and `examples/scroll-mode.lua` re-registers the
/// "scroll" mode under the script's own manifest. Duplicate names are hard
/// build errors, so the build succeeding with the full builtin set proves
/// the disable took effect and the surviving mode is the script's.
#[test]
fn init_lua_disables_a_builtin_and_a_script_replaces_it() {
    let dir = temp_dir("replace");
    std::fs::write(
        dir.join("init.lua"),
        r#"return { extensions = { disabled = { "ekko-builtins.scroll-mode" } } }"#,
    )
    .unwrap();
    let ext_dir = dir.join("extensions");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/scroll-mode.lua"
        ),
        ext_dir.join("scroll-mode.lua"),
    )
    .unwrap();

    let config = ekko_lua::load_config_cascade_in(&dir).unwrap();
    let runtime = RuntimeBuilder::new()
        .with_disabled(&config.extensions.disabled)
        .register_boxed_extensions(ekko_builtins::client_extensions(&config, None))
        .register_boxed_extensions(ekko_lua::load_extensions(
            &ext_dir,
            ekko_lua::HostKind::Client,
            &config,
        ))
        .build()
        .unwrap();
    assert!(runtime.mode("scroll").is_some(), "script's mode registered");
    assert!(
        runtime
            .manifests()
            .iter()
            .any(|m| m.id == "user.scroll-mode"),
        "the replacement extension is live"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The P4-acceptance end-to-end: `init.lua` disables the panes builtin and
/// `examples/pane-keys.lua` re-registers the whole pane surface — same
/// command names, same leader keys — under the script's own manifest. The
/// runtime builds (duplicate names are hard errors) and the script's
/// commands/keybindings dispatch the public pane actions.
#[test]
fn init_lua_disables_the_panes_builtin_and_a_script_replaces_it() {
    let dir = temp_dir("replace-panes");
    std::fs::write(
        dir.join("init.lua"),
        r#"return { extensions = { disabled = { "ekko-builtins.panes" } } }"#,
    )
    .unwrap();
    let ext_dir = dir.join("extensions");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pane-keys.lua"),
        ext_dir.join("pane-keys.lua"),
    )
    .unwrap();

    let config = ekko_lua::load_config_cascade_in(&dir).unwrap();
    let runtime = RuntimeBuilder::new()
        .with_disabled(&config.extensions.disabled)
        .register_boxed_extensions(ekko_builtins::client_extensions(&config, None))
        .register_boxed_extensions(ekko_lua::load_extensions(
            &ext_dir,
            ekko_lua::HostKind::Client,
            &config,
        ))
        .build()
        .unwrap();

    assert!(runtime.manifests().iter().any(|m| m.id == "user.pane-keys"));
    assert_eq!(
        runtime.invoke_command(":split right"),
        CommandDispatch::Invoked(vec![UiAction::SplitRight])
    );
    assert_eq!(
        runtime.invoke_command(":split down"),
        CommandDispatch::Invoked(vec![UiAction::SplitDown])
    );
    assert_eq!(
        runtime.invoke_command(":pane-focus left"),
        CommandDispatch::Invoked(vec![UiAction::FocusPaneDirection {
            direction: ekko_ext::PaneDirection::Left,
        }])
    );
    assert_eq!(
        runtime.invoke_command(":pane-close"),
        CommandDispatch::Invoked(vec![UiAction::CloseFocusedPane])
    );
    let spec = runtime
        .match_keybinding(b"|", Some("leader"))
        .expect("script's leader key registered");
    assert_eq!(
        (spec.handler)(&snapshot_with_panes()),
        vec![UiAction::ExitMode, UiAction::SplitRight]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

fn snapshot_with_panes() -> ekko_ext::ClientSnapshot {
    ekko_ext::ClientSnapshot {
        session_name: "s".into(),
        mode: ekko_ext::ClientSnapshot::NORMAL_MODE.into(),
        cols: 80,
        rows: 24,
        grid_cols: 80,
        grid_rows: 24,
        scrollback: 0,
        panes: vec![],
        focused_pane: None,
        projects: vec![],
        status_note: None,
        keybindings: vec![],
        now_ms: 0,
        hidden_surfaces: Vec::new(),
        theme: ekko_ext::ThemePalette::fallback(),
    }
}

#[test]
fn scripts_read_the_resolved_config_as_ekko_config() {
    let script = r#"
        local ext = { id = "user.cfg" }
        function ext.register(ekko)
          local text = ekko.config.general.default_shell
            .. ":" .. ekko.config.ui.sidebar_width
            .. ":" .. ekko.config.keybinds.detach
          ekko.register_command({
            name = "cfg",
            handler = function(args)
              return { { set_status_note = { text = text, kind = "ok", ttl_ms = 1000 } } }
            end,
          })
        end
        return ext
    "#;
    let mut ext = LuaExtension::from_source("cfg.lua", script).unwrap();
    let mut config = ekko_config::Config::default();
    config.general.default_shell = "/bin/zsh".into();
    config.ui.sidebar_width = 28;
    config.keybinds.insert(
        "detach".into(),
        ekko_config::Keybind::Single("ctrl+q".into()),
    );
    ext.set_config(&config);
    let runtime = RuntimeBuilder::new()
        .register_boxed_extension(Box::new(ext))
        .build()
        .unwrap();
    assert_eq!(
        runtime.invoke_command(":cfg"),
        CommandDispatch::Invoked(vec![UiAction::SetStatusNote {
            text: "/bin/zsh:28:ctrl+q".into(),
            kind: NoteKind::Ok,
            ttl_ms: 1000,
        }])
    );
}

#[test]
fn ekko_config_defaults_when_the_host_provides_none() {
    // from_source without set_config (unit tests, ad-hoc embedding): the
    // table is still present, carrying Config::default().
    let script = r#"
        local ext = { id = "user.cfgdefault" }
        function ext.register(ekko)
          local text = "w=" .. ekko.config.ui.sidebar_width
          ekko.register_command({
            name = "cfgd",
            handler = function(args)
              return { { set_status_note = { text = text, kind = "ok", ttl_ms = 1000 } } }
            end,
          })
        end
        return ext
    "#;
    let runtime = RuntimeBuilder::new()
        .register_boxed_extension(Box::new(
            LuaExtension::from_source("cfgdefault.lua", script).unwrap(),
        ))
        .build()
        .unwrap();
    assert_eq!(
        runtime.invoke_command(":cfgd"),
        CommandDispatch::Invoked(vec![UiAction::SetStatusNote {
            text: format!("w={}", ekko_config::SIDEBAR_WIDTH_DEFAULT),
            kind: NoteKind::Ok,
            ttl_ms: 1000,
        }])
    );
}
