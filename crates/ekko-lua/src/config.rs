//! `init.lua` config loading: the Lua settings source — the *evaluator*
//! half of the config cascade. The *precedence policy* (init.lua supersedes
//! init.lua supersedes defaults) lives in `ekko-config::Config::load_cascade`;
//! this crate only implements [`ekko_config::LuaConfigEvaluator`] so the
//! config crate stays a dumb, dependency-free store.
//!
//! `~/.config/ekko/init.lua`, when present, is the only config file. It
//! evaluates — in a throwaway Lua state, under the hard-coded bootstrap
//! budget (config can raise the `[lua]` budgets scripts run under, but not
//! the budget it is itself read under) — to a table congruent with
//! [`ekko_config::Config`];
//! being Lua, users get conditionals and env dispatch for free, and ekko
//! only ever sees the returned table.
//!
//! A broken `init.lua` is a **hard error**, not a fall-through to TOML:
//! silently ignoring the user's config is worse than refusing to start.
//! Unknown top-level keys only warn — config files outlive binaries.

use std::path::Path;

use ekko_config::LuaConfigEvaluator;
use ekko_err::{Context, Result};
use mlua::{Lua, LuaSerdeExt, Table, Value};

use crate::{BOOTSTRAP_BUDGET, with_budget};

/// The stateless evaluator the config cascade calls. Lua state is created
/// per evaluation (config is read once at process start), so there's
/// nothing to share.
pub struct InitLuaEvaluator;

impl LuaConfigEvaluator for InitLuaEvaluator {
    fn eval_init_lua(&self, path: &Path) -> Result<ekko_config::Config> {
        load_config(path)
    }
}

/// Load config per the cascade both processes share: `init.lua` if present,
/// else defaults. Thin wrapper over
/// [`ekko_config::Config::load_cascade`] with this crate's evaluator
/// injected; kept for the existing call sites.
pub fn load_config_cascade() -> Result<ekko_config::Config> {
    ekko_config::Config::load_cascade(Some(&InitLuaEvaluator))
}

/// [`load_config_cascade`] against an explicit config directory — the seam
/// the precedence tests use.
pub fn load_config_cascade_in(dir: &Path) -> Result<ekko_config::Config> {
    ekko_config::Config::load_cascade_in(dir, Some(&InitLuaEvaluator))
}

/// Evaluate one `init.lua` into a [`ekko_config::Config`].
pub fn load_config(path: &Path) -> Result<ekko_config::Config> {
    let source =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let origin = path.display().to_string();
    config_from_source(&origin, &source).with_context(|| format!("loading config '{origin}'"))
}

fn config_from_source(origin: &str, source: &str) -> Result<ekko_config::Config> {
    let lua = Lua::new();
    let table: Table = with_budget(&lua, BOOTSTRAP_BUDGET, |lua| {
        lua.load(source).set_name(origin).eval()
    })
    .context("evaluating (must return a table)")?;

    const KNOWN: [&str; 5] = ["general", "ui", "keybinds", "extensions", "lua"];
    for pair in table.pairs::<Value, Value>() {
        let (key, _) = pair?;
        let name = match &key {
            Value::String(s) => s.to_string_lossy(),
            other => format!("<{}>", other.type_name()),
        };
        if !KNOWN.contains(&name.as_str()) {
            log::warn!("config '{origin}': ignoring unknown key '{name}'");
        }
    }

    let mut config: ekko_config::Config = lua
        .from_value(Value::Table(table))
        .context("converting the returned table")?;
    config.normalize();
    Ok(config)
}
