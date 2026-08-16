//! Spatiotemporal composability surface for the client.
//!
//! The kernel crate supplies the mechanics ([`kernel::Context`],
//! [`kernel::Component`], [`kernel::Effect`], [`kernel::Inverse`]); this module
//! turns the client's lifecycle into those terms so it can be exercised on the
//! two axes:
//!
//! - **Effect scope** — a session mounts the resources it owns as
//!   [`kernel::Component`] effects: the scene composition, the daemon spawn,
//!   the clipboard watcher, and the event-loop subscription. Each effect
//!   commits its registration into the [`Context`] and returns an inverse that
//!   removes it, so [`Clientspace::unmount_session`] replays the inverses in
//!   reverse order and the context returns to its pre-session state with no
//!   residue.
//! - **Reactive dependency graph** — the render path is a [`kernel::Component`]
//!   that declares the state keys it reads. A committed `set` on a declared
//!   key notifies only those readers; an undeclared key changes nothing.
//!   Replacing "repaint everything" with declared-reader notification plans a
//!   repaint ([`Clientspace::take_dirty`]) rather than eagerly redrawing.
//! - **Single write path** — visible-state mutation goes through
//!   [`Clientspace::set_hidden_surfaces`], never ad-hoc on `&mut` host data.
//!
//! ```text
//!   mount ─► [scene] [spawn] [clipboard] [subscription]  (effects, in order)
//!                   └─ each returns an Inverse
//!   unmount (reverse) ◄─┘
//! ```

use kernel::{Component, Context};
use std::any::Any;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Declared reactive keys the render path reads. A committed `set` (or
/// `remove`) on one of these notifies exactly the components that declared
/// it — the kernel's replacement for "repaint everything".
pub const SESSION_KEY: &str = "client.session";
pub const SCENE_KEY: &str = "client.scene";
pub const SPAWN_KEY: &str = "client.spawn";
pub const CLIPBOARD_KEY: &str = "client.clipboard";
pub const SUBSCRIPTION_KEY: &str = "client.event_loop";
pub const HIDDEN_SURFACES_KEY: &str = "client.hidden_surfaces";

/// The identity of the session whose lifecycle a component scopes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionId {
    pub name: String,
    /// Connection generation; socket events from other generations are stale.
    pub generation: u64,
}

/// The client host shell: owns the [`Context`], mounts per-session lifecycle
/// components, and routes visible-state writes through the single write path.
pub struct Clientspace {
    ctx: Context,
    /// Handle of the mounted per-session component, if any.
    session_scope: Option<usize>,
    /// The render reader sets this when a declared key changes; the event loop
    /// consumes it as a "please repaint" signal.
    dirty: Arc<AtomicBool>,
}

impl Clientspace {
    pub fn new() -> Self {
        let mut this = Self {
            ctx: Context::new(),
            session_scope: None,
            dirty: Arc::new(AtomicBool::new(true)),
        };
        this.mount_render_reader();
        this
    }

    /// Mount the per-session lifecycle component. Its effects (session, scene,
    /// spawn, clipboard watcher, event-loop subscription) each commit state and
    /// return an inverse; `unmount_session` replays them in reverse order.
    pub fn mount_session(&mut self, session: SessionId) -> usize {
        let id = self.ctx.mount(session_component(session));
        self.session_scope = Some(id);
        id
    }

    /// Unmount the session: replay every inverse in reverse order so the
    /// context returns to its pre-mount state except for the persistent render
    /// reader.
    pub fn unmount_session(&mut self) {
        if let Some(id) = self.session_scope.take() {
            self.ctx.unmount(id);
        }
    }

    /// The single committed write path for visible state. A `set` here
    /// notifies the render reader, which flags a repaint.
    pub fn set_hidden_surfaces(&mut self, hidden: HashSet<String>) {
        self.ctx.set(HIDDEN_SURFACES_KEY, hidden);
    }

    /// Read back the current hidden-surface set (never `&mut` host data).
    pub fn hidden_surfaces(&self) -> HashSet<String> {
        self.ctx
            .get(HIDDEN_SURFACES_KEY)
            .cloned()
            .unwrap_or_default()
    }

    /// Consume and clear the repaint flag set by the render reader when a
    /// declared key changed.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
    }

    fn mount_render_reader(&mut self) {
        let dirty = Arc::clone(&self.dirty);
        let _ = self.ctx.mount(
            Component::new(vec![
                SESSION_KEY,
                SCENE_KEY,
                SPAWN_KEY,
                CLIPBOARD_KEY,
                SUBSCRIPTION_KEY,
                HIDDEN_SURFACES_KEY,
            ])
            .on_change(move |_, _| dirty.store(true, Ordering::Relaxed)),
        );
    }
}

