//! WASM bridge (`wasm` feature): load config and extensions as compiled
//! `.wasm` modules through the shared `cordis-rs` kernel instead of the
//! deleted `ekko-lua` (mlua) bridge.
//!
//! Two entry points mirror the former `ekko_lua` surface:
//! - [`load_config_cascade`] — the WASM config cascade: a user `config.wasm`
//!   in the config dir supersedes a compiled *default* config module loaded
//!   at startup. A config module is an extension that only writes ([`abi`]
//!   function set 1): its `mount` `ctx_set`s a `config` key holding the
//!   [`Config`] JSON, and the host rebuilds [`Config`] from it
//!   (`#[serde(default)]` yields defaults for any unspecified field).
//! - [`load_extensions`] — load every `*.wasm` extension in `dir`. Each is
//!   mounted on its own [`cordis`][`cordis::Context`] (effects revert on
//!   unmount, `[[principle:spatiotemporal]]`), its declared units (set 2:
//!   `(name, kind)`) are read back, and it is handed to the runtime as an
//!   [`Extension`].
//!
//! # Dynamic host->guest dispatch (cordis set 6)
//!
//! A `.wasm` extension that needs live callbacks — a command handler, a mode
//! key/render hook, an event subscriber, a status-bar/which-key renderer —
//! drives them over [`cordis::Context::call`]: the host surfaces one guest
//! **export per unit kind** and calls it synchronously when the runtime
//! dispatches the matching event. Each export receives an immutable JSON
//! payload snapshot and returns a JSON action string the host decodes into
//! the same return type a native handler produces — functional-core at the
//! WASM boundary (the guest never holds `&mut` host state; `UiAction`s are
//! applied by the host's single write path).
//!
//! The dispatch table (a registered unit of `kind` maps to the guest export):
//!
//! | kind | guest export | payload (`DispatchRequest`, `kind` field) | return |
//! |---|---|---|---|
//! | `command` | `on_command` | `command` + `args` | [`CommandOutput`] |
//! | `keybinding` | `on_key` | `key` + `bytes` + `snapshot` | `Vec<UiAction>` |
//! | `mode` | `on_mode_key` | `mode_key` + `bytes` + `snapshot` | [`ModeOutcome`] |
//! | `mode` | `on_mode_render` | `mode_render` + `snapshot` | cursor JSON |
//! | `subscription` | `on_event` | `event` + `event` payload | `Option<EventReturn>` |
//!
//! A build without `wasm` links no wasmtime and reaches only the native
//! extension surface (bare harness).

use std::path::Path;
use std::sync::{Arc, Mutex};

use cordis::Context as CordisContext;
use ekko_config::{Config, ConfigWasmEvaluator};
use ekko_err::{Context as _, Result};
use ekko_event::{EventKind, EventPayload, LifecycleEvent};
use serde::{Deserialize, Serialize};

use crate::keybinding::{parse_key_binding, parse_key_chords};
use crate::snapshot::ClientSnapshot;
use crate::{
    Color, CommandSpec, DockEdge, DrawContext, Extension, ExtensionHost, ExtensionManifest,
    KeybindingSpec, ModeOutcome, ModeSpec, ModeState, Rect, ScrollbarModel, ScrollbarStyle,
    SurfaceDrawFn, SurfaceSize, SurfaceSpec, SurfaceTickFn, TextStyle,
};
// `CommandOutput`, `EventReturn` and `UiAction` appear only in the dispatch
// table's intra-doc links / inferred return types; keep them imported (with
// the allow) for the documented contract without a literal use.
#[allow(unused_imports)]
use ekko_event::{EventReturn, UiAction};

/// Which process loads `.wasm` extensions. Kept for parity with the deleted
/// `ekko_lua::HostKind` and to keep the loaders symmetric; the kernel's
/// current set-2 ABI has no host declaration field, so loaders accept it for
/// call-site documentation and logging only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKind {
    Client,
    Server,
}

/// The compiled *default* config module, loaded at startup when no user
/// `config.wasm` exists. It writes `config = "{}"` (an empty override), so
/// parsing with the schema's `#[serde(default)]` yields [`Config::default`]:
/// config really is data on the cordis ABI, with no privileged text parser.
const DEFAULT_CONFIG_WAT: &str = include_str!("default_config.wat");

/// The config evaluator that runs a compiled config `.wasm` on the cordis
/// kernel (set 1) and rebuilds [`Config`] from the `config` key it writes.
struct WasmConfigEvaluator;

impl ConfigWasmEvaluator for WasmConfigEvaluator {
    fn eval_config_wasm(&self, wasm: &[u8]) -> Result<Config> {
        let mut ctx = CordisContext::new();
        let id = ctx
            .mount(wasm)
            .map_err(|e| ekko_err::err!("mounting config.wasm: {e}"))?;
        let mut config: Config = match ctx.get("config") {
            Some(json) => serde_json::from_str(&json)
                .map_err(|e| ekko_err::err!("config.wasm wrote invalid config JSON: {e}"))?,
            None => Config::default(),
        };
        config.normalize();
        let _ = ctx
            .unmount(id)
            .map_err(|e| ekko_err::err!("unmounting config: {e}"));
        Ok(config)
    }
}

/// The WASM config cascade both processes (client and daemon) call at
/// startup: a user `config.wasm` supersedes the compiled default; a stale
/// `config.toml` is a hard migration error. Mirrors the shape of the deleted
/// `ekko_lua::load_config_cascade`.
pub fn load_config_cascade() -> Result<Config> {
    let dir = ekko_config::config_dir();
    let evaluator = WasmConfigEvaluator;
    let user = dir.join("config.wasm");
    if user.is_file() {
        return Config::load_cascade_in(&dir, Some(&evaluator));
    }
    if dir.join("config.toml").exists() {
        ekko_err::bail!(
            "unsupported config file {}; migrate to config.wasm",
            dir.join("config.toml").display()
        );
    }
    // Default config ships as a compiled `.wasm` loaded at startup (the
    // reference config-wasm pattern: embed `.wat`, parse once, mount before
    // any extension). `{}` -> `#[serde(default)]` -> `Config::default()`.
    let wasm = wat::parse_str(DEFAULT_CONFIG_WAT)
        .map_err(|e| ekko_err::err!("parsing default config.wasm: {e}"))?;
    evaluator.eval_config_wasm(&wasm)
}

/// A `.wasm` module loaded as an ekko extension. Mounting runs its effects
/// and collects its matrix-2 registration units + buffered ops on the
/// [`cordis::Context`]; the mounted instance is retained so the declared
/// units stay owned by the kernel for the runtime's lifetime.
pub struct WasmExtension {
    manifest: ExtensionManifest,
    /// The live kernel for this module; declared effects/registrations are
    /// owned here (revertible on drop) rather than copied into the host.
    ctx: Arc<Mutex<CordisContext>>,
    /// This module's plugin id inside `ctx`, handed to every
    /// [`cordis::Context::call`] so a dynamic dispatch attributes to this
    /// extension (its `ctx_*` host funcs and buffered ops).
    plugin: usize,
    /// The set-2 `(name, kind, descriptors)` units the module declared, in
    /// order. `descriptors` carries the validated args after the name (a
    /// keybinding's mode, a surface's dock/priority/size, ...).
    declared: Vec<(String, String, Vec<String>)>,
}

