//! `init.lua` config loading: the Lua settings source.
//!
//! `~/.config/ekko/init.lua`, when present, supersedes `config.toml`. It
//! evaluates — in a throwaway Lua state, under the hard-coded bootstrap
//! budget (config can raise the `[lua]` budgets scripts run under, but not
//! the budget it is itself read under) — to a table congruent with
//! [`ekko_config::Config`];
//! being Lua, users get conditionals and env dispatch for free, and ekko
//! only ever sees the returned table. Evaluation lives here rather than in
//! `ekko-config` so the config crate stays a dumb, dependency-free store.
//!
//! A broken `init.lua` is a **hard error**, not a fall-through to TOML:
//! silently ignoring the user's config is worse than refusing to start.
//! Unknown top-level keys only warn — config files outlive binaries.

use std::path::Path;

use anyhow::{Context, Result};
use mlua::{Lua, LuaSerdeExt, Table, Value};

use crate::{BOOTSTRAP_BUDGET, with_budget};

/// Load config per the cascade both processes share: `init.lua` if present,
/// else `config.toml`, else defaults. Only the `init.lua` arm is a hard
/// error; a broken `config.toml` degrades to defaults with a warning,
/// exactly as it did before `init.lua` existed.
pub fn load_config_cascade() -> Result<ekko_config::Config> {
    load_config_cascade_in(&ekko_config::config_dir())
}

/// [`load_config_cascade`] against an explicit config directory — the seam
/// the precedence tests use.
pub fn load_config_cascade_in(dir: &Path) -> Result<ekko_config::Config> {
    let init = dir.join("init.lua");
    if init.exists() {
        return load_config(&init);
    }
    Ok(
        ekko_config::Config::load_from(&dir.join("config.toml")).unwrap_or_else(|err| {
            log::warn!("falling back to default config: {err:#}");
            ekko_config::Config::default()
        }),
    )
}

/// Evaluate one `init.lua` into a [`ekko_config::Config`].
pub fn load_config(path: &Path) -> Result<ekko_config::Config> {
    let source =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let origin = path.display().to_string();
    config_from_source(&origin, &source).with_context(|| {
        format!("loading config '{origin}' (config.toml is ignored while it exists)")
    })
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
