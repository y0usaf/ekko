//! ekko configuration: the settings schema shared by client and server.
//!
//! Loaded once at process start; a missing file yields `Config::default()`.
//! The `init.lua` settings source lives in `ekko-lua`, which deserializes the
//! returned table into the same [`Config`], so this crate stays a dumb
//! store (its one dependency beyond serde is `ekko-proto`, for the
//! `PaneBorderStyle` vocabulary the wire shares). Keybind values stay as
//! raw strings here — chord parsing lives in the client's input layer,
//! which owns the key vocabulary.

use ekko_proto::PaneBorderStyle;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SIDEBAR_WIDTH_DEFAULT: u16 = 36;
pub const SIDEBAR_WIDTH_MIN: u16 = 8;
pub const SIDEBAR_WIDTH_MAX: u16 = 120;
pub const ANIMATION_INTERVAL_MS_DEFAULT: u16 = 80;
pub const ANIMATION_INTERVAL_MS_MIN: u16 = 8;
pub const ANIMATION_INTERVAL_MS_MAX: u16 = 1000;
pub const SCROLLBACK_LINES_DEFAULT: usize = 10_000;
pub const LUA_DRAW_BUDGET_DEFAULT: u32 = 200_000;
pub const LUA_HANDLER_BUDGET_DEFAULT: u32 = 2_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneLayout {
    #[default]
    Manual,
    Equal,
}

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
    /// Animation tick cadence for client-side surfaces, in milliseconds.
    pub animation_interval_ms: u16,
    /// Pane separator style: `"none"` (default, edge-to-edge),
    /// `"compact"` (shared zellij-style boundary lines), or `"frame"`
    /// (a full box frame around every pane). Owned by the daemon — it
    /// reserves the separator cells in the canvas layout — and announced
    /// to clients over the wire, so set it where the server reads config.
    pub pane_borders: PaneBorderStyle,
    /// Pane sizing policy; manual preserves the canonical BSP behavior.
    pub pane_layout: PaneLayout,
    /// Optional ASCII (or custom) glyphs for pane separators. `None` keeps
    /// the box-drawing glyphs. Client-side only — the daemon reserves the
    /// separator cells but never sees these glyphs.
    pub border_glyphs: Option<BorderGlyphs>,
}

