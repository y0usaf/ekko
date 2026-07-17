//! Value conversions across the Lua boundary: snapshots and event payloads
//! go in as plain tables; actions and event returns come back as strings /
//! small tables. Everything is data — no host references ever cross into
//! Lua, mirroring the snapshot-reads/action-writes discipline.

use anyhow::{Result, anyhow, bail};
use ekko_ext::{
    ClientSnapshot, Color, EventKind, EventPayload, EventReturn, KeyIntercept, ModeOutcome,
    NoteKind, NoticeLevel, ProjectGroup, RegistryView, SessionEntry, SurfaceMouseEvent,
    ThemePalette, UiAction,
};
use mlua::{Lua, Table, Value};

// ── Into Lua ─────────────────────────────────────────────────────────────

pub fn snapshot_table(lua: &Lua, snapshot: &ClientSnapshot) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("session_name", snapshot.session_name.clone())?;
    t.set("mode", snapshot.mode.clone())?;
    t.set("cols", snapshot.cols)?;
    t.set("rows", snapshot.rows)?;
    t.set("grid_cols", snapshot.grid_cols)?;
    t.set("grid_rows", snapshot.grid_rows)?;
    t.set("scrollback", snapshot.scrollback)?;
    t.set("now_ms", snapshot.now_ms)?;
    let panes = lua.create_table()?;
    for (i, pane) in snapshot.panes.iter().enumerate() {
        let p = lua.create_table()?;
        p.set("id", pane.id)?;
        p.set("x", pane.x)?;
        p.set("y", pane.y)?;
        p.set("cols", pane.cols)?;
        p.set("rows", pane.rows)?;
        if let Some(title) = &pane.title {
            p.set("title", title.clone())?;
        }
        panes.set(i + 1, p)?;
    }
    t.set("panes", panes)?;
    if let Some(focused) = snapshot.focused_pane {
        t.set("focused_pane", focused)?;
    }
    let hidden = lua.create_table()?;
    for (i, name) in snapshot.hidden_surfaces.iter().enumerate() {
        hidden.set(i + 1, name.clone())?;
    }
    t.set("hidden_surfaces", hidden)?;
    if let Some(note) = &snapshot.status_note {
        let n = lua.create_table()?;
        n.set("text", note.text.clone())?;
        n.set(
            "kind",
            match note.kind {
                NoteKind::Ok => "ok",
                NoteKind::Error => "error",
                NoteKind::Info => "info",
            },
        )?;
        t.set("status_note", n)?;
    }
    let keybindings = lua.create_table()?;
    for (i, binding) in snapshot.keybindings.iter().enumerate() {
        let b = lua.create_table()?;
        b.set("chord_text", binding.chord_text.clone())?;
        b.set("mode", binding.mode.clone())?;
        b.set("description", binding.description.clone())?;
        keybindings.set(i + 1, b)?;
    }
    t.set("keybindings", keybindings)?;
    let projects = lua.create_table()?;
    for (i, project) in snapshot.projects.iter().enumerate() {
        let p = lua.create_table()?;
        p.set("name", project.name.clone())?;
        p.set("sessions", session_entries_table(lua, &project.sessions)?)?;
        projects.set(i + 1, p)?;
    }
    t.set("projects", projects)?;
    Ok(t)
}

/// Session entries as an array of plain tables — the grouper's input and
/// the per-project session lists inside the snapshot.
pub fn session_entries_table(lua: &Lua, sessions: &[SessionEntry]) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (i, session) in sessions.iter().enumerate() {
        let s = lua.create_table()?;
        s.set("name", session.name.clone())?;
        s.set("cwd", session.cwd.display().to_string())?;
        s.set(
            "state",
            match session.state {
                ekko_ext::SessionState::Alive => "alive",
                ekko_ext::SessionState::Gone => "gone",
            },
        )?;
        s.set("created_at_secs", session.created_at_secs)?;
        t.set(i + 1, s)?;
    }
    Ok(t)
}

pub fn mouse_event_table(lua: &Lua, event: &SurfaceMouseEvent) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "kind",
        match event.kind {
            ekko_ext::MouseKind::LeftPress => "left_press",
            ekko_ext::MouseKind::LeftDrag => "left_drag",
            ekko_ext::MouseKind::LeftRelease => "left_release",
            ekko_ext::MouseKind::WheelUp => "wheel_up",
            ekko_ext::MouseKind::WheelDown => "wheel_down",
            ekko_ext::MouseKind::Other => "other",
        },
    )?;
    t.set("col", event.col)?;
    t.set("row", event.row)?;
    t.set("region_cols", event.region_cols)?;
    t.set("region_rows", event.region_rows)?;
    Ok(t)
}

