//! Command registration: `:name args` lines typed in command mode resolve
//! through this registry. Built-in commands register through this same spec
//! from `ekko-builtins` — there is no separate built-in command path.

use std::sync::Arc;

use anyhow::Result;
use ekko_event::{NoteKind, UiAction};

#[derive(Clone)]
pub struct CommandSpec {
    pub name: String,
    /// Alternate names resolving to the same command (e.g. `q`, `quit` for
    /// `detach`). Aliases share the global command namespace.
    pub aliases: Vec<String>,
    pub description: String,
    /// Usage hint rendered after the name in help, e.g. `"[name]"`.
    pub args_hint: String,
    pub handler: CommandHandler,
}

pub type CommandHandler = Arc<dyn Fn(CommandInvocation) -> Result<CommandOutput> + Send + Sync>;

#[derive(Clone, Debug)]
pub struct CommandInvocation {
    /// Everything after the command name, trimmed.
    pub raw_args: String,
}

/// Listing entry for help output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    pub name: String,
    pub aliases: Vec<String>,
    pub args_hint: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub actions: Vec<UiAction>,
}

impl CommandOutput {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn action(action: UiAction) -> Self {
        Self {
            actions: vec![action],
        }
    }

    pub fn actions(actions: Vec<UiAction>) -> Self {
        Self { actions }
    }

    pub fn note(text: impl Into<String>, kind: NoteKind) -> Self {
        Self::action(UiAction::SetStatusNote {
            text: text.into(),
            kind,
            ttl_ms: 4_000,
        })
    }
}

/// Resolves a `UiAction::Custom { name, payload }` into ordinary actions.
/// Registered by extensions through
/// [`ExtensionHost::register_action_interpreter`](crate::ExtensionHost::register_action_interpreter);
/// the host chains the returned actions like any others (bounded by the
/// same recursion limit). Errors surface as status notes, never panics.
pub type ActionInterpreterFn = Arc<dyn Fn(&str) -> anyhow::Result<Vec<UiAction>> + Send + Sync>;

/// A named interpreter for `UiAction::Custom`. `name` is matched exactly;
/// duplicate names are hard errors at runtime build, like every registry.
pub struct ActionInterpreterSpec {
    pub name: String,
    /// Shown in the help overlay's command listing context and logs.
    pub description: String,
    pub handler: ActionInterpreterFn,
}
