//! Stock pane management: `:split` / `:pane-focus` / `:pane-close` commands
//! and the leader-map pane keys, all registered through the public API. The
//! mechanism (wire requests, hub topology) is core; this extension is only
//! the stock command names and chords. Leader entries are ordinary
//! `mode = "leader"` keybindings, so the which-key panel and help pick them
//! up from the live registry with no pane-specific branch anywhere.

use std::sync::Arc;

use anyhow::Result;
use ekko_config::Config;
use ekko_ext::{
    CommandInvocation, CommandOutput, CommandSpec, Extension, ExtensionHost, ExtensionManifest,
    KeybindingSpec, NoteKind, PaneDirection, UiAction, resolve_chords,
};

use crate::leader::LEADER_MODE;

pub struct PanesExtension {
    /// Leader-map entries: `(config action, default keys, description,
    /// actions)`. Key strings come from `[keybinds] "leader.split_right"`
    /// etc. with the stock single-char defaults.
    leader_map: Vec<(Vec<String>, String, Vec<UiAction>)>,
}

impl PanesExtension {
    pub fn new(config: &Config) -> Self {
        let focus = |direction: PaneDirection| {
            vec![
                UiAction::ExitMode,
                UiAction::FocusPaneDirection { direction },
            ]
        };
        let stock: Vec<(&str, &[&str], &str, Vec<UiAction>)> = vec![
            (
                "leader.split_right",
                &["|"],
                "split right",
                vec![UiAction::ExitMode, UiAction::SplitRight],
            ),
            (
                "leader.split_down",
                &["-"],
                "split down",
                vec![UiAction::ExitMode, UiAction::SplitDown],
            ),
            (
                "leader.pane_left",
                &["h"],
                "focus left",
                focus(PaneDirection::Left),
            ),
            (
                "leader.pane_down",
                &["j"],
                "focus down",
                focus(PaneDirection::Down),
            ),
            (
                "leader.pane_up",
                &["k"],
                "focus up",
                focus(PaneDirection::Up),
            ),
            (
                "leader.pane_right",
                &["l"],
                "focus right",
                focus(PaneDirection::Right),
            ),
            (
                "leader.pane_close",
                &["x"],
                "close pane",
                vec![UiAction::ExitMode, UiAction::CloseFocusedPane],
            ),
        ];
        Self {
            leader_map: stock
                .into_iter()
                .map(|(action, defaults, description, actions)| {
                    (
                        config.bindings_for(action, defaults),
                        description.to_string(),
                        actions,
                    )
                })
                .collect(),
        }
    }
}

