//! ekko-lua: the Lua scripting front-end of the ekko extension system (WS9).
//!
//! A script is an extension. It evaluates to a table carrying the manifest
//! fields plus a `register(ekko)` function, and the bridge terminates in the
//! same [`Extension`]/[`ExtensionHost`] traits native extensions use — the
//! host cannot tell a scripted extension from a compiled one:
//!
//! ```lua
//! local ext = {
//!   id = "user.hello",
//!   name = "hello",
//!   version = "0.1.0",
//!   description = "example extension",
//! }
//!
//! function ext.register(ekko)
//!   ekko.register_command({
//!     name = "hello",
//!     description = "say hello",
//!     handler = function(args)
//!       return { { set_status_note = { text = "hello " .. args, kind = "ok" } } }
//!     end,
//!   })
//! end
//!
//! return ext
//! ```
//!
//! Guard rails (the "scripting bridge adds its own guard" from `DESIGN.md`):
//! every callback into Lua runs under an instruction budget (a runaway
//! script errors out instead of wedging the host), and draw callbacks write
//! buffered, data-only draw ops that are replayed into the real
//! [`ekko_ext::DrawContext`] after the Lua call returns — native extensions
//! pay for none of this.
//!
//! Registration surface (v1): commands, keybindings, surfaces (with
//! `visible`/`wants_tick`/`on_mouse`), overlays (with `build_payload`),
//! themes, the session namer, and event subscriptions. Modes, spinners,
//! and the session grouper are not yet bridged.

mod convert;
mod draw;

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use ekko_ext::{
    CommandOutput, CommandSpec, EventHandlerRegistration, Extension, ExtensionHost,
    ExtensionManifest, KeybindingSpec, OverlayOutcome, OverlaySpec, OverlayState, SessionNamerSpec,
    SurfaceSpec, ThemeSpec, parse_key_chords,
};
use mlua::{Function, Lua, RegistryKey, Table, Value};

use convert::{
    actions_from_value, event_kind_from_name, event_return_from_value, mouse_event_table,
    palette_from_table, payload_table, registry_view_table, snapshot_table,
};
use draw::{DrawOp, ops_context_table, replay};

/// Instruction budget for render-path callbacks (draw / visible /
/// wants_tick), which run on the client's frame pass.
const DRAW_BUDGET: u32 = 200_000;
/// Instruction budget for handler callbacks (commands, keybindings, events,
/// overlay keys), which the host already bounds by wall-clock timeouts.
const HANDLER_BUDGET: u32 = 2_000_000;

/// A shared handle on one script's Lua state. All callbacks from all
/// registries of one script serialize on this lock; separate scripts get
/// separate states.
type SharedLua = Arc<Mutex<Lua>>;

/// Run `f` on the Lua state with an instruction budget: the script errors
/// with "instruction budget exceeded" instead of looping forever.
fn with_budget<R>(lua: &Lua, budget: u32, f: impl FnOnce(&Lua) -> mlua::Result<R>) -> Result<R> {
    lua.set_hook(
        mlua::HookTriggers::new().every_nth_instruction(budget),
        |_lua, _debug| {
            Err(mlua::Error::RuntimeError(
                "instruction budget exceeded".into(),
            ))
        },
    );
    let result = f(lua);
    lua.remove_hook();
    result.map_err(|e| anyhow!("{e}"))
}

/// One `.lua` script, loaded and evaluated, ready to register.
pub struct LuaExtension {
    manifest: ExtensionManifest,
    lua: SharedLua,
    register_fn: RegistryKey,
}