pub fn registry_view_table(lua: &Lua, view: &RegistryView) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let keybindings = lua.create_table()?;
    for (i, binding) in view.keybindings.iter().enumerate() {
        let b = lua.create_table()?;
        b.set("chord_text", binding.chord_text.clone())?;
        b.set("mode", binding.mode.clone())?;
        b.set("description", binding.description.clone())?;
        keybindings.set(i + 1, b)?;
    }
    t.set("keybindings", keybindings)?;
    let commands = lua.create_table()?;
    for (i, command) in view.commands.iter().enumerate() {
        let c = lua.create_table()?;
        c.set("name", command.name.clone())?;
        c.set("args_hint", command.args_hint.clone())?;
        c.set("description", command.description.clone())?;
        commands.set(i + 1, c)?;
    }
    t.set("commands", commands)?;
    Ok(t)
}

pub fn payload_table(lua: &Lua, payload: &EventPayload) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    match payload {
        EventPayload::Empty => {}
        EventPayload::SessionAttached {
            session_name,
            wire_version,
        } => {
            t.set("session_name", session_name.clone())?;
            t.set("wire_version", *wire_version)?;
        }
        EventPayload::BeforeSessionDetach { session_name } => {
            t.set("session_name", session_name.clone())?;
        }
        EventPayload::SessionSwitch { from, to } => {
            t.set("from", from.clone())?;
            t.set("to", to.clone())?;
        }
        EventPayload::GridUpdated { epoch, cols, rows } => {
            t.set("epoch", *epoch)?;
            t.set("cols", *cols)?;
            t.set("rows", *rows)?;
        }
        EventPayload::Resize { cols, rows } => {
            t.set("cols", *cols)?;
            t.set("rows", *rows)?;
        }
        EventPayload::Tick { now_ms } => {
            t.set("now_ms", *now_ms)?;
        }
        EventPayload::KeyInput { bytes } => {
            t.set("bytes", lua.create_string(bytes)?)?;
        }
        EventPayload::ModeChanged { from, to } => {
            t.set("from", from.clone())?;
            t.set("to", to.clone())?;
        }
        EventPayload::CommandInvoked { name, raw_args } => {
            t.set("name", name.clone())?;
            t.set("raw_args", raw_args.clone())?;
        }
        EventPayload::PtySpawn {
            session_name,
            shell,
            cwd,
            cols,
            rows,
        } => {
            t.set("session_name", session_name.clone())?;
            t.set("shell", shell.display().to_string())?;
            t.set("cwd", cwd.display().to_string())?;
            t.set("cols", *cols)?;
            t.set("rows", *rows)?;
        }
        EventPayload::SessionCreated {
            session_name,
            shell,
            cwd,
        } => {
            t.set("session_name", session_name.clone())?;
            t.set("shell", shell.display().to_string())?;
            t.set("cwd", cwd.display().to_string())?;
        }
        EventPayload::ClientAttached {
            session_name,
            client_id,
            cols,
            rows,
        } => {
            t.set("session_name", session_name.clone())?;
            t.set("client_id", *client_id)?;
            t.set("cols", *cols)?;
            t.set("rows", *rows)?;
        }
        EventPayload::ClientDetached {
            session_name,
            client_id,
        } => {
            t.set("session_name", session_name.clone())?;
            t.set("client_id", *client_id)?;
        }
        EventPayload::SessionExited {
            session_name,
            exit_code,
            reason,
        } => {
            t.set("session_name", session_name.clone())?;
            t.set("exit_code", *exit_code)?;
            t.set(
                "reason",
                match reason {
                    ekko_ext::SessionExitReason::ShellExited => "shell_exited",
                    ekko_ext::SessionExitReason::Killed => "killed",
                    ekko_ext::SessionExitReason::Crashed => "crashed",
                    ekko_ext::SessionExitReason::Shutdown => "shutdown",
                },
            )?;
        }
        EventPayload::PtyResized {
            session_name,
            cols,
            rows,
        } => {
            t.set("session_name", session_name.clone())?;
            t.set("cols", *cols)?;
            t.set("rows", *rows)?;
        }
        EventPayload::Heartbeat { session_name } | EventPayload::Bell { session_name } => {
            t.set("session_name", session_name.clone())?;
        }
    }
    Ok(t)
}

// ── Names ────────────────────────────────────────────────────────────────

