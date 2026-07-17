//! ekko configuration: the settings schema shared by client and server.
//!
//! Loaded once at process start; a missing file yields `Config::default()`.
//! This crate parses only TOML (`config.toml`) — the `init.lua` settings
//! source that supersedes it lives in `ekko-lua`, which deserializes the
//! returned table into the same [`Config`], so this crate stays a dumb
//! store (its one dependency beyond serde is `ekko-proto`, for the
//! `PaneBorderStyle` vocabulary the wire shares). Keybind values stay as
//! raw strings here — chord parsing lives in the client's input layer,
//! which owns the key vocabulary.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use ekko_proto::PaneBorderStyle;
use serde::{Deserialize, Serialize};

pub const SIDEBAR_WIDTH_DEFAULT: u16 = 36;
pub const SIDEBAR_WIDTH_MIN: u16 = 8;
pub const SIDEBAR_WIDTH_MAX: u16 = 120;
pub const SCROLLBACK_LINES_DEFAULT: usize = 10_000;
pub const LUA_DRAW_BUDGET_DEFAULT: u32 = 200_000;
pub const LUA_HANDLER_BUDGET_DEFAULT: u32 = 2_000_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub ui: Ui,
    /// Action name -> binding text(s), e.g. `detach = "ctrl+q"`.
    pub keybinds: BTreeMap<String, Keybind>,
    pub extensions: Extensions,
    pub lua: LuaLimits,
}

/// Extension loading controls. Manifest ids listed in `disabled` are skipped
/// at runtime build (e.g. `disabled = ["ekko-builtins.sidebar"]`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Extensions {
    pub disabled: Vec<String>,
}

/// Instruction budgets for the Lua bridge (`ekko-lua`). Draw-path callbacks
/// run on the client's frame pass and get the tight budget; handler
/// callbacks (commands, keybindings, events) get the loose one, since the
/// host already bounds them with wall-clock timeouts. Bootstrap exception:
/// `init.lua` itself and a script's top-level chunk are evaluated before any
/// config applies, so they always run under the hard-coded defaults.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LuaLimits {
    pub draw_budget: u32,
    pub handler_budget: u32,
}

