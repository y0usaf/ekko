//! The assembled, immutable extension runtime: registries plus the sync
//! event dispatcher. ekko has no async runtime — the dispatcher gets phi's
//! per-kind-timeout semantics from `std::thread` + `mpsc::recv_timeout`
//! instead of tokio: run each handler on its own thread, bound the wait,
//! and CONTINUE past errors/timeouts so one misbehaving extension never
//! wedges the host. A timed-out handler's thread is detached, not killed.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::Duration;

use ekko_event::{
    EventHandlerRegistration, EventKind, EventPayload, EventReturn, LifecycleEvent, UiAction,
};

use crate::host::RegistryHost;
use crate::{
    CommandInfo, CommandInvocation, CommandSpec, ExtensionManifest, KeybindingInfo, KeybindingSpec,
    ModeSpec, OverlaySpec, SessionGrouperSpec, SessionNamerSpec, SpinnerSpec, SurfaceSpec,
    ThemeSpec,
};

/// Per-[`EventKind`] timeout budget for a single extension handler:
/// - **Notifications** (return discarded, can be frequent) get a tight
///   bound so a slow extension can't stall the input or render loop.
/// - **Interception gates** (return drives control flow) get a longer bound
///   since their result is consumed.
/// - **One-shot lifecycle** events (may do real work) get the longest bound.
fn handler_timeout(kind: EventKind) -> Duration {
    use EventKind::*;
    match kind {
        // Notifications.
        SessionDetached | SessionSwitched | SessionListRefreshed | GridUpdated | Bell | Resize
        | Tick | ModeChanged | ClientDetached | PtyResized | Heartbeat => {
            Duration::from_millis(100)
        }
        // Interception gates.
        KeyInput | CommandInvoked | BeforeSessionDetach | BeforeSessionSwitch | BeforePtySpawn => {
            Duration::from_millis(500)
        }
        // One-shot lifecycle.
        ClientReady | SessionAttached | SessionCreated | ClientAttached | SessionExited => {
            Duration::from_secs(2)
        }
    }
}

pub struct AppRuntime {
    manifests: Vec<ExtensionManifest>,
    commands: BTreeMap<String, CommandSpec>,
    command_aliases: BTreeMap<String, String>,
    keybindings: Vec<KeybindingSpec>,
    modes: BTreeMap<String, ModeSpec>,
    surfaces: BTreeMap<String, SurfaceSpec>,
    overlays: BTreeMap<String, OverlaySpec>,
    themes: BTreeMap<String, ThemeSpec>,
    spinners: BTreeMap<String, SpinnerSpec>,
    session_grouper: Option<SessionGrouperSpec>,
    session_namer: Option<SessionNamerSpec>,
    event_handlers: Vec<EventHandlerRegistration>,
}

/// Outcome of routing a `:command` line through the registry.
#[derive(Debug, PartialEq, Eq)]
pub enum CommandDispatch {
    /// Nothing to run (empty line).
    Empty,
    /// No command registered under that name.
    NotFound(String),
    /// A `CommandInvoked` subscriber canceled the invocation.
    Canceled(String),
    /// The handler ran; apply its actions.
    Invoked(Vec<UiAction>),
    /// The handler itself errored.
    Failed(String),
}

impl AppRuntime {
    pub(crate) fn from_registry(manifests: Vec<ExtensionManifest>, host: RegistryHost) -> Self {
        Self {
            manifests,
            commands: host.commands,
            command_aliases: host.command_aliases,
            keybindings: host.keybindings,
            modes: host.modes,
            surfaces: host.surfaces,
            overlays: host.overlays,
            themes: host.themes,
            spinners: host.spinners,
            session_grouper: host.session_grouper,
            session_namer: host.session_namer,
            event_handlers: host.event_handlers,
        }
    }

    /// An empty runtime (the bare harness).
    pub fn empty() -> Self {
        Self::from_registry(Vec::new(), RegistryHost::default())
    }

    pub fn manifests(&self) -> &[ExtensionManifest] {
        &self.manifests
    }

    // ── Event dispatch ──────────────────────────────────────────────────────

