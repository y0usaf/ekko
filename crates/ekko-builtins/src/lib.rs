//! ekko-builtins: every stock ekko feature, implemented through the public
//! `ekko-ext` API. Per The Rule (see `DESIGN.md`): if a feature can be an
//! extension, it must be an extension — this crate is the extension API's
//! test suite. Deleting it from a build must leave a bare-but-functional
//! harness (attach, raw key passthrough, full-screen grid).
//!
//! This crate never depends on `ekko-client`, `ekko-server`, or `ekko-proto`:
//! builtins only see what any extension sees.

pub mod bell;
pub mod command_mode;
pub mod env_file;
pub mod grouping;
pub mod help;
pub mod keybindings;
pub mod leader;
pub mod naming;
pub mod resurrection;
pub mod rows;
pub mod scroll_mode;
pub mod sidebar;
pub mod statusbar;
pub mod theme;

use ekko_config::Config;
use ekko_ext::Extension;
use ekko_tui::TerminalColors;

/// The stock client-side extension set, in registration order. Built-ins
/// register first so a user extension reusing a name fails loudly.
///
/// `terminal_colors` is the host's OSC probe result; `None` (no answer)
/// falls back to the standard ANSI palette.
pub fn client_extensions(
    config: &Config,
    terminal_colors: Option<TerminalColors>,
) -> Vec<Box<dyn Extension>> {
    vec![
        Box::new(theme::ThemeExtension::new(terminal_colors)),
        Box::new(sidebar::SidebarExtension::new(config.sidebar_width())),
        Box::new(statusbar::StatusbarExtension),
        Box::new(command_mode::CommandModeExtension),
        Box::new(scroll_mode::ScrollModeExtension),
        Box::new(keybindings::KeybindingsExtension::new(config)),
        Box::new(leader::LeaderExtension::new(config)),
        Box::new(grouping::GroupingExtension),
        Box::new(naming::NamingExtension),
        Box::new(help::HelpOverlayExtension),
    ]
}

/// The stock server-side (daemon) extension set.
pub fn server_extensions() -> Vec<Box<dyn Extension>> {
    vec![
        Box::new(resurrection::ResurrectionExtension),
        Box::new(bell::BellThrottleExtension::default()),
        Box::new(env_file::EnvFileExtension),
    ]
}