impl LuaExtension {
    /// Evaluate `source` (named `origin` in error messages) into an
    /// extension. Fails if the script errors or does not return a table
    /// with an `id` and a `register` function.
    pub fn from_source(origin: &str, source: &str) -> Result<Self> {
        let lua = Lua::new();
        let ext: Table = with_budget(&lua, HANDLER_BUDGET, |lua| {
            lua.load(source).set_name(origin).eval()
        })
        .with_context(|| format!("evaluating lua extension '{origin}'"))?;

        let field = |name: &str, default: &str| -> String {
            ext.get::<Option<String>>(name)
                .ok()
                .flatten()
                .unwrap_or_else(|| default.to_string())
        };
        let id = ext
            .get::<Option<String>>("id")
            .ok()
            .flatten()
            .ok_or_else(|| anyhow!("lua extension '{origin}' is missing an 'id' field"))?;
        let manifest = ExtensionManifest {
            name: field("name", &id),
            version: field("version", "0.0.0"),
            description: field("description", ""),
            id,
        };
        let register: Function = ext
            .get::<Option<Function>>("register")
            .ok()
            .flatten()
            .ok_or_else(|| anyhow!("lua extension '{origin}' is missing a 'register' function"))?;
        let register_fn = lua.create_registry_value(register)?;
        Ok(Self {
            manifest,
            lua: Arc::new(Mutex::new(lua)),
            register_fn,
        })
    }

    /// Load one `.lua` file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_source(&path.display().to_string(), &source)
    }
}

/// Load every `*.lua` file in `dir` (sorted by name), skipping — with a
/// logged warning — scripts that fail to evaluate, so one broken user
/// script degrades to a warning instead of an unusable terminal.
/// Registration conflicts (duplicate names) still fail the runtime build
/// loudly, exactly as they do for native extensions.
pub fn load_extensions(dir: &Path) -> Vec<Box<dyn Extension>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "lua"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| match LuaExtension::from_file(&path) {
            Ok(ext) => Some(Box::new(ext) as Box<dyn Extension>),
            Err(err) => {
                log::warn!("skipping lua extension {}: {err:#}", path.display());
                None
            }
        })
        .collect()
}

impl Extension for LuaExtension {
    fn manifest(&self) -> ExtensionManifest {
        self.manifest.clone()
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        let lua = self.lua.lock().unwrap();
        // The `ekko` table the script's register() sees is a plain collector:
        // each register_* call appends its spec table. The bridge then walks
        // the collected specs and registers real host entries whose closures
        // call back into the stored Lua functions.
        let collector: Table = with_budget(&lua, HANDLER_BUDGET, |lua| {
            let regs: Table = lua
                .load(
                    r#"
                    local regs = {
                      commands = {}, keybindings = {}, surfaces = {},
                      overlays = {}, themes = {}, namers = {}, subscriptions = {},
                    }
                    regs.ekko = {
                      register_command = function(spec) table.insert(regs.commands, spec) end,
                      register_keybinding = function(spec) table.insert(regs.keybindings, spec) end,
                      register_surface = function(spec) table.insert(regs.surfaces, spec) end,
                      register_overlay = function(spec) table.insert(regs.overlays, spec) end,
                      register_theme = function(spec) table.insert(regs.themes, spec) end,
                      register_session_namer = function(spec) table.insert(regs.namers, spec) end,
                      subscribe = function(event, handler)
                        table.insert(regs.subscriptions, { event = event, handler = handler })
                      end,
                    }
                    return regs
                    "#,
                )
                .set_name("ekko-lua collector")
                .eval()?;
            let register: Function = lua.registry_value(&self.register_fn)?;
            register.call::<()>(regs.get::<Table>("ekko")?)?;
            Ok(regs)
        })
        .with_context(|| format!("running register() of '{}'", self.manifest.id))?;

        for spec in collector.get::<Table>("commands")?.sequence_values() {
            self.register_command(host, &lua, spec?)?;
        }
        for spec in collector.get::<Table>("keybindings")?.sequence_values() {
            self.register_keybinding(host, &lua, spec?)?;
        }
        for spec in collector.get::<Table>("surfaces")?.sequence_values() {
            self.register_surface(host, &lua, spec?)?;
        }
        for spec in collector.get::<Table>("overlays")?.sequence_values() {
            self.register_overlay(host, &lua, spec?)?;
        }
        for spec in collector.get::<Table>("themes")?.sequence_values() {
            self.register_theme(host, spec?)?;
        }
        for spec in collector.get::<Table>("namers")?.sequence_values() {
            self.register_session_namer(host, &lua, spec?)?;
        }
        for spec in collector.get::<Table>("subscriptions")?.sequence_values() {
            self.register_subscription(host, &lua, spec?)?;
        }
        Ok(())
    }
}