impl WasmExtension {
    /// Load and mount the `.wasm` module at `path`, deriving its manifest id
    /// from the file name (striking a stable, unique extension id).
    pub fn from_file(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "extension".to_string());
        Self::from_bytes(&bytes, &stem)
    }

    /// Compile and mount `wasm`, collecting its declared registration units.
    pub fn from_bytes(wasm: &[u8], id: &str) -> Result<Self> {
        let mut ctx = CordisContext::new();
        let pid = ctx
            .mount(wasm)
            .map_err(|e| ekko_err::err!("mounting wasm extension '{id}': {e}"))?;
        // Registration is a kernel-owned effect: read the units back through
        // the same public API builtins and user extensions share.
        let declared = ctx
            .registrations(pid)
            .map_err(|e| ekko_err::err!("reading registrations of '{id}': {e}"))?;
        // Draw/compositor ops emitted during mount are drained (data-only;
        // the kernel discards them on a trap).
        let _ = ctx.take_ops(pid);
        Ok(Self {
            manifest: ExtensionManifest {
                id: id.to_string(),
                name: id.to_string(),
                version: "0".into(),
                description: "WASM extension through the cordis kernel".into(),
            },
            ctx: Arc::new(Mutex::new(ctx)),
            plugin: pid,
            declared,
        })
    }

    /// The units this module declared (`name, kind, descriptors` pairs,
    /// set 2).
    pub fn declared(&self) -> &[(String, String, Vec<String>)] {
        &self.declared
    }
}

impl Extension for WasmExtension {
    fn manifest(&self) -> ExtensionManifest {
        self.manifest.clone()
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        let ctx = Arc::clone(&self.ctx);
        let plugin = self.plugin;
        for (name, kind, descriptors) in &self.declared {
            match kind.as_str() {
                "command" => self.register_dispatch_command(host, &ctx, plugin, name)?,
                "keybinding" => {
                    self.register_dispatch_keybinding(host, &ctx, plugin, name, descriptors)?
                }
                "mode" => self.register_dispatch_mode(host, &ctx, plugin, name)?,
                "surface" => {
                    self.register_dispatch_surface(host, &ctx, plugin, name, descriptors)?
                }
                "overlay" => {
                    self.register_dispatch_overlay(host, &ctx, plugin, name, descriptors)?
                }
                "subscription" => self.register_dispatch_event(host, &ctx, plugin, name)?,
                other => log::info!(
                    "wasm extension '{}' declares {other} '{name}' (no dynamic dispatch for this kind yet)",
                    self.manifest.id
                ),
            }
        }
        Ok(())
    }
}

impl WasmExtension {
    /// Wire a registered command's [`CommandSpec::handler`] to the guest's
    /// `on_command` export. The host serializes the invocation and hands it
    /// to the guest as a JSON payload; the guest returns a JSON spec the host
    /// decodes into [`CommandOutput`] — the actions are applied by the host's
    /// single write path (functional-core, one write path).
    fn register_dispatch_command(
        &self,
        host: &mut dyn ExtensionHost,
        ctx: &Arc<Mutex<CordisContext>>,
        plugin: usize,
        name: &str,
    ) -> Result<()> {
        let ctx = Arc::clone(ctx);
        let name = name.to_string();
        host.register_command(CommandSpec {
            name: name.clone(),
            aliases: Vec::new(),
            description: String::new(),
            args_hint: String::new(),
            handler: Arc::new(move |invocation: crate::CommandInvocation| {
                let req = DispatchRequest {
                    kind: "command",
                    name: &name,
                    bytes: None,
                    args: Some(&invocation.raw_args),
                    snapshot: None,
                    event: None,
                };
                let raw = dispatch(&ctx, plugin, "on_command", req)?;
                serde_json::from_str(&raw).map_err(|e| {
                    ekko_err::err!("wasm '{name}' on_command returned invalid JSON: {e}")
                })
            }),
        })
    }

    /// Wire a registered keybinding's handler to the guest's `on_key` export.
    /// The kernel's set-2 registration keeps the chord as the unit name plus
    /// the remaining validated descriptors; the **mode** descriptor (the
    /// second registration field, `descriptors[0]`) routes the binding to
    /// that mode, or `None` (normal scope) when the guest registered no mode.
    /// The handler returns the guest's [`UiAction`]s.
    fn register_dispatch_keybinding(
        &self,
        host: &mut dyn ExtensionHost,
        ctx: &Arc<Mutex<CordisContext>>,
        plugin: usize,
        name: &str,
        descriptors: &[String],
    ) -> Result<()> {
        let Some(chords) = parse_key_chords(name) else {
            log::warn!(
                "wasm extension '{}' registered unparseable keybinding chord '{name}' (skipped)",
                self.manifest.id
            );
            return Ok(());
        };
        let chord_bytes = parse_key_binding(name).unwrap_or_default();
        // Keybinding descriptors are `[mode, description, handler]` (the
        // non-name fields the guest passes to `register_keybinding`). Mode
        // is the empty-or-absent normal scope; the description feeds the
        // hint bar / keybinding listing surfaced to extensions.
        let mode = descriptors
            .first()
            .map(String::as_str)
            .filter(|m| !m.is_empty())
            .map(str::to_string);
        let description = descriptors
            .get(1)
            .map(String::as_str)
            .unwrap_or("")
            .to_string();
        let ctx = Arc::clone(ctx);
        let name = name.to_string();
        host.register_keybinding(KeybindingSpec {
            chords,
            chord_text: name.clone(),
            mode,
            description,
            handler: Arc::new(move |snapshot: &ClientSnapshot| {
                let req = DispatchRequest {
                    kind: "keybinding",
                    name: &name,
                    bytes: Some(&chord_bytes),
                    args: None,
                    snapshot: Some(snapshot),
                    event: None,
                };
                match dispatch(&ctx, plugin, "on_key", req) {
                    Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
                    Err(e) => {
                        log::warn!("wasm on_key failed: {e:#}");
                        Vec::new()
                    }
                }
            }),
        })
    }

