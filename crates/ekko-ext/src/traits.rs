use anyhow::Result;
use ekko_event::EventHandlerRegistration;

use crate::{
    CommandSpec, ExtensionManifest, KeybindingSpec, ModeSpec, OverlaySpec, SessionGrouperSpec,
    SessionNamerSpec, SpinnerSpec, SurfaceSpec, ThemeSpec,
};

pub trait Extension: Send + Sync {
    fn manifest(&self) -> ExtensionManifest;
    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()>;
}

/// The registration surface available to extensions during `register()`.
///
/// Each method lets an extension add a capability to the runtime. Duplicate
/// names are hard errors — built-ins register first, so a user extension
/// that reuses a built-in name fails loudly at build time; there is no
/// privileged bypass path.
pub trait ExtensionHost {
    /// Register a `:command` (with optional aliases).
    fn register_command(&mut self, command: CommandSpec) -> Result<()>;

    /// Register a raw-byte keybinding.
    fn register_keybinding(&mut self, binding: KeybindingSpec) -> Result<()>;

    /// Register an input mode (e.g. command mode).
    fn register_mode(&mut self, mode: ModeSpec) -> Result<()>;

    /// Register a docked chrome surface (e.g. sidebar, statusbar).
    fn register_surface(&mut self, surface: SurfaceSpec) -> Result<()>;

    /// Register a modal overlay.
    fn register_overlay(&mut self, overlay: OverlaySpec) -> Result<()>;

    /// Register a named color theme.
    fn register_theme(&mut self, theme: ThemeSpec) -> Result<()>;

    /// Register a named spinner animation.
    fn register_spinner(&mut self, spinner: SpinnerSpec) -> Result<()>;

    /// Register the session-list grouping policy.
    fn register_session_grouper(&mut self, grouper: SessionGrouperSpec) -> Result<()>;

    /// Register the session-naming policy (names for sessions created
    /// without an explicit name).
    fn register_session_namer(&mut self, namer: SessionNamerSpec) -> Result<()>;

    /// Subscribe to a lifecycle event.
    fn subscribe(&mut self, handler: EventHandlerRegistration) -> Result<()>;
}