impl LuaExtension {
    /// Stash a Lua function in the registry and hand back a callable that
    /// locks the state, applies `budget`, and invokes it.
    fn stash(&self, lua: &Lua, function: Function) -> Result<Arc<RegistryKey>> {
        Ok(Arc::new(lua.create_registry_value(function)?))
    }

    fn register_command(&self, host: &mut dyn ExtensionHost, lua: &Lua, spec: Table) -> Result<()> {
        let name: String = spec
            .get::<Option<String>>("name")?
            .ok_or_else(|| anyhow!("command spec needs a 'name'"))?;
        let handler = self.stash(
            lua,
            spec.get::<Option<Function>>("handler")?
                .ok_or_else(|| anyhow!("command '{name}' needs a 'handler'"))?,
        )?;
        let aliases: Vec<String> = match spec.get::<Option<Table>>("aliases")? {
            Some(t) => t.sequence_values::<String>().collect::<mlua::Result<_>>()?,
            None => Vec::new(),
        };
        let shared = self.lua.clone();
        host.register_command(CommandSpec {
            name: name.clone(),
            aliases,
            description: spec
                .get::<Option<String>>("description")?
                .unwrap_or_default(),
            args_hint: spec.get::<Option<String>>("args_hint")?.unwrap_or_default(),
            handler: Arc::new(move |invocation| {
                let lua = shared.lock().unwrap();
                let actions = with_budget(&lua, HANDLER_BUDGET, |lua| {
                    let f: Function = lua.registry_value(&handler)?;
                    f.call::<Value>(invocation.raw_args.clone())
                })?;
                Ok(CommandOutput::actions(actions_from_value(&actions)?))
            }),
        })
    }

    fn register_keybinding(
        &self,
        host: &mut dyn ExtensionHost,
        lua: &Lua,
        spec: Table,
    ) -> Result<()> {
        let mut chord_texts: Vec<String> = Vec::new();
        if let Some(chord) = spec.get::<Option<String>>("chord")? {
            chord_texts.push(chord);
        }
        if let Some(chords) = spec.get::<Option<Table>>("chords")? {
            for chord in chords.sequence_values::<String>() {
                chord_texts.push(chord?);
            }
        }
        if chord_texts.is_empty() {
            return Err(anyhow!("keybinding spec needs a 'chord' (or 'chords')"));
        }
        let chords = chord_texts
            .iter()
            .map(|text| parse_key_chords(text).ok_or_else(|| anyhow!("unparseable chord '{text}'")))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();
        let handler = self.stash(
            lua,
            spec.get::<Option<Function>>("handler")?
                .ok_or_else(|| anyhow!("keybinding '{}' needs a 'handler'", chord_texts[0]))?,
        )?;
        let shared = self.lua.clone();
        host.register_keybinding(KeybindingSpec {
            chords,
            chord_text: chord_texts.join(" / "),
            mode: spec.get::<Option<String>>("mode")?,
            description: spec
                .get::<Option<String>>("description")?
                .unwrap_or_default(),
            handler: Arc::new(move |snapshot| {
                let lua = shared.lock().unwrap();
                let result = with_budget(&lua, HANDLER_BUDGET, |lua| {
                    let f: Function = lua.registry_value(&handler)?;
                    f.call::<Value>(snapshot_table(lua, snapshot)?)
                })
                .and_then(|value| actions_from_value(&value));
                match result {
                    Ok(actions) => actions,
                    Err(err) => {
                        log::warn!("lua keybinding handler errored: {err:#}");
                        Vec::new()
                    }
                }
            }),
        })
    }