impl Extension for PanesExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.panes".into(),
            name: "panes".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "pane commands (:split/:pane-focus/:pane-close) and leader pane keys"
                .into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.register_command(CommandSpec {
            name: "split".into(),
            aliases: vec![],
            description: "split the focused pane".into(),
            args_hint: "right|down".into(),
            handler: Arc::new(|inv: CommandInvocation| {
                match inv.raw_args.split_whitespace().next() {
                    Some("right") => Ok(CommandOutput::action(UiAction::SplitRight)),
                    Some("down") => Ok(CommandOutput::action(UiAction::SplitDown)),
                    _ => Ok(CommandOutput::note(
                        "usage: split right|down",
                        NoteKind::Error,
                    )),
                }
            }),
        })?;
        host.register_command(CommandSpec {
            name: "pane-focus".into(),
            aliases: vec![],
            description: "focus the neighboring pane in a direction".into(),
            args_hint: "left|right|up|down".into(),
            handler: Arc::new(|inv: CommandInvocation| {
                let direction = match inv.raw_args.split_whitespace().next() {
                    Some("left") => PaneDirection::Left,
                    Some("right") => PaneDirection::Right,
                    Some("up") => PaneDirection::Up,
                    Some("down") => PaneDirection::Down,
                    _ => {
                        return Ok(CommandOutput::note(
                            "usage: pane-focus left|right|up|down",
                            NoteKind::Error,
                        ));
                    }
                };
                Ok(CommandOutput::action(UiAction::FocusPaneDirection {
                    direction,
                }))
            }),
        })?;
        host.register_command(CommandSpec {
            name: "pane-close".into(),
            aliases: vec![],
            description: "close the focused pane".into(),
            args_hint: String::new(),
            handler: Arc::new(|_| Ok(CommandOutput::action(UiAction::CloseFocusedPane))),
        })?;

        for (strings, description, actions) in &self.leader_map {
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

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_ext::{CommandDispatch, RuntimeBuilder};

    fn runtime() -> ekko_ext::AppRuntime {
        RuntimeBuilder::new()
            .register_extension(PanesExtension::new(&Config::default()))
            .build()
            .unwrap()
    }

    #[test]
    fn split_command_parses_right_and_down() {
        let rt = runtime();
        assert_eq!(
            rt.invoke_command(":split right"),
            CommandDispatch::Invoked(vec![UiAction::SplitRight])
        );
        assert_eq!(
            rt.invoke_command(":split down"),
            CommandDispatch::Invoked(vec![UiAction::SplitDown])
        );
        assert!(matches!(
            rt.invoke_command(":split"),
            CommandDispatch::Invoked(actions)
                if matches!(actions.as_slice(), [UiAction::SetStatusNote { kind: NoteKind::Error, .. }])
        ));
        assert!(matches!(
            rt.invoke_command(":split sideways"),
            CommandDispatch::Invoked(actions)
                if matches!(actions.as_slice(), [UiAction::SetStatusNote { kind: NoteKind::Error, .. }])
        ));
    }

    #[test]
    fn pane_focus_command_parses_directions() {
        let rt = runtime();
        for (arg, direction) in [
            ("left", PaneDirection::Left),
            ("right", PaneDirection::Right),
            ("up", PaneDirection::Up),
            ("down", PaneDirection::Down),
        ] {
            assert_eq!(
                rt.invoke_command(&format!(":pane-focus {arg}")),
                CommandDispatch::Invoked(vec![UiAction::FocusPaneDirection { direction }]),
                "direction {arg}"
            );
        }
        assert!(matches!(
            rt.invoke_command(":pane-focus nowhere"),
            CommandDispatch::Invoked(actions)
                if matches!(actions.as_slice(), [UiAction::SetStatusNote { kind: NoteKind::Error, .. }])
        ));
    }

    #[test]
    fn pane_close_command_invokes_close() {
        let rt = runtime();
        assert_eq!(
            rt.invoke_command(":pane-close"),
            CommandDispatch::Invoked(vec![UiAction::CloseFocusedPane])
        );
    }

    #[test]
    fn stock_leader_keys_are_leader_scoped() {
        let rt = runtime();
        let cases: [(&[u8], Vec<UiAction>); 7] = [
            (b"|", vec![UiAction::ExitMode, UiAction::SplitRight]),
            (b"-", vec![UiAction::ExitMode, UiAction::SplitDown]),
            (
                b"h",
                vec![
                    UiAction::ExitMode,
                    UiAction::FocusPaneDirection {
                        direction: PaneDirection::Left,
                    },
                ],
            ),
            (
                b"j",
                vec![
                    UiAction::ExitMode,
                    UiAction::FocusPaneDirection {
                        direction: PaneDirection::Down,
                    },
                ],
            ),
            (
                b"k",
                vec![
                    UiAction::ExitMode,
                    UiAction::FocusPaneDirection {
                        direction: PaneDirection::Up,
                    },
                ],
            ),
            (
                b"l",
                vec![
                    UiAction::ExitMode,
                    UiAction::FocusPaneDirection {
                        direction: PaneDirection::Right,
                    },
                ],
            ),
            (b"x", vec![UiAction::ExitMode, UiAction::CloseFocusedPane]),
        ];
        for (key, expected) in cases {
            let spec = rt
                .match_keybinding(key, Some(LEADER_MODE))
                .unwrap_or_else(|| panic!("{key:?} bound in leader"));
            assert_eq!((spec.handler)(&snapshot()), expected, "key {key:?}");
            // Pane keys never fire in normal mode.
            assert!(rt.match_keybinding(key, None).is_none());
        }
    }

    #[test]
    fn config_overrides_pane_leader_keys() {
        let config: Config = toml::from_str(
            "[keybinds]\n\"leader.split_right\" = \"v\"\n\"leader.pane_close\" = \"q\"\n",
        )
        .unwrap();
        let rt = RuntimeBuilder::new()
            .register_extension(PanesExtension::new(&config))
            .build()
            .unwrap();
        assert!(rt.match_keybinding(b"|", Some(LEADER_MODE)).is_none());
        assert!(rt.match_keybinding(b"v", Some(LEADER_MODE)).is_some());
        assert!(rt.match_keybinding(b"x", Some(LEADER_MODE)).is_none());
        assert!(rt.match_keybinding(b"q", Some(LEADER_MODE)).is_some());
    }

    fn snapshot() -> ekko_ext::ClientSnapshot {
        ekko_ext::ClientSnapshot {
            session_name: "s".into(),
            mode: ekko_ext::ClientSnapshot::NORMAL_MODE.into(),
            cols: 80,
            rows: 24,
            grid_cols: 80,
            grid_rows: 24,
            scrollback: 0,
            panes: vec![],
            focused_pane: None,
            projects: vec![],
            status_note: None,
            keybindings: vec![],
            now_ms: 0,
            hidden_surfaces: Vec::new(),
            theme: ekko_ext::ThemePalette::fallback(),
        }
    }
}