/// Resolve a subscription string against the canonical names owned by
/// `ekko-event` (`EventKind::name`), so a new event variant is available to
/// Lua the moment it's added to the vocabulary.
pub fn event_kind_from_name(name: &str) -> Option<EventKind> {
    EventKind::from_name(name)
}

// ── Out of Lua ───────────────────────────────────────────────────────────

/// Normalize a handler's return into actions: `nil` (none), one action, or
/// an array of actions. An action is a bare string for unit variants or a
/// single-key table for payload variants.
pub fn actions_from_value(value: &Value) -> Result<Vec<UiAction>> {
    match value {
        Value::Nil => Ok(Vec::new()),
        Value::String(_) => Ok(vec![action_from_value(value)?]),
        Value::Table(t) => {
            if t.raw_len() > 0 {
                // Array of actions.
                let mut actions = Vec::new();
                for entry in t.clone().sequence_values::<Value>() {
                    actions.push(action_from_value(&entry?)?);
                }
                Ok(actions)
            } else {
                // A single table-form action.
                Ok(vec![action_from_value(value)?])
            }
        }
        other => bail!("expected nil, an action, or a list of actions, got {other:?}"),
    }
}

fn action_from_value(value: &Value) -> Result<UiAction> {
    match value {
        Value::String(s) => {
            let s = s.to_str()?;
            Ok(match &*s {
                "detach" => UiAction::Detach,
                "quit" => UiAction::Quit,
                "exit_mode" => UiAction::ExitMode,
                "close_overlay" => UiAction::CloseOverlay,
                "kill_current_session" => UiAction::KillCurrentSession,
                "new_session" => UiAction::NewSession { name: None },
                "scroll_to_bottom" => UiAction::ScrollToBottom,
                "search_clear" => UiAction::SearchClear,
                "edit_scrollback" => UiAction::EditScrollback,
                "split_right" => UiAction::SplitRight,
                "split_down" => UiAction::SplitDown,
                "close_focused_pane" => UiAction::CloseFocusedPane,
                other => bail!("unknown action '{other}'"),
            })
        }
        Value::Table(t) => {
            if let Some(name) = t.get::<Option<String>>("switch_session")? {
                return Ok(UiAction::SwitchSession { name });
            }
            if t.contains_key("new_session")? {
                let name = match t.get::<Value>("new_session")? {
                    Value::String(s) => Some(s.to_str()?.to_string()),
                    _ => None,
                };
                return Ok(UiAction::NewSession { name });
            }
            if let Some(name) = t.get::<Option<String>>("enter_mode")? {
                return Ok(UiAction::EnterMode { name });
            }
            if let Some(name) = t.get::<Option<String>>("open_overlay")? {
                return Ok(UiAction::OpenOverlay { name });
            }
            if let Some(name) = t.get::<Option<String>>("toggle_surface")? {
                return Ok(UiAction::ToggleSurface { name });
            }
            if let Some(line) = t.get::<Option<String>>("invoke_command")? {
                return Ok(UiAction::InvokeCommand { line });
            }
            if let Some(bytes) = t.get::<Option<mlua::String>>("forward_key")? {
                return Ok(UiAction::ForwardKey {
                    bytes: bytes.as_bytes().to_vec(),
                });
            }
            if let Some(delta) = t.get::<Option<i32>>("scroll")? {
                return Ok(UiAction::Scroll { delta });
            }
            if let Some(query) = t.get::<Option<String>>("search_scrollback")? {
                return Ok(UiAction::SearchScrollback { query });
            }
            if let Some(forward) = t.get::<Option<bool>>("search_jump")? {
                return Ok(UiAction::SearchMatchJump { forward });
            }
            if let Some(direction) = t.get::<Option<String>>("focus_direction")? {
                let direction = match direction.as_str() {
                    "left" => ekko_ext::PaneDirection::Left,
                    "right" => ekko_ext::PaneDirection::Right,
                    "up" => ekko_ext::PaneDirection::Up,
                    "down" => ekko_ext::PaneDirection::Down,
                    other => bail!("unknown focus_direction '{other}'"),
                };
                return Ok(UiAction::FocusPaneDirection { direction });
            }
            if let Some(note) = t.get::<Option<Table>>("set_status_note")? {
                return Ok(UiAction::SetStatusNote {
                    text: note.get::<Option<String>>("text")?.unwrap_or_default(),
                    kind: match note
                        .get::<Option<String>>("kind")?
                        .unwrap_or_else(|| "ok".into())
                        .as_str()
                    {
                        "error" => NoteKind::Error,
                        "info" => NoteKind::Info,
                        _ => NoteKind::Ok,
                    },
                    ttl_ms: note.get::<Option<u64>>("ttl_ms")?.unwrap_or(4_000),
                });
            }
            bail!("action table has no recognized key")
        }
        other => bail!("expected an action string or table, got {other:?}"),
    }
}