    fn register_surface(&self, host: &mut dyn ExtensionHost, lua: &Lua, spec: Table) -> Result<()> {
        let name: String = spec
            .get::<Option<String>>("name")?
            .ok_or_else(|| anyhow!("surface spec needs a 'name'"))?;
        let draw = self.stash(
            lua,
            spec.get::<Option<Function>>("draw")?
                .ok_or_else(|| anyhow!("surface '{name}' needs a 'draw' function"))?,
        )?;
        let dock = match spec
            .get::<Option<String>>("dock")?
            .unwrap_or_else(|| "bottom".into())
            .as_str()
        {
            "left" => ekko_ext::DockEdge::Left,
            "right" => ekko_ext::DockEdge::Right,
            "top" => ekko_ext::DockEdge::Top,
            "bottom" => ekko_ext::DockEdge::Bottom,
            other => return Err(anyhow!("surface '{name}': unknown dock edge '{other}'")),
        };

        let shared = self.lua.clone();
        let draw_name = name.clone();
        let draw_fn: ekko_ext::SurfaceDrawFn = Arc::new(move |ctx, snapshot| {
            let lua = shared.lock().unwrap();
            let ops: Arc<Mutex<Vec<DrawOp>>> = Arc::default();
            let called = with_budget(&lua, DRAW_BUDGET, |lua| {
                let f: Function = lua.registry_value(&draw)?;
                let ctx_table = ops_context_table(lua, ops.clone(), ctx.size(), snapshot.theme)?;
                f.call::<()>((ctx_table, snapshot_table(lua, snapshot)?))
            });
            match called {
                Ok(()) => replay(&ops.lock().unwrap(), ctx),
                Err(err) => log::warn!("lua surface '{draw_name}' draw errored: {err:#}"),
            }
        });

        let visible = self.optional_bool_predicate(lua, &spec, "visible")?;
        let wants_tick = self.optional_bool_predicate(lua, &spec, "wants_tick")?;

        let on_mouse: Option<ekko_ext::SurfaceMouseFn> =
            match spec.get::<Option<Function>>("on_mouse")? {
                None => None,
                Some(f) => {
                    let key = self.stash(lua, f)?;
                    let shared = self.lua.clone();
                    Some(Arc::new(move |event, snapshot| {
                        let lua = shared.lock().unwrap();
                        let result = with_budget(&lua, HANDLER_BUDGET, |lua| {
                            let f: Function = lua.registry_value(&key)?;
                            f.call::<Value>((
                                mouse_event_table(lua, event)?,
                                snapshot_table(lua, snapshot)?,
                            ))
                        })
                        .and_then(|value| actions_from_value(&value));
                        match result {
                            Ok(actions) => actions,
                            Err(err) => {
                                log::warn!("lua surface mouse handler errored: {err:#}");
                                Vec::new()
                            }
                        }
                    }))
                }
            };

        host.register_surface(SurfaceSpec {
            name,
            dock,
            priority: spec.get::<Option<i32>>("priority")?.unwrap_or(100),
            size: ekko_ext::SurfaceSize::Fixed(spec.get::<Option<i32>>("size")?.unwrap_or(1)),
            hide_below: None,
            visible,
            draw: draw_fn,
            on_mouse,
            wants_tick,
        })
    }

    /// A `visible`/`wants_tick`-style Lua predicate: `f(snapshot) -> bool`.
    /// Errors count as `false` so a broken script hides rather than wedges.
    fn optional_bool_predicate(
        &self,
        lua: &Lua,
        spec: &Table,
        field: &'static str,
    ) -> Result<Option<ekko_ext::SurfaceVisibleFn>> {
        let Some(f) = spec.get::<Option<Function>>(field)? else {
            return Ok(None);
        };
        let key = self.stash(lua, f)?;
        let shared = self.lua.clone();
        Ok(Some(Arc::new(move |snapshot| {
            let lua = shared.lock().unwrap();
            with_budget(&lua, DRAW_BUDGET, |lua| {
                let f: Function = lua.registry_value(&key)?;
                f.call::<bool>(snapshot_table(lua, snapshot)?)
            })
            .inspect_err(|err| log::warn!("lua '{field}' predicate errored: {err:#}"))
            .unwrap_or(false)
        })))
    }

