//! Overlay registration: named modal panels drawn on top of the normal
//! chrome. The host stores the overlay's type-erased state while it is
//! open; the extension provides init/render/key closures.

use std::any::Any;
use std::sync::Arc;

use ekko_event::UiAction;

use crate::command::CommandInfo;
use crate::draw::DrawContext;
use crate::keybinding::KeybindingInfo;
use crate::snapshot::ClientSnapshot;

/// Type-erased overlay state, created by `init_state` when the overlay
/// opens and threaded through `render`/`handle_key` while it is open.
pub type OverlayState = Box<dyn Any + Send + Sync>;

/// Type-erased payload built by the overlay's own [`OverlaySpec::build_payload`]
/// when the overlay opens; overlays without a builder get `None`.
pub type OverlayPayload = Box<dyn Any + Send + Sync>;

pub type OverlayInitFn = Arc<dyn Fn(Option<OverlayPayload>) -> OverlayState + Send + Sync>;
pub type OverlayRenderFn =
    Arc<dyn Fn(&mut dyn DrawContext, &mut OverlayState, &ClientSnapshot) + Send + Sync>;
pub type OverlayKeyFn = Arc<dyn Fn(&mut OverlayState, &[u8]) -> OverlayOutcome + Send + Sync>;
pub type OverlayPayloadFn = Arc<dyn Fn(&RegistryView) -> OverlayPayload + Send + Sync>;

/// Conventional name of the help overlay. Extensions open it via
/// `UiAction::OpenOverlay`; the host attaches no meaning to the name, so a
/// replacement extension registering under it inherits every caller.
pub const OVERLAY_HELP: &str = "ekko:help";

/// Read-only listing of the live registries, handed to
/// [`OverlaySpec::build_payload`] at open time so overlays can present
/// registry data (e.g. a help reference) that never drifts from what is
/// actually registered — without the host special-casing any overlay name.
#[derive(Clone, Debug)]
pub struct RegistryView {
    pub keybindings: Vec<KeybindingInfo>,
    pub commands: Vec<CommandInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlayOutcome {
    /// The key was consumed; the overlay stays open.
    None,
    /// Close the overlay.
    Close,
    /// Close the overlay and apply the returned actions (e.g. switch session).
    CloseWith(Vec<UiAction>),
}

#[derive(Clone)]
pub struct OverlaySpec {
    pub name: String,
    pub description: String,
    pub init_state: OverlayInitFn,
    pub render: OverlayRenderFn,
    pub handle_key: OverlayKeyFn,
    /// Optional payload builder the host invokes with the live
    /// [`RegistryView`] every time the overlay opens; the result is passed
    /// to `init_state`.
    pub build_payload: Option<OverlayPayloadFn>,
    /// Mode-attached overlay: when set, the host opens this overlay on
    /// entering the named mode and closes it when that mode exits. While
    /// attached, the overlay is render-only — input keeps flowing to the
    /// mode (its `handle_key` is never called), so mode chrome like the
    /// which-key panel stays fully interactive alongside it.
    pub attach_mode: Option<String>,
}
