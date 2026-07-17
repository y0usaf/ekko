//! Leader key + which-key panel, registered through the public API. The
//! leader chord enters a mode whose panel lists every keybinding registered
//! under `mode = "leader"`; the *host* dispatches the next key against those
//! bindings (mode-scoped registry bindings match before the mode's own key
//! handler), so any extension — or user config, via Lua — extends the map by
//! registering more `mode = "leader"` entries, and the panel picks them up
//! automatically. This extension is only the chord, the stock map, the
//! panel, and the unbound-key fallback.
//!
//! A leader entry that runs a plain action should return `UiAction::ExitMode`
//! ahead of it — the host stays in the mode otherwise, which is what lets an
//! entry be "sticky" (repeatable) on purpose.

use std::sync::Arc;

use anyhow::Result;
use ekko_config::Config;
use ekko_ext::{
    ClientSnapshot, DrawContext, Extension, ExtensionHost, ExtensionManifest, KeybindingSpec,
    ModeOutcome, ModeSpec, ModeState, NoteKind, OVERLAY_HELP, Rect, UiAction, resolve_chords,
};
use ekko_tui::display_cell_width;

use crate::command_mode::COMMAND_MODE;
use crate::scroll_mode::SCROLL_MODE;

pub const LEADER_MODE: &str = "leader";

const UNBOUND_NOTE_TTL_MS: u64 = 2000;

pub struct LeaderExtension {
    /// Binding strings for the leader chord itself (`[keybinds] leader`).
    leader: Vec<String>,
    /// The stock map: key strings, description, actions. Keys come from
    /// config (`[keybinds] "leader.help" = "h"`) with single-char defaults.
    map: Vec<(Vec<String>, String, Vec<UiAction>)>,
    /// Whether the builtin sidebar is enabled ([`Config`] `[extensions]
    /// disabled`): the sidebar-toggle entries are pointless without it.
    sidebar_enabled: bool,
}

impl LeaderExtension {
    pub fn new(config: &Config) -> Self {
        let sidebar_enabled = !config
            .extensions
            .disabled
            .iter()
            .any(|id| id == "ekko-builtins.sidebar");
        let mut stock: Vec<(&str, &[&str], &str, Vec<UiAction>)> = vec![
            (
                "leader.command_mode",
                &["e"],
                "command mode",
                vec![UiAction::EnterMode {
                    name: COMMAND_MODE.into(),
                }],
            ),
            (
                "leader.scroll_mode",
                &["s"],
                "scroll",
                vec![UiAction::EnterMode {
                    name: SCROLL_MODE.into(),
                }],
            ),
            (
                "leader.new_session",
                &["n"],
                "new session",
                vec![UiAction::ExitMode, UiAction::NewSession { name: None }],
            ),
            (
                "leader.detach",
                &["d"],
                "detach",
                vec![UiAction::ExitMode, UiAction::Detach],
            ),
            (
                "leader.toggle_sidebar",
                &["b"],
                "toggle sidebar",
                vec![
                    UiAction::ExitMode,
                    UiAction::ToggleSurface {
                        name: "sidebar".into(),
                    },
                ],
            ),
            (
                "leader.help",
                &["?"],
                "help",
                vec![
                    UiAction::ExitMode,
                    UiAction::OpenOverlay {
                        name: OVERLAY_HELP.into(),
                    },
                ],
            ),
        ];
        if !sidebar_enabled {
            stock.retain(|(action, ..)| *action != "leader.toggle_sidebar");
        }
        Self {
            leader: config.bindings_for("leader", &["ctrl+space"]),
            map: stock
                .into_iter()
                .map(|(action, defaults, description, actions)| {
                    (
                        config.bindings_for(action, defaults),
                        description.to_string(),
                        actions,
                    )
                })
                .collect(),
            sidebar_enabled,
        }
    }
}