    fn register_overlay(&self, host: &mut dyn ExtensionHost, lua: &Lua, spec: Table) -> Result<()> {
        let name: String = spec
            .get::<Option<String>>("name")?
            .ok_or_else(|| anyhow!("overlay spec needs a 'name'"))?;
        let render = self.stash(
            lua,
            spec.get::<Option<Function>>("render")?
                .ok_or_else(|| anyhow!("overlay '{name}' needs a 'render' function"))?,
        )?;
        let init = match spec.get::<Option<Function>>("init")? {
            Some(f) => Some(self.stash(lua, f)?),
            None => None,
        };
        let handle_key = match spec.get::<Option<Function>>("handle_key")? {
            Some(f) => Some(self.stash(lua, f)?),
            None => None,
        };
        let build_payload = match spec.get::<Option<Function>>("build_payload")? {
            Some(f) => Some(self.stash(lua, f)?),
            None => None,
        };

        // Overlay state is a Lua value held in the registry for as long as
        // the overlay is open; the host stores only the opaque key.
        struct LuaOverlayState(RegistryKey);

        let init_shared = self.lua.clone();
        let init_key = init.clone();
        let init_fn: ekko_ext::OverlayInitFn = Arc::new(move |payload| {
            let lua = init_shared.lock().unwrap();
            let payload_value: Value = payload
                .and_then(|p| p.downcast::<RegistryKey>().ok())
                .and_then(|key| lua.registry_value::<Value>(&key).ok())
                .unwrap_or(Value::Nil);
            let state_value = match &init_key {
                None => payload_value,
                Some(init) => with_budget(&lua, HANDLER_BUDGET, |lua| {
                    let f: Function = lua.registry_value(init)?;
                    f.call::<Value>(payload_value.clone())
                })
                .unwrap_or_else(|err| {
                    log::warn!("lua overlay init errored: {err:#}");
                    Value::Nil
                }),
            };
            let key = lua
                .create_registry_value(state_value)
                .expect("storing overlay state");
            Box::new(LuaOverlayState(key)) as OverlayState
        });

        let render_shared = self.lua.clone();
        let render_name = name.clone();
        let render_fn: ekko_ext::OverlayRenderFn = Arc::new(move |ctx, state, snapshot| {
            let Some(state) = state.downcast_ref::<LuaOverlayState>() else {
                return;
            };
            let lua = render_shared.lock().unwrap();
            let ops: Arc<Mutex<Vec<DrawOp>>> = Arc::default();
            let called = with_budget(&lua, DRAW_BUDGET, |lua| {
                let f: Function = lua.registry_value(&render)?;
                let ctx_table = ops_context_table(lua, ops.clone(), ctx.size(), snapshot.theme)?;
                let state_value: Value = lua.registry_value(&state.0)?;
                f.call::<()>((ctx_table, state_value, snapshot_table(lua, snapshot)?))
            });
            match called {
                Ok(()) => replay(&ops.lock().unwrap(), ctx),
                Err(err) => log::warn!("lua overlay '{render_name}' render errored: {err:#}"),
            }
        });

        let key_shared = self.lua.clone();
        let key_fn: ekko_ext::OverlayKeyFn = Arc::new(move |state, bytes| {
            let Some(handle_key) = &handle_key else {
                // No key handler: any key closes, mirroring a plain panel.
                return OverlayOutcome::Close;
            };
            let Some(state) = state.downcast_ref::<LuaOverlayState>() else {
                return OverlayOutcome::Close;
            };
            let lua = key_shared.lock().unwrap();
            let outcome = with_budget(&lua, HANDLER_BUDGET, |lua| {
                let f: Function = lua.registry_value(handle_key)?;
                let state_value: Value = lua.registry_value(&state.0)?;
                f.call::<Option<String>>((state_value, lua.create_string(bytes)?))
            });
            match outcome {
                Ok(Some(word)) if word == "close" => OverlayOutcome::Close,
                Ok(_) => OverlayOutcome::None,
                Err(err) => {
                    log::warn!("lua overlay key handler errored: {err:#}");
                    OverlayOutcome::Close
                }
            }
        });

        let payload_fn: Option<ekko_ext::OverlayPayloadFn> = build_payload.map(|key| {
            let shared = self.lua.clone();
            Arc::new(move |registries: &ekko_ext::RegistryView| {
                let lua = shared.lock().unwrap();
                let value = with_budget(&lua, HANDLER_BUDGET, |lua| {
                    let f: Function = lua.registry_value(&key)?;
                    f.call::<Value>(registry_view_table(lua, registries)?)
                })
                .unwrap_or_else(|err| {
                    log::warn!("lua overlay build_payload errored: {err:#}");
                    Value::Nil
                });
                let key = lua
                    .create_registry_value(value)
                    .expect("storing overlay payload");
                Box::new(key) as ekko_ext::OverlayPayload
            }) as ekko_ext::OverlayPayloadFn
        });

        host.register_overlay(OverlaySpec {
            name,
            description: spec
                .get::<Option<String>>("description")?
                .unwrap_or_default(),
            init_state: init_fn,
            render: render_fn,
            handle_key: key_fn,
            build_payload: payload_fn,
        })
    }