    /// Dispatch an event to every subscribed handler, bounded per handler by
    /// the kind's timeout budget. Never blocks past `handlers × budget`;
    /// never fails: handler errors/timeouts are logged and skipped.
    pub fn dispatch(&self, kind: EventKind, payload: EventPayload) -> Vec<EventReturn> {
        self.dispatch_labeled(kind, payload)
            .into_iter()
            .map(|(_, value)| value)
            .collect()
    }

    /// Like [`Self::dispatch`], but pairs each return with the handler label
    /// that produced it (used e.g. to attribute notices to their source).
    pub fn dispatch_labeled(
        &self,
        kind: EventKind,
        payload: EventPayload,
    ) -> Vec<(String, EventReturn)> {
        let event = LifecycleEvent { kind, payload };
        let timeout = handler_timeout(kind);
        let mut returns = Vec::new();
        for reg in self.event_handlers.iter().filter(|h| h.event == kind) {
            let (tx, rx) = mpsc::channel();
            let handler = reg.handler.clone();
            let ev = event.clone();
            std::thread::spawn(move || {
                let _ = tx.send(handler(ev));
            });
            match rx.recv_timeout(timeout) {
                Ok(Ok(Some(value))) => returns.push((reg.label.clone(), value)),
                Ok(Ok(None)) => {}
                Ok(Err(e)) => log::warn!("extension handler '{}' errored: {e:#}", reg.label),
                Err(_) => log::warn!(
                    "extension handler '{}' timed out after {timeout:?} (thread detached)",
                    reg.label
                ),
            }
        }
        returns
    }

    /// Dispatch and report whether any handler returned `Cancel`.
    pub fn dispatch_cancelable(&self, kind: EventKind, payload: EventPayload) -> Option<String> {
        self.dispatch(kind, payload).into_iter().find_map(|r| {
            if let EventReturn::Cancel { reason } = r {
                Some(reason)
            } else {
                None
            }
        })
    }

    /// Whether any handler subscribes to `kind` (lets hosts skip payload
    /// construction on hot paths).
    pub fn has_subscribers(&self, kind: EventKind) -> bool {
        self.event_handlers.iter().any(|h| h.event == kind)
    }

    // ── Commands ───────────────────────────────────────────────────────────

    pub fn command(&self, name: &str) -> Option<&CommandSpec> {
        self.commands.get(name).or_else(|| {
            self.command_aliases
                .get(name)
                .and_then(|canonical| self.commands.get(canonical))
        })
    }

    pub fn command_infos(&self) -> Vec<CommandInfo> {
        self.commands
            .values()
            .map(|c| CommandInfo {
                name: c.name.clone(),
                aliases: c.aliases.clone(),
                args_hint: c.args_hint.clone(),
                description: c.description.clone(),
            })
            .collect()
    }

    /// Parse and run a `:command` line: resolve the name (or alias), fire
    /// the `CommandInvoked` gate, then run the handler on the caller's
    /// thread. This is the single command dispatch path — chords, command
    /// mode, and extensions all funnel through it.
    pub fn invoke_command(&self, line: &str) -> CommandDispatch {
        let line = line.strip_prefix(':').unwrap_or(line).trim();
        if line.is_empty() {
            return CommandDispatch::Empty;
        }
        let (head, rest) = match line.split_once(char::is_whitespace) {
            Some((head, rest)) => (head, rest.trim()),
            None => (line, ""),
        };
        let Some(spec) = self.command(head) else {
            return CommandDispatch::NotFound(line.to_string());
        };
        if let Some(reason) = self.dispatch_cancelable(
            EventKind::CommandInvoked,
            EventPayload::CommandInvoked {
                name: spec.name.clone(),
                raw_args: rest.to_string(),
            },
        ) {
            return CommandDispatch::Canceled(reason);
        }
        match (spec.handler)(CommandInvocation {
            raw_args: rest.to_string(),
        }) {
            Ok(output) => CommandDispatch::Invoked(output.actions),
            Err(e) => CommandDispatch::Failed(format!("{e:#}")),
        }
    }

    // ── Keybindings ────────────────────────────────────────────────────────