impl Extension for LeaderExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.leader".into(),
            name: "leader".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "leader key with a which-key panel of `mode = \"leader\"` bindings".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        if let Some((chords, chord_text)) = resolve_chords(&self.leader) {
            host.register_keybinding(KeybindingSpec {
                chords: chords.clone(),
                chord_text: chord_text.clone(),
                mode: None,
                description: "leader".into(),
                handler: Arc::new(|_| {
                    vec![UiAction::EnterMode {
                        name: LEADER_MODE.into(),
                    }]
                }),
            })?;
            // The leader chord pressed again inside leader mode toggles the
            // session sidebar: leader-leader opens/closes the session list.
            // With the sidebar disabled it just closes the panel (and any
            // mode-attached overlay with it). Registered as a mode-scoped
            // binding so it beats the mode's own key handler (which
            // swallows non-printables).
            let sidebar_enabled = self.sidebar_enabled;
            host.register_keybinding(KeybindingSpec {
                chords,
                chord_text,
                mode: Some(LEADER_MODE.into()),
                description: if sidebar_enabled {
                    "toggle sidebar".into()
                } else {
                    "close panel".into()
                },
                handler: Arc::new(move |_| {
                    let mut actions = vec![UiAction::ExitMode];
                    if sidebar_enabled {
                        actions.push(UiAction::ToggleSurface {
                            name: "sidebar".into(),
                        });
                    }
                    actions
                }),
            })?;
        }

        host.register_mode(ModeSpec {
            name: LEADER_MODE.into(),
            init_state: Arc::new(|| Box::new(()) as ModeState),
            on_key: Arc::new(handle_key),
            render: Some(Arc::new(render_panel)),
        })?;

        for (strings, description, actions) in &self.map {
            let Some((chords, chord_text)) = resolve_chords(strings) else {
                continue;
            };
            let actions = actions.clone();
            host.register_keybinding(KeybindingSpec {
                chords,
                chord_text,
                mode: Some(LEADER_MODE.into()),
                description: description.clone(),
                handler: Arc::new(move |_| actions.clone()),
            })?;
        }
        Ok(())
    }
}

/// Fallback for keys no registered leader binding matched (the host tries
/// the registry first): unbound printables exit with a note, Esc exits
/// quietly, everything else is ignored.
fn handle_key(_state: &mut ModeState, bytes: &[u8], _snapshot: &ClientSnapshot) -> ModeOutcome {
    match printable(bytes) {
        Some(key) => ModeOutcome::ExitWith(vec![UiAction::SetStatusNote {
            text: format!("leader: '{key}' is unbound"),
            kind: NoteKind::Info,
            ttl_ms: UNBOUND_NOTE_TTL_MS,
        }]),
        // Only Esc closes the panel. Every other non-printable — the leader
        // chord autorepeating, a chord typed with ctrl still held, mouse
        // reports, stray escape sequences — is swallowed: exiting on those
        // made the chord toggle the mode, so autorepeat parity decided
        // whether the panel was open when the user let go.
        None if bytes == b"\x1b" => ModeOutcome::Exit,
        None => ModeOutcome::Continue,
    }
}

/// A single non-control character, if that's what the token is.
fn printable(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut chars = text.chars();
    let ch = chars.next()?;
    (chars.next().is_none() && !ch.is_control()).then_some(text)
}

