//! Input mode registration. A mode intercepts raw input while active (e.g.
//! command mode's line editor). The host provides the mechanism — routing
//! bytes to the active mode, storing its state, drawing its render layer —
//! while which modes exist and what they do is extension policy.

use std::any::Any;
use std::sync::Arc;

use ekko_event::UiAction;

use crate::draw::DrawContext;
use crate::snapshot::ClientSnapshot;

/// Type-erased per-activation mode state, created by `init_state` when the
/// mode is entered and dropped when it exits.
pub type ModeState = Box<dyn Any + Send + Sync>;

pub type ModeInitFn = Arc<dyn Fn() -> ModeState + Send + Sync>;
pub type ModeKeyFn =
    Arc<dyn Fn(&mut ModeState, &[u8], &ClientSnapshot) -> ModeOutcome + Send + Sync>;
/// Draws the mode's chrome (e.g. the `:command` line) into a full-frame
/// context. Returns the hardware cursor position, if the mode wants one.
pub type ModeRenderFn = Arc<
    dyn Fn(&mut dyn DrawContext, &ModeState, &ClientSnapshot) -> Option<(i32, i32)> + Send + Sync,
>;

#[derive(Debug, PartialEq, Eq)]
pub enum ModeOutcome {
    /// Stay in the mode.
    Continue,
    /// Stay in the mode and apply the returned actions (e.g. scroll mode
    /// emitting a scroll per keypress).
    ContinueWith(Vec<UiAction>),
    /// Leave the mode without further effect.
    Exit,
    /// Leave the mode and apply the returned actions.
    ExitWith(Vec<UiAction>),
}

#[derive(Clone)]
pub struct ModeSpec {
    pub name: String,
    pub init_state: ModeInitFn,
    pub on_key: ModeKeyFn,
    pub render: Option<ModeRenderFn>,
}