    /// Match a raw input chunk against the registry, scoped to the active
    /// mode (`None` = normal). First registration wins.
    pub fn match_keybinding(&self, bytes: &[u8], mode: Option<&str>) -> Option<&KeybindingSpec> {
        self.keybindings.iter().find(|b| {
            b.mode.as_deref() == mode && b.chords.iter().any(|chord| chord.as_slice() == bytes)
        })
    }

    pub fn keybinding_infos(&self) -> Vec<KeybindingInfo> {
        self.keybindings
            .iter()
            .map(|b| KeybindingInfo {
                chord_text: b.chord_text.clone(),
                mode: b.mode.clone(),
                description: b.description.clone(),
            })
            .collect()
    }

    /// Listing of the live registries, handed to an overlay's
    /// `build_payload` when the host opens it.
    pub fn registry_view(&self) -> crate::RegistryView {
        crate::RegistryView {
            keybindings: self.keybinding_infos(),
            commands: self.command_infos(),
        }
    }

    // ── Modes / surfaces / overlays / visuals ──────────────────────────────

    pub fn mode(&self, name: &str) -> Option<&ModeSpec> {
        self.modes.get(name)
    }

    pub fn surfaces(&self) -> Vec<&SurfaceSpec> {
        self.surfaces.values().collect()
    }

    /// The surfaces that should claim layout regions this cycle: those with
    /// no `visible` predicate, plus those whose predicate returns `true` —
    /// minus any the user has toggled off (`snapshot.hidden_surfaces`, fed
    /// by `UiAction::ToggleSurface`), which the toggle overrides.
    pub fn visible_surfaces(&self, snapshot: &crate::ClientSnapshot) -> Vec<&SurfaceSpec> {
        self.surfaces
            .values()
            .filter(|spec| {
                !snapshot.hidden_surfaces.contains(&spec.name)
                    && spec
                        .visible
                        .as_ref()
                        .is_none_or(|visible| visible(snapshot))
            })
            .collect()
    }

    pub fn surface(&self, name: &str) -> Option<&SurfaceSpec> {
        self.surfaces.get(name)
    }

    pub fn overlay(&self, name: &str) -> Option<&OverlaySpec> {
        self.overlays.get(name)
    }

    /// The overlay attached to `mode` (via [`OverlaySpec::attach_mode`]),
    /// if any. First match in name order wins; registering two overlays
    /// attached to the same mode is a configuration error worth surfacing,
    /// but the host degrades to the deterministic first.
    pub fn overlay_attached_to(&self, mode: &str) -> Option<&OverlaySpec> {
        self.overlays
            .values()
            .find(|o| o.attach_mode.as_deref() == Some(mode))
    }

    pub fn theme(&self, name: &str) -> Option<&ThemeSpec> {
        self.themes.get(name)
    }

    /// The theme to use: the named one if registered, else the first
    /// registered, else `None` (host falls back to its minimal palette).
    pub fn resolve_theme(&self, preferred: Option<&str>) -> Option<&ThemeSpec> {
        preferred
            .and_then(|name| self.themes.get(name))
            .or_else(|| self.themes.values().next())
    }

    pub fn spinner(&self, name: &str) -> Option<&SpinnerSpec> {
        self.spinners.get(name)
    }

    pub fn resolve_spinner(&self, preferred: Option<&str>) -> Option<&SpinnerSpec> {
        preferred
            .and_then(|name| self.spinners.get(name))
            .or_else(|| self.spinners.values().next())
    }

    pub fn session_grouper(&self) -> Option<&SessionGrouperSpec> {
        self.session_grouper.as_ref()
    }

    pub fn session_namer(&self) -> Option<&SessionNamerSpec> {
        self.session_namer.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandOutput, Extension, ExtensionHost, ExtensionManifest, RuntimeBuilder};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    type RegisterFn = Box<dyn Fn(&mut dyn ExtensionHost) -> anyhow::Result<()> + Send + Sync>;

    struct TestExt {
        register: RegisterFn,
    }

    impl TestExt {
        fn new(
            register: impl Fn(&mut dyn ExtensionHost) -> anyhow::Result<()> + Send + Sync + 'static,
        ) -> Self {
            Self {
                register: Box::new(register),
            }
        }
    }