    /// Register a mode whose `on_key` / `render` hooks dispatch to the
    /// guest's `on_mode_key` / `on_mode_render` exports. Mode state lives in
    /// the guest's own WASM memory, so the host's [`ModeState`] is empty here;
    /// the payload carries the key bytes + immutable snapshot and the guest
    /// returns a [`ModeOutcome`]. `render` returns the cursor position (or
    /// `None`); bridging guest draw ops (`set 3`) onto the host
    /// [`DrawContext`](crate::DrawContext) is a documented follow-up.
    fn register_dispatch_mode(
        &self,
        host: &mut dyn ExtensionHost,
        ctx: &Arc<Mutex<CordisContext>>,
        plugin: usize,
        name: &str,
    ) -> Result<()> {
        let ctx = Arc::clone(ctx);
        let name = name.to_string();

        let key_ctx = Arc::clone(&ctx);
        let key_name = name.clone();
        let on_key = Arc::new(
            move |_state: &mut ModeState, bytes: &[u8], snapshot: &ClientSnapshot| {
                let req = DispatchRequest {
                    kind: "mode_key",
                    name: &key_name,
                    bytes: Some(bytes),
                    args: None,
                    snapshot: Some(snapshot),
                    event: None,
                };
                match dispatch(&key_ctx, plugin, "on_mode_key", req) {
                    Ok(raw) => serde_json::from_str(&raw).unwrap_or(ModeOutcome::Continue),
                    Err(e) => {
                        log::warn!("wasm on_mode_key '{key_name}' failed: {e:#}");
                        ModeOutcome::Continue
                    }
                }
            },
        );

        let render_ctx = Arc::clone(&ctx);
        let render_name = name.clone();
        let render = Arc::new(
            move |_draw: &mut dyn crate::DrawContext,
                  _state: &ModeState,
                  snapshot: &ClientSnapshot| {
                let req = DispatchRequest {
                    kind: "mode_render",
                    name: &render_name,
                    bytes: None,
                    args: None,
                    snapshot: Some(snapshot),
                    event: None,
                };
                match dispatch(&render_ctx, plugin, "on_mode_render", req) {
                    Ok(raw) => decode_cursor(&raw),
                    Err(e) => {
                        log::warn!("wasm on_mode_render '{render_name}' failed: {e:#}");
                        None
                    }
                }
            },
        );

        host.register_mode(ModeSpec {
            name,
            init_state: Arc::new(|| Box::new(()) as ModeState),
            on_key,
            render: Some(render),
        })
    }

    /// Wire a registered surface's draw closure to the guest's
    /// `on_surface_draw` export. The surface's `dock`/`priority`/`size`
    /// descriptors (recorded by the kernel as strings) reconstruct the
    /// [`SurfaceSpec`]; each frame the host dispatches to the guest, which
    /// emits set-3 draw ops that are drained and applied to the real
    /// [`DrawContext`](crate::DrawContext). This is what lets a WASM
    /// extension paint chrome (status bar, session panel) like which-key.
    fn register_dispatch_surface(
        &self,
        host: &mut dyn ExtensionHost,
        ctx: &Arc<Mutex<CordisContext>>,
        plugin: usize,
        name: &str,
        descriptors: &[String],
    ) -> Result<()> {
        // descriptors = [dock, priority, size] (all strings, from the kernel).
        let dock = descriptors
            .first()
            .and_then(|d| match d.as_str() {
                "0" => Some(DockEdge::Left),
                "1" => Some(DockEdge::Right),
                "2" => Some(DockEdge::Top),
                "3" => Some(DockEdge::Bottom),
                _ => None,
            })
            .unwrap_or(DockEdge::Top);
        let priority = descriptors
            .get(1)
            .and_then(|p| p.parse::<i32>().ok())
            .unwrap_or(0);
        let size = descriptors
            .get(2)
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(1);

        let ctx = Arc::clone(ctx);
        let name = name.to_string();
        let draw_ctx = Arc::clone(&ctx);
        let draw_name = name.clone();
        let draw: SurfaceDrawFn =
            Arc::new(move |dc: &mut dyn DrawContext, snapshot: &ClientSnapshot| {
                let req = DispatchRequest {
                    kind: "surface_draw",
                    name: &draw_name,
                    bytes: None,
                    args: None,
                    snapshot: Some(snapshot),
                    event: None,
                };
                match dispatch(&draw_ctx, plugin, "on_surface_draw", req) {
                    Ok(_) => {}
                    Err(e) => log::warn!("wasm on_surface_draw '{draw_name}' failed: {e:#}"),
                }
                // Drain the set-3 draw ops the guest emitted and apply them to
                // the real DrawContext.
                let ops = match draw_ctx.lock() {
                    Ok(mut g) => g.take_ops(plugin).unwrap_or_default(),
                    Err(e) => e.into_inner().take_ops(plugin).unwrap_or_default(),
                };
                apply_draw_ops(dc, &ops, snapshot);
            });

        // Optional `wants_tick` predicate: if the guest exports
        // `on_surface_wants_tick(snapshot) -> "true"/"false"`, drive it each
        // frame so a surface can ask to be repainted (e.g. the which-key top
        // bar's note/mode/flipper emphasis). Absent export -> always tick
        // (backward compatible, keeps the surface repainting every frame).
        let tick_ctx = Arc::clone(&ctx);
        let tick_name = name.clone();
        let wants_tick: SurfaceTickFn = Arc::new(move |snapshot: &ClientSnapshot| {
            let req = DispatchRequest {
                kind: "surface_wants_tick",
                name: &tick_name,
                bytes: None,
                args: None,
                snapshot: Some(snapshot),
                event: None,
            };
            match dispatch(&tick_ctx, plugin, "on_surface_wants_tick", req) {
                Ok(raw) => serde_json::from_str::<bool>(raw.trim()).unwrap_or(true),
                // Unknown export -> treat as "always tick".
                Err(_) => true,
            }
        });

        host.register_surface(SurfaceSpec {
            name,
            dock,
            priority,
            size: SurfaceSize::Fixed(size),
            hide_below: None,
            visible: None,
            draw,
            on_mouse: None,
            wants_tick: Some(wants_tick),
        })
    }

    /// Register a mode-attached (or free) overlay whose `render` / `handle_key`
    /// hooks dispatch to the guest's `on_overlay_render` / `on_overlay_key`
    /// exports. Overlay descriptors from the kernel are
    /// `[description, render, key, init, attach_mode]`.
    ///
    /// The render callback dispatches to the guest with a snapshot; the guest
    /// paints via set-3 draw ops which are drained and applied (exactly like a
    /// surface). The key callback dispatches with the key bytes and reads back
    /// one of the overlay outcomes: `null`/empty -> consume, `"close"` ->
    /// `Close`, or a JSON array of [`UiAction`]s -> `CloseWith`. Native overlay
    /// state is empty (`()`); the guest owns any modal state in its own memory.
    fn register_dispatch_overlay(
        &self,
        host: &mut dyn ExtensionHost,
        ctx: &Arc<Mutex<CordisContext>>,
        plugin: usize,
        name: &str,
        descriptors: &[String],
    ) -> Result<()> {
        // descriptors = [description, render, key, init, attach_mode].
        let description = descriptors.first().cloned().unwrap_or_default();
        let attach_mode = descriptors
            .get(4)
            .map(String::as_str)
            .filter(|m| !m.is_empty())
            .map(str::to_string);

        // Render -> guest render export; drain its draw ops like a surface.
        let render_ctx = Arc::clone(ctx);
        let render_name = name.to_string();
        let render: crate::overlay::OverlayRenderFn = Arc::new(
            move |dc: &mut dyn DrawContext,
                  _state: &mut crate::OverlayState,
                  snapshot: &ClientSnapshot| {
                let req = DispatchRequest {
                    kind: "overlay_render",
                    name: &render_name,
                    bytes: None,
                    args: None,
                    snapshot: Some(snapshot),
                    event: None,
                };
                if let Err(e) = dispatch(&render_ctx, plugin, "on_overlay_render", req) {
                    log::warn!("wasm on_overlay_render '{render_name}' failed: {e:#}");
                }
                let ops = match render_ctx.lock() {
                    Ok(mut g) => g.take_ops(plugin).unwrap_or_default(),
                    Err(e) => e.into_inner().take_ops(plugin).unwrap_or_default(),
                };
                apply_draw_ops(dc, &ops, snapshot);
            },
        );

        // Key -> guest key export; decode OverlayOutcome.
        let key_ctx = Arc::clone(ctx);
        let key_name = name.to_string();
        let handle_key: crate::overlay::OverlayKeyFn =
            Arc::new(move |_state: &mut crate::OverlayState, bytes: &[u8]| {
                let req = DispatchRequest {
                    kind: "overlay_key",
                    name: &key_name,
                    bytes: Some(bytes),
                    args: None,
                    snapshot: None,
                    event: None,
                };
                match dispatch(&key_ctx, plugin, "on_overlay_key", req) {
                    Ok(raw) => decode_overlay_outcome(&raw),
                    Err(e) => {
                        log::warn!("wasm on_overlay_key '{key_name}' failed: {e:#}");
                        crate::OverlayOutcome::None
                    }
                }
            });

        host.register_overlay(crate::OverlaySpec {
            name: name.to_string(),
            description,
            init_state: Arc::new(|_: Option<crate::OverlayPayload>| {
                Box::new(()) as crate::OverlayState
            }),
            render,
            handle_key,
            build_payload: None,
            attach_mode,
        })
    }

