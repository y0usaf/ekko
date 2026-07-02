//! ekko-ext: the public extension API of the ekko terminal multiplexer.
//!
//! **The Rule** (see `DESIGN.md` at the repo root): if a feature can be an
//! extension, it must be an extension. Built-in features live in
//! `ekko-builtins` and register through this same API — there is no privileged
//! built-in path. This crate is pure registration/dispatch mechanism; it
//! depends only on `ekko-event` (the vocabulary), never on the client, server,
//! wire protocol, or renderer crates. The client implements [`DrawContext`]
//! as a thin adapter over its cell surface; a future `ekko-lua` bridge
//! implements the same traits for scripted extensions.

mod builder;
mod command;
mod draw;
mod host;
mod keybinding;
mod layout;
mod manifest;
mod mode;
mod overlay;
mod runtime;
mod snapshot;
mod surface;
mod traits;
mod visual;

pub use builder::RuntimeBuilder;
pub use command::{CommandHandler, CommandInfo, CommandInvocation, CommandOutput, CommandSpec};
pub use draw::{DrawContext, Rect, ScrollbarModel, ScrollbarStyle, TextStyle};
pub use keybinding::{
    KeybindingHandler, KeybindingInfo, KeybindingSpec, parse_key_binding, parse_key_chords,
    resolve_chords,
};
pub use layout::{ResolvedLayout, ResolvedRegion, resolve_layout};
pub use manifest::ExtensionManifest;
pub use mode::{ModeInitFn, ModeKeyFn, ModeOutcome, ModeRenderFn, ModeSpec, ModeState};
pub use overlay::{
    OVERLAY_HELP, OverlayInitFn, OverlayKeyFn, OverlayOutcome, OverlayPayload, OverlayPayloadFn,
    OverlayRenderFn, OverlaySpec, OverlayState, RegistryView,
};
pub use runtime::{AppRuntime, CommandDispatch};
pub use snapshot::{
    ClientSnapshot, NamerInput, ProjectGroup, SessionEntry, SessionGrouperSpec, SessionNameFn,
    SessionNamerSpec, SessionState, StatusNote,
};
pub use surface::{
    DockEdge, MouseKind, SurfaceDrawFn, SurfaceMouseEvent, SurfaceMouseFn, SurfaceSize,
    SurfaceSpec, SurfaceTickFn, SurfaceVisibleFn,
};
pub use traits::{Extension, ExtensionHost};
pub use visual::{Color, SpinnerSpec, ThemePalette, ThemeSpec, brighten, fade_toward};

// Re-export the event vocabulary so extensions only need one dependency.
pub use ekko_event::{
    EventHandler, EventHandlerRegistration, EventKind, EventPayload, EventReturn, KeyIntercept,
    LifecycleEvent, NoteKind, NoticeLevel, SessionExitReason, UiAction,
};