    impl Extension for TestExt {
        fn manifest(&self) -> ExtensionManifest {
            ExtensionManifest {
                id: "test".into(),
                name: "test".into(),
                version: "0".into(),
                description: String::new(),
            }
        }

        fn register(&self, host: &mut dyn ExtensionHost) -> anyhow::Result<()> {
            (self.register)(host)
        }
    }

    fn subscription(
        kind: EventKind,
        label: &str,
        handler: impl Fn(LifecycleEvent) -> anyhow::Result<Option<EventReturn>> + Send + Sync + 'static,
    ) -> EventHandlerRegistration {
        EventHandlerRegistration {
            event: kind,
            label: label.to_string(),
            handler: Arc::new(handler),
        }
    }

    #[test]
    fn dispatch_collects_returns_and_skips_observers() {
        let runtime = RuntimeBuilder::new()
            .register_extension(TestExt::new(|host| {
                host.subscribe(subscription(EventKind::Bell, "observer", |_| Ok(None)))?;
                host.subscribe(subscription(EventKind::Bell, "canceler", |_| {
                    Ok(Some(EventReturn::Cancel {
                        reason: "no".into(),
                    }))
                }))?;
                Ok(())
            }))
            .build()
            .unwrap();
        let returns = runtime.dispatch(EventKind::Bell, EventPayload::Empty);
        assert_eq!(returns.len(), 1);
        assert!(matches!(&returns[0], EventReturn::Cancel { reason } if reason == "no"));
    }

    #[test]
    fn dispatch_continues_past_erroring_handler() {
        let runtime = RuntimeBuilder::new()
            .register_extension(TestExt::new(|host| {
                host.subscribe(subscription(EventKind::Bell, "boom", |_| {
                    anyhow::bail!("boom")
                }))?;
                host.subscribe(subscription(EventKind::Bell, "ok", |_| {
                    Ok(Some(EventReturn::Cancel {
                        reason: "after".into(),
                    }))
                }))?;
                Ok(())
            }))
            .build()
            .unwrap();
        let returns = runtime.dispatch(EventKind::Bell, EventPayload::Empty);
        assert_eq!(returns.len(), 1);
    }