    /// Subscribe to the event the WASM extension named (its `subscribe` call's
    /// first field, the event name string), routing each dispatch to the
    /// guest's `on_event` export. The guest returns `null` for observe-only or
    /// an [`EventReturn`] the host applies (functional-core: return drives the
    /// host; the guest never mutates it).
    fn register_dispatch_event(
        &self,
        host: &mut dyn ExtensionHost,
        ctx: &Arc<Mutex<CordisContext>>,
        plugin: usize,
        name: &str,
    ) -> Result<()> {
        let kind = EventKind::from_name(name).ok_or_else(|| {
            ekko_err::err!(
                "wasm extension '{}' subscribed to unknown event '{name}'",
                self.manifest.id
            )
        })?;
        let ctx = Arc::clone(ctx);
        let name = name.to_string();
        host.subscribe(crate::EventHandlerRegistration {
            event: kind,
            label: name.clone(),
            handler: Arc::new(move |ev: LifecycleEvent| {
                let req = DispatchRequest {
                    kind: "event",
                    name: &name,
                    bytes: None,
                    args: None,
                    snapshot: None,
                    event: Some(&ev.payload),
                };
                let raw = dispatch(&ctx, plugin, "on_event", req)?.trim().to_string();
                if raw.is_empty() || raw == "null" {
                    return Ok(None);
                }
                serde_json::from_str(&raw)
                    .map(Some)
                    .map_err(|e| ekko_err::err!("wasm on_event returned invalid JSON: {e}"))
            }),
        })
    }
}

/// Host→guest dispatch payload (`cordis::Context::call`, set 6). One JSON
/// snapshot per call; the guest reads it (immutable — never `&mut` host
/// state) and returns an action string the host decodes. Fields are present
/// exactly per dispatch `kind`:
///
/// - `command`: `kind = "command"`, `name`, `args`
/// - `keybinding`: `kind = "keybinding"`, `name`, `bytes`, `snapshot`
/// - `mode_key`: `kind = "mode_key"`, `name`, `bytes`, `snapshot`
/// - `mode_render`: `kind = "mode_render"`, `name`, `snapshot`
/// - `event`: `kind = "event"`, `name`, `event`
#[derive(Serialize)]
struct DispatchRequest<'a> {
    kind: &'a str,
    /// The registered unit's name (command name / binding chord / mode name /
    /// event name).
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<&'a [u8]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    args: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot: Option<&'a ClientSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event: Option<&'a EventPayload>,
}

/// Drive one host->guest dispatch: serialize the request, lock this
/// extension's kernel, and call the named guest export with the payload.
/// The kernel writes the payload into the guest scratch, runs the export
/// fuel-metered, and reads the result string back. A trap / fuel-exhaustion
/// surfaces as an error and leaves no state residue (kernel-owned).
fn dispatch(
    ctx: &Mutex<CordisContext>,
    plugin: usize,
    export: &str,
    req: DispatchRequest<'_>,
) -> Result<String> {
    let payload = serde_json::to_string(&req)
        .map_err(|e| ekko_err::err!("wasm dispatch '{export}': serializing payload: {e}"))?;
    let mut guard = ctx.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .call(plugin, export, &payload)
        .map_err(|e| ekko_err::err!("wasm dispatch '{export}': {e}"))
}

/// Decode a mode-render cursor from the guest's JSON: `"null"`/empty/failure
/// -> `None`; `{"cursor": [row, col]}` -> `Some((row, col))`.
fn decode_cursor(raw: &str) -> Option<(i32, i32)> {
    #[derive(Serialize, Deserialize)]
    struct CursorReply {
        cursor: Option<(i32, i32)>,
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }
    serde_json::from_str::<CursorReply>(trimmed)
        .ok()
        .and_then(|r| r.cursor)
}

/// Decode a guest overlay-key reply into an [`OverlayOutcome`]: `null`/empty
/// -> consume (stay open), the JSON string `"close"` -> close, or a JSON array
/// of [`UiAction`]s -> close-and-apply. Anything else degrades to consume.
fn decode_overlay_outcome(raw: &str) -> crate::OverlayOutcome {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return crate::OverlayOutcome::None;
    }
    if trimmed == "\"close\"" || trimmed == "close" {
        return crate::OverlayOutcome::Close;
    }
    match serde_json::from_str::<Vec<UiAction>>(trimmed) {
        Ok(actions) if !actions.is_empty() => crate::OverlayOutcome::CloseWith(actions),
        _ => crate::OverlayOutcome::None,
    }
}