impl Default for LuaLimits {
    fn default() -> Self {
        Self {
            draw_budget: LUA_DRAW_BUDGET_DEFAULT,
            handler_budget: LUA_HANDLER_BUDGET_DEFAULT,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Shell to spawn in new sessions; empty means `$SHELL` then `/bin/sh`.
    pub default_shell: String,
    pub scrollback_lines: usize,
}

impl Default for General {
    fn default() -> Self {
        Self {
            default_shell: String::new(),
            scrollback_lines: SCROLLBACK_LINES_DEFAULT,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    pub sidebar_width: u16,
    /// Pane separator style: `"none"` (default, edge-to-edge),
    /// `"compact"` (shared zellij-style boundary lines), or `"frame"`
    /// (a full box frame around every pane). Owned by the daemon — it
    /// reserves the separator cells in the canvas layout — and announced
    /// to clients over the wire, so set it where the server reads config.
    pub pane_borders: PaneBorderStyle,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            sidebar_width: SIDEBAR_WIDTH_DEFAULT,
            pane_borders: PaneBorderStyle::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Keybind {
    Single(String),
    Multiple(Vec<String>),
}

impl Keybind {
    pub fn binding_strings(&self) -> Vec<String> {
        match self {
            Self::Single(text) => {
                let text = text.trim();
                if text.is_empty() {
                    vec![]
                } else {
                    vec![text.to_string()]
                }
            }
            Self::Multiple(bindings) => bindings
                .iter()
                .map(|b| b.trim())
                .filter(|b| !b.is_empty())
                .map(str::to_string)
                .collect(),
        }
    }
}

impl Config {
    pub fn load_default() -> anyhow::Result<Self> {
        Self::load_from(&default_config_path())
    }

    pub fn load_from(path: &PathBuf) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut config: Self =
            toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
        config.normalize();
        Ok(config)
    }

    /// Sidebar width clamped to the valid range.
    pub fn sidebar_width(&self) -> u16 {
        self.ui
            .sidebar_width
            .clamp(SIDEBAR_WIDTH_MIN, SIDEBAR_WIDTH_MAX)
    }

    /// Resolve the shell for new sessions: config, then `$SHELL`, then `/bin/sh`.
    pub fn resolve_shell(&self) -> PathBuf {
        let configured = self.general.default_shell.trim();
        if !configured.is_empty() {
            return PathBuf::from(configured);
        }
        std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/sh"))
    }

    /// Binding strings for an action, or the given defaults when unset/empty.
    pub fn bindings_for(&self, action: &str, defaults: &[&str]) -> Vec<String> {
        if let Some(bind) = self.keybinds.get(action) {
            let overrides = bind.binding_strings();
            if !overrides.is_empty() {
                return overrides;
            }
        }
        defaults.iter().map(|s| s.to_string()).collect()
    }

    /// Repair nonsense values after deserializing (from TOML here, or from
    /// an `init.lua` table in `ekko-lua`).
    pub fn normalize(&mut self) {
        if self.general.scrollback_lines == 0 {
            self.general.scrollback_lines = SCROLLBACK_LINES_DEFAULT;
        }
        // A zero budget would abort every callback on its first instruction.
        if self.lua.draw_budget == 0 {
            self.lua.draw_budget = LUA_DRAW_BUDGET_DEFAULT;
        }
        if self.lua.handler_budget == 0 {
            self.lua.handler_budget = LUA_HANDLER_BUDGET_DEFAULT;
        }
    }
}

pub fn config_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("ekko"))
        .unwrap_or_else(|| PathBuf::from(".config/ekko"))
}

pub fn default_config_path() -> PathBuf {
    config_dir().join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let config = Config::load_from(&PathBuf::from("/nonexistent/ekko-config.toml")).unwrap();
        assert_eq!(config.sidebar_width(), SIDEBAR_WIDTH_DEFAULT);
        assert_eq!(config.general.scrollback_lines, SCROLLBACK_LINES_DEFAULT);
    }

    #[test]
    fn parses_full_config() {
        let config: Config = toml::from_str(
            r#"
            [general]
            default_shell = "/bin/zsh"
            scrollback_lines = 500

            [ui]
            sidebar_width = 28

            [keybinds]
            detach = "ctrl+q"
            session_next = ["ctrl+j", "ctrl+down"]
            "#,
        )
        .unwrap();
        assert_eq!(config.general.default_shell, "/bin/zsh");
        assert_eq!(config.sidebar_width(), 28);
        assert_eq!(
            config.bindings_for("detach", &["ctrl+d"]),
            vec!["ctrl+q".to_string()]
        );
        assert_eq!(
            config.bindings_for("session_next", &[]),
            vec!["ctrl+j".to_string(), "ctrl+down".to_string()]
        );
        assert_eq!(
            config.bindings_for("session_prev", &["ctrl+k"]),
            vec!["ctrl+k".to_string()]
        );
    }

    #[test]
    fn lua_budgets_parse_and_zero_is_normalized() {
        let mut config: Config =
            toml::from_str("[lua]\ndraw_budget = 500000\nhandler_budget = 0\n").unwrap();
        config.normalize();
        assert_eq!(config.lua.draw_budget, 500_000);
        assert_eq!(config.lua.handler_budget, LUA_HANDLER_BUDGET_DEFAULT);
        assert_eq!(Config::default().lua.draw_budget, LUA_DRAW_BUDGET_DEFAULT);
    }

    #[test]
    fn sidebar_width_clamped() {
        let config: Config = toml::from_str("[ui]\nsidebar_width = 2\n").unwrap();
        assert_eq!(config.sidebar_width(), SIDEBAR_WIDTH_MIN);
        let config: Config = toml::from_str("[ui]\nsidebar_width = 500\n").unwrap();
        assert_eq!(config.sidebar_width(), SIDEBAR_WIDTH_MAX);
    }
}