    fn register_theme(&self, host: &mut dyn ExtensionHost, spec: Table) -> Result<()> {
        let name: String = spec
            .get::<Option<String>>("name")?
            .ok_or_else(|| anyhow!("theme spec needs a 'name'"))?;
        let palette = palette_from_table(spec.get::<Option<Table>>("palette")?)?;
        host.register_theme(ThemeSpec {
            name,
            description: spec
                .get::<Option<String>>("description")?
                .unwrap_or_default(),
            palette,
        })
    }

    fn register_session_namer(
        &self,
        host: &mut dyn ExtensionHost,
        lua: &Lua,
        spec: Table,
    ) -> Result<()> {
        let name: String = spec
            .get::<Option<String>>("name")?
            .ok_or_else(|| anyhow!("session namer spec needs a 'name'"))?;
        let generate = self.stash(
            lua,
            spec.get::<Option<Function>>("generate")?
                .ok_or_else(|| anyhow!("session namer '{name}' needs a 'generate' function"))?,
        )?;
        let shared = self.lua.clone();
        host.register_session_namer(SessionNamerSpec {
            name,
            generate: Arc::new(move |input| {
                let lua = shared.lock().unwrap();
                with_budget(&lua, HANDLER_BUDGET, |lua| {
                    let f: Function = lua.registry_value(&generate)?;
                    let t = lua.create_table()?;
                    t.set("cwd", input.cwd.display().to_string())?;
                    t.set(
                        "taken",
                        lua.create_sequence_from(input.taken.iter().cloned())?,
                    )?;
                    f.call::<String>(t)
                })
                .unwrap_or_else(|err| {
                    // Empty fails the host's sanitizer, which falls back to
                    // its own generator.
                    log::warn!("lua session namer errored: {err:#}");
                    String::new()
                })
            }),
        })
    }

    fn register_subscription(
        &self,
        host: &mut dyn ExtensionHost,
        lua: &Lua,
        spec: Table,
    ) -> Result<()> {
        let event_name: String = spec
            .get::<Option<String>>("event")?
            .ok_or_else(|| anyhow!("subscribe() needs an event name"))?;
        let kind = event_kind_from_name(&event_name)
            .ok_or_else(|| anyhow!("unknown event '{event_name}'"))?;
        let handler = self.stash(
            lua,
            spec.get::<Option<Function>>("handler")?
                .ok_or_else(|| anyhow!("subscribe('{event_name}') needs a handler"))?,
        )?;
        let shared = self.lua.clone();
        host.subscribe(EventHandlerRegistration {
            event: kind,
            label: format!("{}:{event_name}", self.manifest.id),
            handler: Arc::new(move |event| {
                let lua = shared.lock().unwrap();
                let value = with_budget(&lua, HANDLER_BUDGET, |lua| {
                    let f: Function = lua.registry_value(&handler)?;
                    f.call::<Value>(payload_table(lua, &event.payload)?)
                })?;
                event_return_from_value(&value)
            }),
        })
    }
}