    #[test]
    fn dispatch_times_out_blocked_handler_and_continues() {
        let runtime = RuntimeBuilder::new()
            .register_extension(TestExt::new(|host| {
                host.subscribe(subscription(EventKind::Bell, "sleeper", |_| {
                    std::thread::sleep(Duration::from_secs(10));
                    Ok(None)
                }))?;
                host.subscribe(subscription(EventKind::Bell, "fast", |_| {
                    Ok(Some(EventReturn::Cancel {
                        reason: "fast".into(),
                    }))
                }))?;
                Ok(())
            }))
            .build()
            .unwrap();
        let start = std::time::Instant::now();
        let returns = runtime.dispatch(EventKind::Bell, EventPayload::Empty);
        assert_eq!(returns.len(), 1);
        // Bell is a notification: 100ms budget, well under the 10s sleep.
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn duplicate_command_registration_fails_loudly() {
        fn cmd(name: &str) -> CommandSpec {
            CommandSpec {
                name: name.into(),
                aliases: vec![],
                description: String::new(),
                args_hint: String::new(),
                handler: Arc::new(|_| Ok(CommandOutput::none())),
            }
        }
        let result = RuntimeBuilder::new()
            .register_extension(TestExt::new(|host| {
                host.register_command(cmd("kill"))?;
                host.register_command(cmd("kill"))?;
                Ok(())
            }))
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn duplicate_keybinding_chord_fails_loudly_within_a_mode_scope() {
        fn binding(chord: &[u8], mode: Option<&str>) -> crate::KeybindingSpec {
            crate::KeybindingSpec {
                chords: vec![chord.to_vec()],
                chord_text: String::from_utf8_lossy(chord).into_owned(),
                mode: mode.map(str::to_string),
                description: String::new(),
                handler: Arc::new(|_| Vec::new()),
            }
        }
        // Same chord in the same scope: hard error.
        let result = RuntimeBuilder::new()
            .register_extension(TestExt::new(|host| {
                host.register_keybinding(binding(b"\x11", None))?;
                host.register_keybinding(binding(b"\x11", None))?;
                Ok(())
            }))
            .build();
        assert!(result.is_err());
        // Same chord in different mode scopes: fine.
        let result = RuntimeBuilder::new()
            .register_extension(TestExt::new(|host| {
                host.register_keybinding(binding(b"\x11", None))?;
                host.register_keybinding(binding(b"\x11", Some("leader")))?;
                Ok(())
            }))
            .build();
        assert!(result.is_ok());
    }

    #[test]
    fn alias_collision_with_command_name_fails_loudly() {
        let result = RuntimeBuilder::new()
            .register_extension(TestExt::new(|host| {
                host.register_command(CommandSpec {
                    name: "quit".into(),
                    aliases: vec![],
                    description: String::new(),
                    args_hint: String::new(),
                    handler: Arc::new(|_| Ok(CommandOutput::none())),
                })?;
                host.register_command(CommandSpec {
                    name: "detach".into(),
                    aliases: vec!["quit".into()],
                    description: String::new(),
                    args_hint: String::new(),
                    handler: Arc::new(|_| Ok(CommandOutput::none())),
                })?;
                Ok(())
            }))
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn invoke_command_resolves_aliases_and_passes_args() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in = calls.clone();
        let runtime = RuntimeBuilder::new()
            .register_extension(TestExt::new(move |host| {
                let calls = calls_in.clone();
                host.register_command(CommandSpec {
                    name: "detach".into(),
                    aliases: vec!["q".into(), "quit".into()],
                    description: String::new(),
                    args_hint: String::new(),
                    handler: Arc::new(move |inv| {
                        assert_eq!(inv.raw_args, "");
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(CommandOutput::action(UiAction::Detach))
                    }),
                })
            }))
            .build()
            .unwrap();
        assert_eq!(
            runtime.invoke_command(":q"),
            CommandDispatch::Invoked(vec![UiAction::Detach])
        );
        assert_eq!(
            runtime.invoke_command("quit"),
            CommandDispatch::Invoked(vec![UiAction::Detach])
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(runtime.invoke_command(":"), CommandDispatch::Empty);
        assert!(matches!(
            runtime.invoke_command(":bogus"),
            CommandDispatch::NotFound(_)
        ));
    }

    #[test]
    fn command_invoked_gate_can_cancel() {
        let runtime = RuntimeBuilder::new()
            .register_extension(TestExt::new(|host| {
                host.register_command(CommandSpec {
                    name: "kill".into(),
                    aliases: vec![],
                    description: String::new(),
                    args_hint: String::new(),
                    handler: Arc::new(|_| Ok(CommandOutput::action(UiAction::KillCurrentSession))),
                })?;
                host.subscribe(subscription(EventKind::CommandInvoked, "guard", |ev| {
                    if let EventPayload::CommandInvoked { name, .. } = &ev.payload
                        && name == "kill"
                    {
                        return Ok(Some(EventReturn::Cancel {
                            reason: "protected".into(),
                        }));
                    }
                    Ok(None)
                }))?;
                Ok(())
            }))
            .build()
            .unwrap();
        assert_eq!(
            runtime.invoke_command("kill"),
            CommandDispatch::Canceled("protected".into())
        );
    }

    #[test]
    fn visible_surfaces_respects_the_predicate() {
        use crate::{DockEdge, SurfaceSize, SurfaceSpec};
        let shown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shown_in = shown.clone();
        let runtime = RuntimeBuilder::new()
            .register_extension(TestExt::new(move |host| {
                let shown = shown_in.clone();
                host.register_surface(SurfaceSpec {
                    name: "always".into(),
                    dock: DockEdge::Bottom,
                    priority: 0,
                    size: SurfaceSize::Fixed(1),
                    hide_below: None,
                    visible: None,
                    draw: Arc::new(|_, _| {}),
                    on_mouse: None,
                    wants_tick: None,
                })?;
                host.register_surface(SurfaceSpec {
                    name: "toggled".into(),
                    dock: DockEdge::Bottom,
                    priority: 1,
                    size: SurfaceSize::Fixed(1),
                    hide_below: None,
                    visible: Some(Arc::new(move |_| shown.load(Ordering::SeqCst))),
                    draw: Arc::new(|_, _| {}),
                    on_mouse: None,
                    wants_tick: None,
                })
            }))
            .build()
            .unwrap();
        let snapshot = crate::ClientSnapshot {
            panes: vec![],
            focused_pane: None,
            session_name: String::new(),
            mode: crate::ClientSnapshot::NORMAL_MODE.into(),
            cols: 80,
            rows: 24,
            grid_cols: 80,
            grid_rows: 24,
            scrollback: 0,
            projects: Vec::new(),
            status_note: None,
            keybindings: vec![],
            now_ms: 0,
            hidden_surfaces: Vec::new(),
            theme: crate::ThemePalette::fallback(),
        };
        let names =
            |specs: Vec<&SurfaceSpec>| specs.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(runtime.visible_surfaces(&snapshot)), vec!["always"]);
        shown.store(true, Ordering::SeqCst);
        assert_eq!(
            names(runtime.visible_surfaces(&snapshot)),
            vec!["always", "toggled"]
        );
        // The unfiltered listing is unaffected.
        assert_eq!(runtime.surfaces().len(), 2);
        // A user toggle (`UiAction::ToggleSurface`) hides a surface even
        // when its predicate says visible (or it has none).
        let hidden_snapshot = crate::ClientSnapshot {
            hidden_surfaces: vec!["always".into()],
            ..snapshot
        };
        assert_eq!(
            names(runtime.visible_surfaces(&hidden_snapshot)),
            vec!["toggled"]
        );
    }

    #[test]
    fn overlay_attached_to_finds_the_mode_attached_overlay() {
        fn overlay(name: &str, attach_mode: Option<&str>) -> crate::OverlaySpec {
            crate::OverlaySpec {
                name: name.into(),
                description: String::new(),
                init_state: Arc::new(|_| Box::new(()) as crate::OverlayState),
                render: Arc::new(|_, _, _| {}),
                handle_key: Arc::new(|_, _| crate::OverlayOutcome::None),
                build_payload: None,
                attach_mode: attach_mode.map(Into::into),
            }
        }
        let runtime = RuntimeBuilder::new()
            .register_extension(TestExt::new(|host| {
                host.register_overlay(overlay("free", None))?;
                host.register_overlay(overlay("panel", Some("leader")))
            }))
            .build()
            .unwrap();
        assert_eq!(
            runtime
                .overlay_attached_to("leader")
                .map(|o| o.name.as_str()),
            Some("panel")
        );
        assert!(runtime.overlay_attached_to("scroll").is_none());
    }

    #[test]
    fn registry_view_lists_live_registrations() {
        let runtime = RuntimeBuilder::new()
            .register_extension(TestExt::new(|host| {
                host.register_command(CommandSpec {
                    name: "detach".into(),
                    aliases: vec![],
                    description: "detach from the session".into(),
                    args_hint: String::new(),
                    handler: Arc::new(|_| Ok(CommandOutput::none())),
                })?;
                host.register_keybinding(crate::KeybindingSpec {
                    chords: vec![vec![0x11]],
                    chord_text: "ctrl+q".into(),
                    mode: None,
                    description: "detach".into(),
                    handler: Arc::new(|_| Vec::new()),
                })
            }))
            .build()
            .unwrap();
        let view = runtime.registry_view();
        assert_eq!(view.commands.len(), 1);
        assert_eq!(view.commands[0].name, "detach");
        assert_eq!(view.keybindings.len(), 1);
        assert_eq!(view.keybindings[0].chord_text, "ctrl+q");
    }

    #[test]
    fn empty_runtime_is_inert() {
        let runtime = AppRuntime::empty();
        assert!(
            runtime
                .dispatch(EventKind::Bell, EventPayload::Empty)
                .is_empty()
        );
        assert!(runtime.surfaces().is_empty());
        assert!(runtime.match_keybinding(&[0x11], None).is_none());
        assert!(matches!(
            runtime.invoke_command("help"),
            CommandDispatch::NotFound(_)
        ));
    }
}