/// One glyph per separator shape, replacing the box-drawing table when
/// `Ui::border_glyphs` is set. Each field is a single character from a
/// one-character Lua string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorderGlyphs {
    pub horizontal: char,
    pub vertical: char,
    pub junction: char,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            sidebar_width: SIDEBAR_WIDTH_DEFAULT,
            animation_interval_ms: ANIMATION_INTERVAL_MS_DEFAULT,
            pane_borders: PaneBorderStyle::None,
            pane_layout: PaneLayout::Manual,
            border_glyphs: None,
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
    /// The config cascade, owned here so every process resolves config the
    /// same way: `init.lua` in `dir` supersedes defaults.
    ///
    /// Lua evaluation is injected via [`LuaConfigEvaluator`] (implemented by
    /// `ekko-lua`) so this crate stays a dumb, dependency-free store while
    /// still owning the *precedence policy*:
    /// - `init.lua` present → evaluate it. A broken one is a **hard error**,
    ///   never a silent fall-through: refusing to start beats ignoring the
    ///   user's config.
    /// - else a stale `config.toml` is a hard error directing migration;
    /// - neither → defaults.
    ///
    /// With `lua = None` (a build without the bridge), defaults apply unless
    /// a stale `config.toml` is present.
    pub fn load_cascade_in(
        dir: &Path,
        lua: Option<&dyn LuaConfigEvaluator>,
    ) -> anyhow::Result<Self> {
        let init = dir.join("init.lua");
        if init.exists()
            && let Some(evaluator) = lua
        {
            return evaluator.eval_init_lua(&init);
        }
        let toml = dir.join("config.toml");
        if toml.exists() {
            anyhow::bail!(
                "unsupported config file {}; migrate to init.lua",
                toml.display()
            );
        }
        Ok(Self::default())
    }

    /// [`Self::load_cascade_in`] against the platform config directory —
    /// the single entry point both processes (client and daemon) call.
    pub fn load_cascade(lua: Option<&dyn LuaConfigEvaluator>) -> anyhow::Result<Self> {
        Self::load_cascade_in(&config_dir(), lua)
    }

    /// Sidebar width clamped to the valid range.
    pub fn sidebar_width(&self) -> u16 {
        self.ui
            .sidebar_width
            .clamp(SIDEBAR_WIDTH_MIN, SIDEBAR_WIDTH_MAX)
    }

    /// Animation cadence clamped to a sane range for terminal rendering.
    pub fn animation_interval_ms(&self) -> u16 {
        self.ui
            .animation_interval_ms
            .clamp(ANIMATION_INTERVAL_MS_MIN, ANIMATION_INTERVAL_MS_MAX)
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

    /// Repair nonsense values after deserializing an `init.lua` table in
    /// `ekko-lua`.
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

/// The Lua `init.lua` evaluator, injected into the config cascade by the
/// process's runtime builder. Implemented by `ekko-lua` (the only crate
/// that may depend on a Lua VM); `ekko-config` depends only on this trait,
/// keeping the crate graph acyclic: `ekko-lua → ekko-config`, never back.
pub trait LuaConfigEvaluator {
    /// Evaluate `init.lua` at `path` into a normalized [`Config`]. A broken
    /// script must be an error, not a fall-through.
    fn eval_init_lua(&self, path: &Path) -> anyhow::Result<Config>;
}

/// Config directory, resolved by the workspace's single resolver
/// (`ekko-paths`): `$XDG_CONFIG_HOME/ekko` (or `~/.config/ekko`).
pub fn config_dir() -> PathBuf {
    ekko_paths::config_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_expected_values() {
        let config = Config::default();
        assert_eq!(config.sidebar_width(), SIDEBAR_WIDTH_DEFAULT);
        assert_eq!(
            config.animation_interval_ms(),
            ANIMATION_INTERVAL_MS_DEFAULT
        );
        assert_eq!(config.general.scrollback_lines, SCROLLBACK_LINES_DEFAULT);
        assert!(Ui::default().border_glyphs.is_none());
    }

    #[test]
    fn config_values_work() {
        let config = Config {
            general: General {
                default_shell: "/bin/zsh".into(),
                scrollback_lines: 500,
            },
            ui: Ui {
                sidebar_width: 28,
                animation_interval_ms: 33,
                ..Default::default()
            },
            keybinds: BTreeMap::from([
                ("detach".into(), Keybind::Single("ctrl+q".into())),
                (
                    "session_next".into(),
                    Keybind::Multiple(vec!["ctrl+j".into(), "ctrl+down".into()]),
                ),
            ]),
            ..Default::default()
        };
        assert_eq!(config.general.default_shell, "/bin/zsh");
        assert_eq!(config.sidebar_width(), 28);
        assert_eq!(config.animation_interval_ms(), 33);
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
        let mut config = Config {
            lua: LuaLimits {
                draw_budget: 500_000,
                handler_budget: 0,
            },
            ..Default::default()
        };
        config.normalize();
        assert_eq!(config.lua.draw_budget, 500_000);
        assert_eq!(config.lua.handler_budget, LUA_HANDLER_BUDGET_DEFAULT);
        assert_eq!(Config::default().lua.draw_budget, LUA_DRAW_BUDGET_DEFAULT);
    }

    #[test]
    fn animation_interval_clamped() {
        let config = Config {
            ui: Ui {
                animation_interval_ms: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.animation_interval_ms(), ANIMATION_INTERVAL_MS_MIN);
        let config = Config {
            ui: Ui {
                animation_interval_ms: 5000,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.animation_interval_ms(), ANIMATION_INTERVAL_MS_MAX);
    }

    #[test]
    fn sidebar_width_clamped() {
        let config = Config {
            ui: Ui {
                sidebar_width: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.sidebar_width(), SIDEBAR_WIDTH_MIN);
        let config = Config {
            ui: Ui {
                sidebar_width: 500,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.sidebar_width(), SIDEBAR_WIDTH_MAX);
    }
}