/// Normalize a mode `on_key` return into a [`ModeOutcome`]. The dialect
/// deliberately mirrors the overlay `handle_key` dialect ("close" becomes
/// "exit"): `nil` → `Continue`; the string `"exit"` → `Exit`; an array with
/// an `"exit"` head → `ExitWith` the remaining actions; any other action
/// table/array → `ContinueWith`. Unrecognized values degrade to `Continue`
/// — a mode swallows what it doesn't understand.
pub fn mode_outcome_from_value(value: &Value) -> ModeOutcome {
    match value {
        Value::Nil => ModeOutcome::Continue,
        Value::String(s) => {
            if s.to_string_lossy() == "exit" {
                ModeOutcome::Exit
            } else {
                ModeOutcome::Continue
            }
        }
        Value::Table(t) => {
            // Array form: { "exit", action, action, ... }
            if t.raw_len() > 0
                && matches!(
                    t.get::<Option<Value>>(1),
                    Ok(Some(Value::String(s))) if s.to_string_lossy() == "exit"
                )
            {
                let mut actions = Vec::new();
                let vals: Vec<Value> = t
                    .clone()
                    .sequence_values::<Value>()
                    .skip(1)
                    .collect::<mlua::Result<_>>()
                    .unwrap_or_default();
                for v in &vals {
                    if let Ok(mut a) = actions_from_value(v) {
                        actions.append(&mut a);
                    }
                }
                if actions.is_empty() {
                    ModeOutcome::Exit
                } else {
                    ModeOutcome::ExitWith(actions)
                }
            } else {
                match actions_from_value(value) {
                    Ok(actions) if !actions.is_empty() => ModeOutcome::ContinueWith(actions),
                    _ => ModeOutcome::Continue,
                }
            }
        }
        _ => ModeOutcome::Continue,
    }
}

/// A mode render's optional hardware-cursor return: `{ row =, col = }` → a
/// position, anything else → no cursor.
pub fn cursor_from_value(value: &Value) -> Option<(i32, i32)> {
    let Value::Table(t) = value else {
        return None;
    };
    let row = t.get::<Option<i32>>("row").ok()??;
    let col = t.get::<Option<i32>>("col").ok()??;
    Some((row, col))
}

/// Parse a grouper's return: an array of `{ name =, sessions = { ... } }`
/// tables where sessions are referenced **by name** — scripts cannot
/// fabricate entries, only arrange the ones they were given.
pub fn group_claims_from_value(value: &Value) -> Result<Vec<(String, Vec<String>)>> {
    let Value::Table(t) = value else {
        bail!("expected an array of groups, got {value:?}");
    };
    let mut claims = Vec::new();
    for entry in t.clone().sequence_values::<Table>() {
        let group = entry?;
        let name: String = group
            .get::<Option<String>>("name")?
            .ok_or_else(|| anyhow!("group table needs a 'name'"))?;
        let sessions: Vec<String> = match group.get::<Option<Table>>("sessions")? {
            Some(t) => t.sequence_values::<String>().collect::<mlua::Result<_>>()?,
            None => Vec::new(),
        };
        claims.push((name, sessions));
    }
    Ok(claims)
}

/// Rehydrate name claims against the real input entries. Unknown names and
/// repeat claims are dropped; groups left empty are dropped; every input
/// session no group claimed lands in a trailing "ungrouped" group, so a
/// buggy script cannot make sessions vanish from the sidebar.
pub fn rehydrate_groups(
    claims: Vec<(String, Vec<String>)>,
    sessions: Vec<SessionEntry>,
) -> Vec<ProjectGroup> {
    let index: std::collections::HashMap<String, usize> = sessions
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.clone(), i))
        .collect();
    let mut pool: Vec<Option<SessionEntry>> = sessions.into_iter().map(Some).collect();
    let mut groups = Vec::new();
    for (name, session_names) in claims {
        let members: Vec<SessionEntry> = session_names
            .iter()
            .filter_map(|n| pool[*index.get(n)?].take())
            .collect();
        if !members.is_empty() {
            groups.push(ProjectGroup {
                name,
                sessions: members,
            });
        }
    }
    let unclaimed: Vec<SessionEntry> = pool.into_iter().flatten().collect();
    if !unclaimed.is_empty() {
        groups.push(ProjectGroup {
            name: "ungrouped".to_string(),
            sessions: unclaimed,
        });
    }
    groups
}