/// Bottom-anchored which-key panel: the `mode = "leader"` registry entries
/// laid out column-major, keys accented.
fn render_panel(
    ctx: &mut dyn DrawContext,
    _state: &ModeState,
    snapshot: &ClientSnapshot,
) -> Option<(i32, i32)> {
    let entries: Vec<(&str, &str)> = snapshot
        .keybindings
        .iter()
        .filter(|b| b.mode.as_deref() == Some(LEADER_MODE))
        .map(|b| (b.chord_text.as_str(), b.description.as_str()))
        .collect();
    let (cols, rows) = ctx.size();
    if cols < 8 || rows < 4 {
        return None;
    }
    let theme = &snapshot.theme;

    if entries.is_empty() {
        let rect = Rect::new(0, rows - 3, cols, 3);
        ctx.draw_box(rect, theme.text, theme.surface, theme.border);
        ctx.put_text_bold(2, rect.row, 8, theme.heading, theme.surface, " leader ");
        ctx.put_text(
            2,
            rect.row + 1,
            cols - 4,
            theme.muted,
            theme.surface,
            "no leader bindings registered",
        );
        return None;
    }

    let key_width = entries
        .iter()
        .map(|(key, _)| display_cell_width(key) as i32)
        .max()
        .unwrap_or(1);
    let cell_width = entries
        .iter()
        .map(|(key, description)| {
            display_cell_width(key) as i32 + 2 + display_cell_width(description) as i32
        })
        .max()
        .unwrap_or(8)
        .min(cols - 4);
    let inner = cols - 4;
    let columns = ((inner + 3) / (cell_width + 3)).clamp(1, entries.len() as i32);
    let panel_rows = (entries.len() as i32 + columns - 1) / columns;
    let height = (panel_rows + 2).min(rows - 1);
    let rect = Rect::new(0, rows - height, cols, height);
    ctx.draw_box(rect, theme.text, theme.surface, theme.border);
    ctx.put_text_bold(2, rect.row, 8, theme.heading, theme.surface, " leader ");
    for (i, (key, description)) in entries.iter().enumerate() {
        let row = rect.row + 1 + (i as i32 % panel_rows);
        if row >= rect.row + rect.rows - 1 {
            continue;
        }
        let col = 2 + (i as i32 / panel_rows) * (cell_width + 3);
        ctx.put_text_bold(col, row, key_width, theme.accent, theme.surface, key);
        ctx.put_text(
            col + key_width + 2,
            row,
            cell_width - key_width - 2,
            theme.text,
            theme.surface,
            description,
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_ext::{RuntimeBuilder, ThemePalette};

    fn runtime() -> ekko_ext::AppRuntime {
        RuntimeBuilder::new()
            .register_extension(LeaderExtension::new(&Config::default()))
            .build()
            .unwrap()
    }

    fn runtime_without_sidebar() -> ekko_ext::AppRuntime {
        let mut config = Config::default();
        config
            .extensions
            .disabled
            .push("ekko-builtins.sidebar".into());
        RuntimeBuilder::new()
            .register_extension(LeaderExtension::new(&config))
            .build()
            .unwrap()
    }

    fn snapshot() -> ClientSnapshot {
        ClientSnapshot {
            panes: vec![],
            focused_pane: None,
            session_name: "s".into(),
            mode: LEADER_MODE.into(),
            cols: 80,
            rows: 24,
            grid_cols: 80,
            grid_rows: 22,
            scrollback: 0,
            projects: vec![],
            status_note: None,
            keybindings: vec![],
            now_ms: 0,
            hidden_surfaces: Vec::new(),
            theme: ThemePalette::fallback(),
        }
    }

    #[test]
    fn leader_chord_enters_the_mode() {
        let rt = runtime();
        // ctrl+space arrives as NUL.
        let spec = rt.match_keybinding(&[0x00], None).expect("leader bound");
        assert_eq!(
            (spec.handler)(&snapshot()),
            vec![UiAction::EnterMode {
                name: LEADER_MODE.into()
            }]
        );
    }

    #[test]
    fn stock_entries_are_leader_scoped() {
        let rt = runtime();
        let spec = rt
            .match_keybinding(b"s", Some(LEADER_MODE))
            .expect("s bound in leader");
        assert_eq!(
            (spec.handler)(&snapshot()),
            vec![UiAction::EnterMode {
                name: SCROLL_MODE.into()
            }]
        );
        let spec = rt
            .match_keybinding(b"n", Some(LEADER_MODE))
            .expect("n bound in leader");
        assert_eq!(
            (spec.handler)(&snapshot()),
            vec![UiAction::ExitMode, UiAction::NewSession { name: None }]
        );
        // Leader entries never fire in normal mode.
        assert!(rt.match_keybinding(b"s", None).is_none());
    }

    #[test]
    fn unbound_printable_exits_with_a_note() {
        let rt = runtime();
        let spec = rt.mode(LEADER_MODE).unwrap();
        let mut state = (spec.init_state)();
        let outcome = (spec.on_key)(&mut state, b"z", &snapshot());
        assert_eq!(
            outcome,
            ModeOutcome::ExitWith(vec![UiAction::SetStatusNote {
                text: "leader: 'z' is unbound".into(),
                kind: NoteKind::Info,
                ttl_ms: UNBOUND_NOTE_TTL_MS,
            }])
        );
    }

    #[test]
    fn esc_exits_quietly() {
        let rt = runtime();
        let spec = rt.mode(LEADER_MODE).unwrap();
        let mut state = (spec.init_state)();
        assert_eq!(
            (spec.on_key)(&mut state, b"\x1b", &snapshot()),
            ModeOutcome::Exit
        );
    }

    #[test]
    fn leader_chord_repeats_and_other_non_printables_are_swallowed() {
        let rt = runtime();
        let spec = rt.mode(LEADER_MODE).unwrap();
        // The leader chord autorepeating (0x00/0x02), a chord with ctrl
        // still held (0x13), arrows, and mouse reports must not close the
        // panel — exiting here made autorepeat parity toggle the mode.
        for bytes in [
            [0x00].as_slice(),
            &[0x02],
            &[0x13],
            b"\x1b[A",
            b"\x1b[<64;1;1M",
        ] {
            let mut state = (spec.init_state)();
            assert_eq!(
                (spec.on_key)(&mut state, bytes, &snapshot()),
                ModeOutcome::Continue,
                "bytes {bytes:?}"
            );
        }
    }

    #[test]
    fn leader_chord_again_toggles_the_sidebar() {
        let rt = runtime();
        // ctrl+space arrives as NUL; inside leader mode it toggles the
        // session sidebar (leader-leader = open/close the session list).
        let spec = rt
            .match_keybinding(&[0x00], Some(LEADER_MODE))
            .expect("leader chord bound in leader mode");
        assert_eq!(
            (spec.handler)(&snapshot()),
            vec![
                UiAction::ExitMode,
                UiAction::ToggleSurface {
                    name: "sidebar".into()
                }
            ]
        );
    }

    #[test]
    fn toggle_sidebar_entry_is_leader_scoped() {
        let rt = runtime();
        let spec = rt
            .match_keybinding(b"b", Some(LEADER_MODE))
            .expect("b bound in leader");
        assert_eq!(
            (spec.handler)(&snapshot()),
            vec![
                UiAction::ExitMode,
                UiAction::ToggleSurface {
                    name: "sidebar".into()
                }
            ]
        );
    }

    #[test]
    fn disabled_sidebar_drops_the_toggle_and_repeat_just_closes() {
        let rt = runtime_without_sidebar();
        // No 'b' entry: toggling a surface that is never registered would
        // only produce an error note.
        assert!(rt.match_keybinding(b"b", Some(LEADER_MODE)).is_none());
        // The leader chord repeated inside leader mode plainly closes the
        // panel (and any mode-attached overlay with it).
        let spec = rt
            .match_keybinding(&[0x00], Some(LEADER_MODE))
            .expect("leader chord bound in leader mode");
        assert_eq!((spec.handler)(&snapshot()), vec![UiAction::ExitMode]);
    }

    #[test]
    fn config_overrides_leader_and_entry_keys() {
        let config: Config =
            toml::from_str("[keybinds]\nleader = \"alt+g\"\n\"leader.help\" = \"h\"\n").unwrap();
        let rt = RuntimeBuilder::new()
            .register_extension(LeaderExtension::new(&config))
            .build()
            .unwrap();
        assert!(rt.match_keybinding(&[0x00], None).is_none());
        assert!(rt.match_keybinding(b"\x1bg", None).is_some());
        assert!(rt.match_keybinding(b"?", Some(LEADER_MODE)).is_none());
        assert!(rt.match_keybinding(b"h", Some(LEADER_MODE)).is_some());
    }
}