/// Apply a drained set-3 draw-op buffer to a real [`DrawContext`], resolving
/// color names against the snapshot's theme palette. Each op is a `(kind,
/// args)` pair where `args` are the validated string parameters the guest
/// passed (coordinates, text, color/style names). Unknown op kinds and
/// malformed args are skipped with a warning rather than panicking — a guest
/// drawing something the host can't express degrades gracefully.
fn apply_draw_ops(
    dc: &mut dyn DrawContext,
    ops: &[(String, Vec<String>)],
    snapshot: &ClientSnapshot,
) {
    for (kind, args) in ops {
        let n = |i: usize| -> i32 { args.get(i).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0) };
        let color = |i: usize| -> Color { resolve_color(args, i, snapshot) };
        match kind.as_str() {
            "fill_rect" => {
                let rect = Rect::new(n(0), n(1), n(2), n(3));
                let (fg, bg) = (color(4), Color::TRANSPARENT);
                dc.fill_rect(rect, fg, bg);
            }
            "set_cell" => {
                dc.set_cell(n(0), n(1), color(2), Color::TRANSPARENT, " ", false);
            }
            "put_text" => {
                let text = args.get(2).map(String::as_str).unwrap_or("");
                dc.put_text(n(0), n(1), 1 << 30, color(3), Color::TRANSPARENT, text);
            }
            "put_text_bold" => {
                let text = args.get(2).map(String::as_str).unwrap_or("");
                dc.put_text_bold(n(0), n(1), 1 << 30, color(3), Color::TRANSPARENT, text);
            }
            "put_text_styled" => {
                let text = args.get(2).map(String::as_str).unwrap_or("");
                let style = TextStyle {
                    fg: color(3),
                    bg: Color::TRANSPARENT,
                    reverse: false,
                    bold: false,
                };
                dc.put_text_styled(n(0), n(1), 1 << 30, text, style);
            }
            "draw_box" => {
                let rect = Rect::new(n(0), n(1), n(2), n(3));
                let border = color(4);
                dc.draw_box(rect, Color::TRANSPARENT, Color::TRANSPARENT, border);
            }
            "render_scrollbar" => {
                let model = ScrollbarModel {
                    visible_items: n(2).max(0) as usize,
                    total_items: n(3).max(0) as usize,
                    scroll_from_top: 0,
                };
                let style = ScrollbarStyle {
                    fg: color(4),
                    bg: Color::TRANSPARENT,
                    track_glyph: "│",
                    thumb_fg: color(5),
                    thumb_glyph: "┃",
                };
                dc.render_scrollbar(n(0), n(1), n(2), model, style);
            }
            other => log::warn!("wasm draw op '{other}' not applied (unknown kind)"),
        }
    }
}

/// Resolve a color argument by index: a named theme color (e.g. `"accent"`,
/// `"surface_raised"`, `"term_bg"`) maps to the snapshot palette; a
/// `#rrggbb` hex literal parses to an opaque [`Color`]; anything else falls
/// back to transparent.
fn resolve_color(args: &[String], i: usize, snapshot: &ClientSnapshot) -> Color {
    let Some(name) = args.get(i) else {
        return Color::TRANSPARENT;
    };
    let p = &snapshot.theme;

    match name.as_str() {
        "text" => p.text,
        "muted" => p.muted,
        "heading" => p.heading,
        "accent" => p.accent,
        "accent_2" => p.accent_2,
        "surface" => p.surface,
        "surface_raised" => p.surface_raised,
        "sidebar_bg" => p.sidebar_bg,
        "status_fg" => p.status_fg,
        "status_bg" => p.status_bg,
        "border" => p.border,
        "running" => p.running,
        "warning" => p.warning,
        "error" => p.error,
        "success" => p.success,
        "term_fg" => p.term_fg,
        "term_bg" => p.term_bg,
        "selection_fg" => p.selection_fg,
        "selection_bg" => p.selection_bg,
        "transparent" => Color::TRANSPARENT,
        _ => {
            // Try a #rrggbb hex literal.
            if let Some(hex) = name.strip_prefix('#')
                && let Ok(v) = u32::from_str_radix(hex, 16)
            {
                return Color::rgb(
                    ((v >> 16) & 0xff) as u8,
                    ((v >> 8) & 0xff) as u8,
                    (v & 0xff) as u8,
                );
            }
            Color::TRANSPARENT
        }
    }
}