/// Normalize a subscription handler's return into an [`EventReturn`].
pub fn event_return_from_value(value: &Value) -> Result<Option<EventReturn>> {
    let Value::Table(t) = value else {
        return Ok(None);
    };
    if let Some(reason) = t.get::<Option<String>>("cancel")? {
        return Ok(Some(EventReturn::Cancel { reason }));
    }
    if t.get::<Option<bool>>("consume")?.unwrap_or(false) {
        return Ok(Some(EventReturn::KeyIntercept(KeyIntercept::Consume)));
    }
    if let Some(replacement) = t.get::<Option<mlua::String>>("transform")? {
        return Ok(Some(EventReturn::KeyIntercept(KeyIntercept::Transform(
            replacement.as_bytes().to_vec(),
        ))));
    }
    if let Some(notice) = t.get::<Option<Table>>("notice")? {
        return Ok(Some(EventReturn::EmitNotice {
            level: match notice
                .get::<Option<String>>("level")?
                .unwrap_or_else(|| "info".into())
                .as_str()
            {
                "warn" => NoticeLevel::Warn,
                _ => NoticeLevel::Info,
            },
            message: notice.get::<Option<String>>("message")?.unwrap_or_default(),
        }));
    }
    if let Some(over) = t.get::<Option<Table>>("spawn_override")? {
        let mut env = Vec::new();
        if let Some(env_table) = over.get::<Option<Table>>("env")? {
            for pair in env_table.pairs::<String, String>() {
                let (key, value) = pair?;
                env.push((key, value));
            }
        }
        return Ok(Some(EventReturn::PtySpawnOverride {
            shell: over.get::<Option<String>>("shell")?.map(Into::into),
            cwd: over.get::<Option<String>>("cwd")?.map(Into::into),
            env,
        }));
    }
    Ok(None)
}

// ── Colors / themes ──────────────────────────────────────────────────────

/// Resolve a color word against the active palette: a role name (`"text"`,
/// `"accent"`, ...), `"transparent"`, or `"#rrggbb"` hex.
pub fn resolve_color(word: &str, palette: &ThemePalette) -> Result<Color> {
    Ok(match word {
        "text" => palette.text,
        "muted" => palette.muted,
        "heading" => palette.heading,
        "accent" => palette.accent,
        "accent_2" => palette.accent_2,
        "surface" => palette.surface,
        "surface_raised" => palette.surface_raised,
        "sidebar_bg" => palette.sidebar_bg,
        "status_fg" => palette.status_fg,
        "status_bg" => palette.status_bg,
        "border" => palette.border,
        "running" => palette.running,
        "warning" => palette.warning,
        "error" => palette.error,
        "success" => palette.success,
        "term_fg" => palette.term_fg,
        "term_bg" => palette.term_bg,
        "selection_fg" => palette.selection_fg,
        "selection_bg" => palette.selection_bg,
        "transparent" => Color::TRANSPARENT,
        hex => parse_hex_color(hex)?,
    })
}

fn parse_hex_color(word: &str) -> Result<Color> {
    let hex = word
        .strip_prefix('#')
        .ok_or_else(|| anyhow!("unknown color '{word}' (role name or #rrggbb)"))?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("bad hex color '{word}' (want #rrggbb)");
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
    Ok(Color::rgb(channel(0), channel(2), channel(4)))
}

/// Build a theme palette from an optional Lua table of hex colors; missing
/// fields keep the fallback palette's value.
pub fn palette_from_table(table: Option<Table>) -> Result<ThemePalette> {
    let mut palette = ThemePalette::fallback();
    let Some(table) = table else {
        return Ok(palette);
    };
    let set = |field: &mut Color, name: &str| -> Result<()> {
        if let Some(word) = table.get::<Option<String>>(name)? {
            *field = parse_hex_color(&word)?;
        }
        Ok(())
    };
    macro_rules! fields {
        ($($name:ident),* $(,)?) => {
            $(set(&mut palette.$name, stringify!($name))?;)*
        };
    }
    fields!(
        text,
        muted,
        heading,
        accent,
        accent_2,
        surface,
        surface_raised,
        sidebar_bg,
        status_fg,
        status_bg,
        border,
        running,
        warning,
        error,
        success,
        term_fg,
        term_bg,
        selection_fg,
        selection_bg,
    );
    Ok(palette)
}
