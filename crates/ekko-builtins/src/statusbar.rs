//! The stock statusbar: mode chip, project name, centered session name, and
//! on the right a transient status note or a key-hint strip generated from
//! the live keybinding registry. A docked bottom surface.

use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{
    ClientSnapshot, DockEdge, DrawContext, Extension, ExtensionHost, ExtensionManifest, NoteKind,
    Rect, SessionState, SurfaceSize, SurfaceSpec,
};
use ekko_tui::display_cell_width;

pub struct StatusbarExtension;

impl Extension for StatusbarExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.statusbar".into(),
            name: "statusbar".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "mode/project/session statusbar".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.register_surface(SurfaceSpec {
            name: "statusbar".into(),
            dock: DockEdge::Bottom,
            priority: 0,
            size: SurfaceSize::Fixed(1),
            hide_below: None,
            visible: None,
            draw: Arc::new(draw_statusbar),
            on_mouse: None,
            wants_tick: None,
        })
    }
}

fn draw_statusbar(ctx: &mut dyn DrawContext, snapshot: &ClientSnapshot) {
    let (cols, rows) = ctx.size();
    if cols <= 0 || rows <= 0 {
        return;
    }
    let theme = &snapshot.theme;
    let in_mode = snapshot.mode != ClientSnapshot::NORMAL_MODE;
    let mode_chip = mode_chip(&snapshot.mode);
    let (mode_label, bg, fg) = match (in_mode, &snapshot.status_note) {
        (true, _) => (mode_chip.as_str(), theme.warning, theme.status_fg),
        (false, Some(note)) if note.kind == NoteKind::Error => {
            (" ERR ", theme.error, theme.status_fg)
        }
        (false, Some(note)) if note.kind == NoteKind::Ok => {
            (" OK  ", theme.success, theme.status_fg)
        }
        (false, _) => (" NOR ", theme.status_bg, theme.status_fg),
    };

    ctx.fill_rect(Rect::new(0, 0, cols, rows), fg, bg);
    ctx.put_text_bold(0, 0, mode_label.len() as i32, fg, bg, mode_label);

    let main_col = mode_label.len() as i32;
    let main_cols = (cols - main_col).max(0);
    if main_cols <= 0 {
        return;
    }

    let project = current_project_name(snapshot);
    let left = format!(" {project} ");
    let left_cols = display_cell_width(&left).min(main_cols as usize) as i32;
    ctx.put_text(main_col, 0, left_cols, fg, bg, &left);

    let center = format!(" {} ", snapshot.session_name);
    let center_cols = display_cell_width(&center) as i32;
    let mut center_end = main_col + left_cols;
    if center_cols > 0 && center_cols <= main_cols {
        let centered_col = main_col + (main_cols - center_cols) / 2;
        if centered_col >= main_col + left_cols {
            ctx.put_text(centered_col, 0, center_cols, theme.accent_2, bg, &center);
            center_end = centered_col + center_cols;
        }
    }

    // Right side: transient note > dead-session marker > key hints. Budgeted
    // to the space right of the centered session name so nothing overlaps.
    let right_budget = (cols - center_end - 1).max(0) as usize;
    let (right, right_fg) = status_right_text(snapshot, right_budget, theme.muted, fg);
    let right_cols = display_cell_width(&right).min(right_budget) as i32;
    if right_cols > 0 {
        ctx.put_text(cols - right_cols, 0, right_cols, right_fg, bg, &right);
    }
}

/// Three-letter chip for the active mode: `command` → ` CMD `, `scroll` →
/// ` SCR `, anything else its first three letters uppercased.
fn mode_chip(mode: &str) -> String {
    let short: String = match mode {
        "command" => "CMD".into(),
        "scroll" => "SCR".into(),
        other => other.chars().take(3).collect::<String>().to_uppercase(),
    };
    format!(" {short:<3} ")
}

fn current_project_name(snapshot: &ClientSnapshot) -> String {
    snapshot
        .projects
        .iter()
        .find(|p| p.sessions.iter().any(|s| s.name == snapshot.session_name))
        .map(|p| p.name.clone())
        .unwrap_or_default()
}

fn status_right_text(
    snapshot: &ClientSnapshot,
    budget: usize,
    hint_fg: ekko_ext::Color,
    fg: ekko_ext::Color,
) -> (String, ekko_ext::Color) {
    if let Some(note) = &snapshot.status_note {
        return (format!(" {} ", note.text), fg);
    }
    if snapshot.scrollback > 0 {
        return (format!(" ⇡ {} ", snapshot.scrollback), fg);
    }
    let alive = snapshot
        .projects
        .iter()
        .flat_map(|p| p.sessions.iter())
        .find(|s| s.name == snapshot.session_name)
        .map(|s| s.state == SessionState::Alive)
        .unwrap_or(true);
    if !alive {
        return (" gone ".to_string(), fg);
    }
    (key_hints(&snapshot.keybindings, budget), hint_fg)
}

/// Greedy hint strip from the live registry: `"alt+j next session · ..."`,
/// taking whole entries while they fit. Multi-chord bindings show only their
/// primary chord. Self-documenting by construction — a rebind in config
/// changes the hint automatically.
fn key_hints(keybindings: &[ekko_ext::KeybindingInfo], budget: usize) -> String {
    let mut hints = String::new();
    // Only normal-mode bindings: mode-scoped entries (a leader map's keys)
    // are hints for a mode the user isn't in.
    for info in keybindings.iter().filter(|info| info.mode.is_none()) {
        let chord = info.chord_text.split(" / ").next().unwrap_or_default();
        if chord.is_empty() || info.description.is_empty() {
            continue;
        }
        let segment = if hints.is_empty() {
            format!("{chord} {}", info.description)
        } else {
            format!(" · {chord} {}", info.description)
        };
        if display_cell_width(&hints) + display_cell_width(&segment) + 1 > budget {
            break;
        }
        hints.push_str(&segment);
    }
    if hints.is_empty() {
        return String::new();
    }
    format!("{hints} ")
}