/// Load every `*.wasm` file in `dir` (sorted by name) as an extension,
/// skipping — with a logged warning — modules that fail to compile or mount,
/// so one broken user module degrades to a warning instead of an unusable
/// terminal. Once mounted, [`WasmExtension::register`] builds the native specs
/// (command/keybinding/mode/subscription) whose handlers dispatch back to the
/// guest, so a duplicate spec name collides with a builtin exactly like the
/// native surface (hard build error); a module that only declares kinds the
/// bridge doesn't yet drive (overlay, theme, ...) is logged and left inert —
/// that stays the documented follow-up.
pub fn load_extensions(dir: &Path, _host: HostKind, _config: &Config) -> Vec<Box<dyn Extension>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "wasm"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| match WasmExtension::from_file(&path) {
            Ok(ext) => Some(Box::new(ext) as Box<dyn Extension>),
            Err(err) => {
                log::warn!("skipping wasm extension {}: {err:#}", path.display());
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compiled default config module loads at startup and yields
    /// [`Config::default`] — config really is data on the cordis ABI
    /// (set 1), with no privileged text parser.
    #[test]
    fn default_config_module_loads_at_startup() {
        let evaluator = WasmConfigEvaluator;
        let wasm = wat::parse_str(DEFAULT_CONFIG_WAT).expect("default config wat is valid");
        let config = evaluator.eval_config_wasm(&wasm).expect("config evaluates");
        let expected = Config::default();
        assert_eq!(
            config.general.scrollback_lines,
            expected.general.scrollback_lines
        );
        assert_eq!(config.sidebar_width(), expected.sidebar_width());
        assert_eq!(
            config.animation_interval_ms(),
            expected.animation_interval_ms()
        );
    }

    /// A user module that overrides a field through the `config` JSON key is
    /// honored (round-trip through the same kernel).
    #[test]
    fn user_config_module_overrides_defaults() {
        let json = r#"{"general":{"default_shell":"/bin/zsh"}}"#;
        // "config" key (6) then the JSON; WAT string literals need escaped
        // double quotes for the JSON.
        let json_escaped = json.replace('\"', "\\\"");
        let mut data = String::from("config");
        data.push_str(&json_escaped);
        let wat = format!(
            r#"
            (module
              (import "host" "ctx_set" (func $set (param i32 i32 i32 i32)))
              (memory (export "memory") 2)
              (data (i32.const 0) "{data}")
              (func (export "scratch") (result i32 i32) i32.const 1024 i32.const 1024)
              (func (export "mount")
                i32.const 0 i32.const 6
                i32.const 6 i32.const {json_len}
                call $set)
              (func (export "on_change") (param i32 i32))
            )
            "#,
            data = data,
            json_len = json.len()
        );
        let evaluator = WasmConfigEvaluator;
        let wasm_bytes = wat::parse_str(&wat).expect("valid wat");
        let config = evaluator
            .eval_config_wasm(&wasm_bytes)
            .expect("config evaluates");
        assert_eq!(config.general.default_shell, "/bin/zsh");
    }

    /// The finix `config.wasm` (the user's real settings, formerly `init.lua`)
    /// evaluates to the intended [`Config`]: the builtin chrome extensions are
    /// disabled, pane borders are framed with ASCII glyphs, layout is equal,
    /// and the animation interval is 33ms.
    #[test]
    fn finix_config_module_evaluates_to_intended_settings() {
        let evaluator = WasmConfigEvaluator;
        let wasm =
            wat::parse_str(include_str!("finix_config.wat")).expect("valid finix config wat");
        let config = evaluator.eval_config_wasm(&wasm).expect("config evaluates");
        assert_eq!(
            config.extensions.disabled,
            vec![
                "ekko-builtins.leader",
                "ekko-builtins.statusbar",
                "ekko-builtins.sidebar",
                "ekko-builtins.panes",
                "ekko-builtins.keybindings",
            ],
            "builtin chrome extensions are disabled (which-key owns them)"
        );
        assert_eq!(
            serde_json::to_value(config.ui.pane_borders).unwrap(),
            serde_json::json!("frame"),
            "pane borders are framed"
        );
        assert_eq!(
            serde_json::to_value(config.ui.pane_layout).unwrap(),
            serde_json::json!("equal"),
            "pane layout is equal"
        );
        assert_eq!(config.ui.animation_interval_ms, 33);
        let glyphs = config.ui.border_glyphs.expect("ascii border glyphs set");
        assert_eq!(glyphs.horizontal, '-');
        assert_eq!(glyphs.vertical, '|');
        assert_eq!(glyphs.junction, '+');
    }
    /// Minimal no-op [`DrawContext`](crate::DrawContext) for render dispatch.
    struct NoopDraw;
    impl crate::DrawContext for NoopDraw {
        fn size(&self) -> (i32, i32) {
            (120, 30)
        }
        fn fill_rect(&mut self, _: crate::Rect, _: crate::Color, _: crate::Color) {}
        fn set_cell(&mut self, _: i32, _: i32, _: crate::Color, _: crate::Color, _: &str, _: bool) {
        }
        fn put_text(&mut self, _: i32, _: i32, _: i32, _: crate::Color, _: crate::Color, _: &str) {}
        fn put_text_bold(
            &mut self,
            _: i32,
            _: i32,
            _: i32,
            _: crate::Color,
            _: crate::Color,
            _: &str,
        ) {
        }
        fn put_text_styled(&mut self, _: i32, _: i32, _: i32, _: &str, _: crate::TextStyle) {}
        fn draw_box(&mut self, _: crate::Rect, _: crate::Color, _: crate::Color, _: crate::Color) {}
        fn render_scrollbar(
            &mut self,
            _: i32,
            _: i32,
            _: i32,
            _: crate::ScrollbarModel,
            _: crate::ScrollbarStyle<'_>,
        ) {
        }
    }

    // ── Dynamic host→guest dispatch (cordis set 6) ──────────────────────────

    /// Escape a string for a WAT `(data ...)` literal (quotes + backslashes).
    fn wat_str(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }

    /// A `.wasm` guest that registers a command + a keybinding + a mode + a
    /// subscription and *responds* when the host dispatches: each dispatch
    /// export returns a fixed JSON action string the host decodes into the
    /// native return type. Proves a full payload-in / action-out round trip
    /// through the real runtime dispatch paths.
    fn demo_guest() -> Vec<u8> {
        // Sequential slabs, aligned: short string identities near the low
        // addresses, the two JSON return strings on wide slabs so the payload
        // and result can never overlap the identity bytes.
        let cmd_name = "hello";
        let cmd_desc = "from-wasm";
        let mode_name = "leader";
        let key_chord = "ctrl+q";
        let event = "bell";
        let cmd_res =
            r#"{"actions":[{"SetStatusNote":{"text":"from wasm","kind":"Info","ttl_ms":1000}}]}"#;
        // `serde` unit-variant `ModeOutcome::Exit`.
        let mode_res = r#""Exit""#;

        let cmd_off = 0usize;
        let desc_off = 16;
        let mode_off = 32;
        let key_off = 48;
        let event_off = 64;
        let cmd_res_off = 256;
        let mode_res_off = 1024;

        let data = [
            (cmd_off, cmd_name),
            (desc_off, cmd_desc),
            (mode_off, mode_name),
            (key_off, key_chord),
            (event_off, event),
            (cmd_res_off, cmd_res),
            (mode_res_off, mode_res),
        ];
        let data_items = data
            .iter()
            .map(|(off, s)| format!("(data (i32.const {off}) \"{}\")", wat_str(s)))
            .collect::<Vec<_>>()
            .join("\n  ");

        let wat = format!(
            r#"(module
  (import "host" "register_command" (func $reg_cmd (param i32 i32 i32 i32)))
  (import "host" "register_keybinding" (func $reg_key (param i32 i32 i32 i32 i32 i32 i32 i32)))
  (import "host" "register_mode" (func $reg_mode (param i32 i32 i32 i32 i32 i32 i32 i32)))
  (import "host" "subscribe" (func $subscribe (param i32 i32 i32 i32)))
  (memory (export "memory") 4)
  {data_items}
  (func (export "mount")
    ;; register_command("hello", "from-wasm")
    i32.const {cmd_off} i32.const {cmd_len}
    i32.const {desc_off} i32.const {desc_len}
    call $reg_cmd
    ;; register_keybinding("ctrl+q", "normal", "desc", "handler") — the
    ;; kernel keeps only the chord (first field) as the unit name.
    i32.const {key_off} i32.const {key_len}
    i32.const {mode_off} i32.const {mode_len}
    i32.const {mode_off} i32.const {mode_len}
    i32.const {mode_off} i32.const {mode_len}
    call $reg_key
    ;; register_mode("leader", key, init, render)
    i32.const {mode_off} i32.const {mode_len}
    i32.const {mode_off} i32.const {mode_len}
    i32.const {mode_off} i32.const {mode_len}
    i32.const {mode_off} i32.const {mode_len}
    call $reg_mode
    ;; subscribe("bell", "handler")
    i32.const {event_off} i32.const {event_len}
    i32.const {mode_off} i32.const {mode_len}
    call $subscribe)
  (func (export "on_change") (param i32 i32))
  (func (export "scratch") (result i32 i32) i32.const 4096 i32.const 4096)

  (func (export "on_command") (param $p i32) (param $l i32) (result i32 i32)
    i32.const {cmd_res_off} i32.const {cmd_res_len})
  (func (export "on_key") (param $p i32) (param $l i32) (result i32 i32)
    i32.const {mode_res_off} i32.const {mode_res_len})
  (func (export "on_mode_key") (param $p i32) (param $l i32) (result i32 i32)
    i32.const {mode_res_off} i32.const {mode_res_len})
  (func (export "on_mode_render") (param $p i32) (param $l i32) (result i32 i32)
    i32.const {mode_res_off} i32.const {mode_res_len})
  (func (export "on_event") (param $p i32) (param $l i32))
)"#,
            data_items = data_items,
            cmd_off = cmd_off,
            cmd_len = cmd_name.len(),
            desc_off = desc_off,
            desc_len = cmd_desc.len(),
            key_off = key_off,
            key_len = key_chord.len(),
            mode_off = mode_off,
            mode_len = mode_name.len(),
            event_off = event_off,
            event_len = event.len(),
            cmd_res_off = cmd_res_off,
            cmd_res_len = cmd_res.len(),
            mode_res_off = mode_res_off,
            mode_res_len = mode_res.len(),
        );
        wat::parse_str(&wat).expect("valid dynamic-dispatch guest")
    }

    /// A minimal-but-valid [`ClientSnapshot`] for driving key/mode callbacks.
    fn minimal_snapshot() -> ClientSnapshot {
        ClientSnapshot {
            session_name: "test".into(),
            mode: ClientSnapshot::NORMAL_MODE.into(),
            cols: 120,
            rows: 30,
            grid_cols: 120,
            grid_rows: 28,
            scrollback: 0,
            panes: Vec::new(),
            focused_pane: None,
            projects: Vec::new(),
            status_note: None,
            keybindings: Vec::new(),
            now_ms: 0,
            hidden_surfaces: Vec::new(),
            theme: crate::visual::ThemePalette::fallback(),
        }
    }

    #[test]
    fn wasm_dynamic_dispatch_round_trip_end_to_end() {
        let ext = WasmExtension::from_bytes(&demo_guest(), "demo")
            .expect("guest mounts and registers its units");
        let runtime = crate::RuntimeBuilder::new()
            .register_extension(ext)
            .build()
            .expect("runtime builds with the wasm command");

        // (1) command round trip: register_command then invoke_command hits
        // the guest's on_command export and the host decodes the returned
        // CommandOutput into the actions it applies.
        let dispatched = runtime.invoke_command(":hello");
        let crate::CommandDispatch::Invoked(actions) = dispatched else {
            panic!("invoke_command must reach the guest handler, got {dispatched:?}");
        };
        assert_eq!(
            actions,
            vec![UiAction::SetStatusNote {
                text: "from wasm".into(),
                kind: ekko_event::NoteKind::Info,
                ttl_ms: 1000,
            }]
        );

        // (2) keybinding: the registered "ctrl+q" is mode-scoped to "leader"
        // (the guest registered its mode descriptor), so it matches only in
        // leader mode — not normal scope. This proves the widened cordis
        // registration (mode descriptor retained) drives the native spec.
        assert!(
            runtime.match_keybinding(&[0x11], None).is_none(),
            "mode-scoped wasm keybinding must NOT match in normal scope"
        );
        assert!(
            runtime.match_keybinding(&[0x11], Some("leader")).is_some(),
            "mode-scoped wasm keybinding matches in leader mode"
        );

        // (3) mode key + render dispatch decode the guest outcomes.
        let mode = runtime.mode("leader").expect("wasm mode registered");
        let mut state = (mode.init_state)();
        let outcome = (mode.on_key)(&mut state, b"g", &minimal_snapshot());
        assert_eq!(
            outcome,
            ModeOutcome::Exit,
            "mode key dispatch decodes the guest outcome"
        );
        let cursor = (mode.render.as_ref().unwrap())(&mut NoopDraw, &state, &minimal_snapshot());
        assert_eq!(cursor, None, "on_mode_render 'Exit' JSON yields no cursor");

        // (4) subscription: the wasm module subscribed to `bell`.
        assert!(
            runtime.has_subscribers(EventKind::Bell),
            "wasm subscription is registered"
        );
    }

    /// A recording [`DrawContext`] that captures the ops applied, so a test
    /// can assert a WASM guest's draw ops actually reach the host surface.
    #[derive(Default)]
    struct RecordingDraw {
        fill_rects: Vec<Rect>,
        texts: Vec<String>,
        boxes: Vec<Rect>,
    }
    impl crate::DrawContext for RecordingDraw {
        fn size(&self) -> (i32, i32) {
            (120, 30)
        }
        fn fill_rect(&mut self, rect: Rect, _: Color, _: Color) {
            self.fill_rects.push(rect);
        }
        fn set_cell(&mut self, _: i32, _: i32, _: Color, _: Color, _: &str, _: bool) {}
        fn put_text(&mut self, _: i32, _: i32, _: i32, _: Color, _: Color, value: &str) {
            self.texts.push(value.to_string());
        }
        fn put_text_bold(&mut self, _: i32, _: i32, _: i32, _: Color, _: Color, value: &str) {
            self.texts.push(value.to_string());
        }
        fn put_text_styled(&mut self, _: i32, _: i32, _: i32, value: &str, _: TextStyle) {
            self.texts.push(value.to_string());
        }
        fn draw_box(&mut self, rect: Rect, _: Color, _: Color, _: Color) {
            self.boxes.push(rect);
        }
        fn render_scrollbar(
            &mut self,
            _: i32,
            _: i32,
            _: i32,
            _: ScrollbarModel,
            _: ScrollbarStyle<'_>,
        ) {
        }
    }

    /// A which-key-style guest: registers a mode-scoped keybinding (leader
    /// mode) and a docked surface whose draw closure emits set-3 draw ops.
    /// Proves the full path — registration descriptors -> native spec ->
    /// dispatch -> draw-op application — end to end.
    fn which_key_guest() -> Vec<u8> {
        // Data layout (offset: string):
        //   0:"j" 1:"leader" 7:"desc" 11:"handler" 18:"wk"
        //   20:"status" 26:"accent" 32:"10" 34:"1"
        let data = [
            (0usize, "j"),
            (1, "leader"),
            (7, "desc"),
            (11, "handler"),
            (18, "wk"),
            (20, "status"),
            (26, "accent"),
            (32, "10"),
            (34, "1"),
        ];
        let data_items = data
            .iter()
            .map(|(off, s)| format!("(data (i32.const {off}) \"{}\")", wat_str(s)))
            .collect::<Vec<_>>()
            .join("\n  ");
        let wat = format!(
            r#"(module
  (import "host" "register_keybinding" (func $reg_key (param i32 i32 i32 i32 i32 i32 i32 i32)))
  (import "host" "register_surface" (func $reg_surf (param i32 i32 i32 i32 i32)))
  (import "host" "put_text" (func $put_text (param i32 i32 i32 i32 i32 i32 i32 i32)))
  (import "host" "fill_rect" (func $fill_rect (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (memory (export "memory") 2)
  {data_items}
  (func (export "scratch") (result i32 i32) i32.const 1024 i32.const 1024)
  (func (export "mount")
    ;; register_keybinding("j", "leader", "desc", "handler")
    i32.const 0 i32.const 1
    i32.const 1 i32.const 6
    i32.const 7 i32.const 4
    i32.const 11 i32.const 7
    call $reg_key
    ;; register_surface("wk", dock=2(top), priority=0, size=1)
    i32.const 18 i32.const 2
    i32.const 2 i32.const 0 i32.const 1
    call $reg_surf)
  (func (export "on_change") (param i32 i32))
  (func (export "on_key") (param i32 i32) (result i32 i32)
    i32.const 0 i32.const 0)
  (func (export "on_surface_draw") (param i32 i32)
    ;; put_text(x=0, y=0, text="status"@20, color="accent"@26)
    i32.const 0 i32.const 1
    i32.const 0 i32.const 1
    i32.const 20 i32.const 6
    i32.const 26 i32.const 6
    call $put_text
    ;; fill_rect(x=0, y=0, w="10"@32, h="1"@34, color="accent"@26)
    i32.const 0 i32.const 1
    i32.const 0 i32.const 1
    i32.const 32 i32.const 2
    i32.const 34 i32.const 1
    i32.const 26 i32.const 6
    call $fill_rect)
)"#,
            data_items = data_items,
        );
        wat::parse_str(&wat).expect("valid which-key-style guest")
    }

    #[test]
    fn which_key_style_guest_draws_and_scopes_bindings() {
        let ext = WasmExtension::from_bytes(&which_key_guest(), "which-key")
            .expect("guest mounts and registers");
        let runtime = crate::RuntimeBuilder::new()
            .register_extension(ext)
            .build()
            .expect("runtime builds");

        // The mode-scoped binding matches in leader mode, not normal scope.
        assert!(
            runtime.match_keybinding(b"j", None).is_none(),
            "leader-scoped 'j' must not match in normal scope"
        );
        assert!(
            runtime.match_keybinding(b"j", Some("leader")).is_some(),
            "leader-scoped 'j' matches in leader mode"
        );

        // The registered surface is present with the reconstructed geometry.
        let surf = runtime.surface("wk").expect("surface registered");
        assert_eq!(surf.dock, DockEdge::Top);
        assert_eq!(surf.size, SurfaceSize::Fixed(1));

        // Driving its draw closure dispatches to the guest and applies the
        // drained draw ops to a real DrawContext.
        let mut dc = RecordingDraw::default();
        (surf.draw)(&mut dc, &minimal_snapshot());
        assert_eq!(dc.texts, vec!["status"], "guest put_text applied");
        assert_eq!(
            dc.fill_rects,
            vec![Rect::new(0, 0, 10, 1)],
            "guest fill_rect applied"
        );
    }

    /// An overlay guest: registers a leader-attached session-list panel whose
    /// render emits a `draw_box`, and whose key handler returns a close-with
    /// action. Proves the overlay descriptors -> OverlaySpec ->
    /// on_overlay_render/on_overlay_key dispatch -> draw-op application path.
    #[test]
    fn overlay_dispatch_renders_and_handles_keys() {
        // Data layout (offset: string):
        //   0:"sessions" 8:"sessions" 16:"leader"
        //   22:"" 30:"panel" 40:"accent"
        //   48: JSON reply for on_overlay_key: ["exit_mode"]
        let data = [
            (0usize, "sessions"), // overlay name
            (8, ""),              // description
            (9, ""),              // render tag
            (9, ""),              // key tag
            (9, ""),              // init tag
            (10, "leader"),       // attach_mode
            (17, ""),             // padding
            (22, "panel"),        // text drawn by render
            (28, "1"),            // box height
        ];
        let data_items = data
            .iter()
            .map(|(off, s)| format!("(data (i32.const {off}) \"{}\")", wat_str(s)))
            .collect::<Vec<_>>()
            .join("\n  ");
        // The JSON action reply lives at offset 512 (outside the data layout).
        let reply = r#"["ExitMode","Detach"]"#;
        // Escape the reply's quotes/backslashes into the `(data ...)` string
        // via wat_str, which emits a valid wasm string literal.
        let reply_wat = format!("\"{}\"", wat_str(reply));
        let wat = format!(
            r#"(module
  (import "host" "register_overlay" (func $reg_ov
    (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (import "host" "draw_box" (func $draw_box (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (memory (export "memory") 2)
  {data_items}
  (data (i32.const 512) {reply_wat})
  (func (export "scratch") (result i32 i32) i32.const 1024 i32.const 1024)
  (func (export "mount")
    ;; register_overlay("sessions", desc, render, key, init, attach="leader")
    i32.const 0 i32.const 8
    i32.const 8 i32.const 0
    i32.const 9 i32.const 0
    i32.const 9 i32.const 0
    i32.const 9 i32.const 0
    i32.const 10 i32.const 6
    call $reg_ov)
  (func (export "on_change") (param i32 i32))
  (func (export "on_overlay_render") (param i32 i32)
    ;; draw_box(x=0, y=0, w=5, h=1, color="accent")
    i32.const 0 i32.const 1
    i32.const 0 i32.const 1
    i32.const 22 i32.const 1
    i32.const 28 i32.const 1
    i32.const 40 i32.const 6
    call $draw_box)
  (func (export "on_overlay_key") (param i32 i32) (result i32 i32)
    i32.const 512 i32.const {reply_len})
)"#,
            data_items = data_items,
            reply_wat = reply_wat,
            reply_len = reply.len(),
        );
        let wasm = wat::parse_str(&wat).expect("valid overlay guest");
        let ext = WasmExtension::from_bytes(&wasm, "overlay-ext").expect("overlay mounts");
        let runtime = crate::RuntimeBuilder::new()
            .register_extension(ext)
            .build()
            .expect("runtime builds");

        // The leader-attached overlay is present and attached to leader.
        let ov = runtime.overlay("sessions").expect("overlay registered");
        assert_eq!(ov.attach_mode.as_deref(), Some("leader"), "leader-attached");
        assert!(
            runtime.overlay_attached_to("leader").is_some(),
            "overlay attached to leader mode"
        );

        // Driving its render dispatches to on_overlay_render and applies draw ops.
        let mut dc = RecordingDraw::default();
        let mut state = (ov.init_state)(None);
        (ov.render)(&mut dc, &mut state, &minimal_snapshot());
        // The guest emitted one draw_box; RecordingDraw captures it as a box.
        assert_eq!(dc.boxes.len(), 1, "overlay render emitted a draw_box");

        // Driving its key handler decodes a close-with action reply.
        let outcome = (ov.handle_key)(&mut state, b"\x1b");
        assert!(
            matches!(
                outcome,
                crate::OverlayOutcome::CloseWith(ref a)
                    if a.iter().any(|x| matches!(x, UiAction::ExitMode))
                        && a.iter().any(|x| matches!(x, UiAction::Detach))
            ),
            "overlay key dispatch decodes CloseWith actions, got {outcome:?}"
        );
    }

    /// Guard against the which-key JSON-shape bug: `UiAction` is serde-externally
    /// tagged, so a unit variant decodes as a bare `"ExitMode"` and a struct
    /// variant as `{"EnterMode":{...}}`. A guest returning the Lua-style
    /// `[EnterMode,{...}]` won't parse and degrades to "no action" — exactly why
    /// keybinds silently did nothing. This asserts the correct forms parse.
    #[test]
    fn ui_action_serde_shape_is_externally_tagged() {
        use ekko_event::UiAction;
        // Unit + struct variants, mixed in one array: valid.
        let good = serde_json::from_str::<Vec<UiAction>>(
            r#"["ExitMode",{"NewSession":{"name":null}},{"SwitchSession":{"name":"s2"}},{"SetStatusNote":{"text":"no other session","kind":"Info","ttl_ms":2000}}]"#,
        )
        .expect("mixed unit+struct variants parse");
        assert!(matches!(good[0], UiAction::ExitMode));
        assert!(matches!(good[1], UiAction::NewSession { .. }));
        assert!(matches!(good[2], UiAction::SwitchSession { .. }));

        // A struct variant MUST be tagged as an object {"Variant":{...}}; the
        // Lua-style ["EnterMode",{...}] (two array elements) must NOT parse as
        // a single action — it would mis-deserialize or fail.
        assert!(
            serde_json::from_str::<Vec<UiAction>>(r#"["EnterMode",{"name":"leader"}]"#).is_err()
                || serde_json::from_str::<Vec<UiAction>>(r#"["EnterMode",{"name":"leader"}]"#)
                    .map(|v| v.len() != 1)
                    .unwrap_or(true),
            "Lua-style tuple action form must not decode as one action"
        );
        // The correct object form parses as a single EnterMode.
        let one = serde_json::from_str::<Vec<UiAction>>(r#"[{"EnterMode":{"name":"leader"}}]"#)
            .expect("tagged struct variant parses");
        assert!(matches!(one.as_slice(), [UiAction::EnterMode { .. }]));
    }
}