/// Register `value` under `key`, returning an inverse that removes it.
fn register(ctx: &mut Context, key: &'static str, value: impl Any + Send) -> kernel::Inverse {
    let _had = ctx.has(key);
    ctx.set(key, value);
    Box::new(move |ctx| ctx.remove(key))
}

/// The per-session lifecycle component: scene, spawn, clipboard watcher, and
/// event-loop subscription each an effect with an inverse, so an unmount runs
/// exactly the inverse order. It declares no reads — a component that *writes*
/// keys isn't also a reader of them; the render reader (mounted in
/// [`Clientspace::new`]) is what reacts to those writes.
pub fn session_component(session: SessionId) -> Component {
    Component::new(vec![])
        .effect({
            let session = session.clone();
            move |ctx| {
                ctx.set(SESSION_KEY, session.clone());
                Box::new(move |ctx| ctx.remove(SESSION_KEY))
            }
        })
        .effect({
            let scene = "default".to_string();
            move |ctx| {
                ctx.set(SCENE_KEY, scene.clone());
                Box::new(move |ctx| ctx.remove(SCENE_KEY))
            }
        })
        .effect(move |ctx| register(ctx, SPAWN_KEY, true))
        .effect(move |ctx| register(ctx, CLIPBOARD_KEY, true))
        .effect(move |ctx| register(ctx, SUBSCRIPTION_KEY, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// A component that only declares reads and mounts an `on_change` (the
    /// render reader) commits no state, so it leaves `ctx.len()` at 0.
    const BASELINE: usize = 0;

    #[test]
    fn unmount_session_reverts_every_effect_with_no_residue() {
        let mut cs = Clientspace::new();
        assert_eq!(cs.ctx.len(), BASELINE, "render reader adds no state keys");

        cs.mount_session(SessionId {
            name: "demo".to_string(),
            generation: 2,
        });

        // Every effect is live: each session resource is registered.
        assert!(cs.ctx.has(SESSION_KEY));
        assert!(cs.ctx.has(SCENE_KEY));
        assert!(cs.ctx.has(SPAWN_KEY));
        assert!(cs.ctx.has(CLIPBOARD_KEY));
        assert!(cs.ctx.has(SUBSCRIPTION_KEY));
        assert_eq!(cs.ctx.len(), BASELINE + 5);

        cs.unmount_session();

        assert_eq!(cs.ctx.len(), BASELINE, "residue after unmount");
        assert!(!cs.ctx.has(SESSION_KEY));
        assert!(!cs.ctx.has(SCENE_KEY));
        assert!(!cs.ctx.has(SPAWN_KEY));
        assert!(!cs.ctx.has(CLIPBOARD_KEY));
        assert!(!cs.ctx.has(SUBSCRIPTION_KEY));
    }

    #[test]
    fn declared_keys_notify_only_their_readers() {
        let mut cs = Clientspace::new();
        cs.mount_session(SessionId {
            name: "s".to_string(),
            generation: 1,
        });
        let _ = cs.take_dirty(); // construction + mount both set it; clear.

        // An extra reader that declares only SCENE_KEY, so spatial reach can be
        // checked against the render reader (which declares many keys).
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        let _ = cs
            .ctx
            .mount(Component::new(vec![SCENE_KEY]).on_change(move |_, k| {
                assert_eq!(k, SCENE_KEY, "scene reader got a non-scene key");
                h.fetch_add(1, Ordering::Relaxed);
            }));

        // Change a declared key only the render reader reads (hidden surfaces):
        // render reader fires, scene reader does not.
        cs.set_hidden_surfaces(HashSet::from(["sidebar".into()]));
        assert!(cs.take_dirty(), "render reader fired on declared key");
        assert_eq!(hits.load(Ordering::Relaxed), 0, "scene-only reader fired");

        // Change SCENE_KEY: both the render reader and the scene-only reader
        // declared it, so both react.
        cs.ctx.set(SCENE_KEY, "diff".to_string());
        assert!(cs.take_dirty(), "render reader fired on scene key");
        assert_eq!(hits.load(Ordering::Relaxed), 1, "scene reader fired");

        // Change an undeclared key: nobody reacts.
        cs.ctx.set("plugin.some_key", 7u32);
        cs.ctx.set("plugin.other_key", "x".to_string());
        assert!(!cs.take_dirty(), "undeclared key reached the render reader");
        assert_eq!(
            hits.load(Ordering::Relaxed),
            1,
            "undeclared key reached the scene reader"
        );
    }
}
