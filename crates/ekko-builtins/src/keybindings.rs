//! The stock chord set, registered through the public API. Binding strings
//! come from config (`[keybinds]`) with the historical defaults; the
//! *meaning* of each action — including session/project navigation order —
//! is policy that lives here, computed from the snapshot.
//!
//! Navigation lives on the alt layer (zellij-style): alt+char arrives as an
//! ESC-prefixed pair, so nothing collides with the C0 control bytes that
//! shells and TUIs depend on (ctrl+h/j/k/l are backspace, newline,
//! kill-line, clear-screen). `ctrl+q` (detach) is the only control-byte
//! default.

use std::sync::Arc;

use anyhow::Result;
use ekko_config::Config;
use ekko_ext::{
    ClientSnapshot, Extension, ExtensionHost, ExtensionManifest, KeybindingSpec, NoteKind,
    OVERLAY_HELP, UiAction, parse_key_chords,
};

use crate::command_mode::COMMAND_MODE;
use crate::rows;

const NOOP_NOTE_TTL_MS: u64 = 2000;

pub struct KeybindingsExtension {
    bindings: Vec<(String, Vec<String>)>,
}

impl KeybindingsExtension {
    pub fn new(config: &Config) -> Self {
        let defaults: &[(&str, &[&str])] = &[
            ("detach", &["ctrl+q"]),
            ("new_session", &["alt+n"]),
            ("kill_session", &["alt+x"]),
            ("session_next", &["alt+j", "alt+down"]),
            ("session_prev", &["alt+k", "alt+up"]),
            ("project_prev", &["alt+h", "alt+left"]),
            ("project_next", &["alt+l", "alt+right"]),
            ("command_mode", &["alt+e"]),
            ("scroll_mode", &["alt+s"]),
            ("help", &["alt+/"]),
        ];
        Self {
            bindings: defaults
                .iter()
                .map(|(action, default)| (action.to_string(), config.bindings_for(action, default)))
                .collect(),
        }
    }

    fn binding_strings(&self, action: &str) -> &[String] {
        self.bindings
            .iter()
            .find(|(name, _)| name == action)
            .map(|(_, strings)| strings.as_slice())
            .unwrap_or(&[])
    }
}

type Handler = Arc<dyn Fn(&ClientSnapshot) -> Vec<UiAction> + Send + Sync>;

impl Extension for KeybindingsExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.keybindings".into(),
            name: "keybindings".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "stock chords: session/project navigation, new/kill session, detach, command mode, help".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        let entries: Vec<(&str, &str, Handler)> = vec![
            (
                "session_next",
                "next session",
                Arc::new(|snapshot: &ClientSnapshot| {
                    switch_or_note(
                        rows::next_session_name(&snapshot.projects, &snapshot.session_name),
                        "no other session",
                    )
                }),
            ),
            (
                "session_prev",
                "prev session",
                Arc::new(|snapshot: &ClientSnapshot| {
                    switch_or_note(
                        rows::prev_session_name(&snapshot.projects, &snapshot.session_name),
                        "no other session",
                    )
                }),
            ),
            (
                "project_prev",
                "prev project",
                Arc::new(|snapshot: &ClientSnapshot| {
                    switch_or_note(
                        rows::adjacent_project_first_session(
                            &snapshot.projects,
                            &snapshot.session_name,
                            false,
                        ),
                        "no other project",
                    )
                }),
            ),
            (
                "project_next",
                "next project",
                Arc::new(|snapshot: &ClientSnapshot| {
                    switch_or_note(
                        rows::adjacent_project_first_session(
                            &snapshot.projects,
                            &snapshot.session_name,
                            true,
                        ),
                        "no other project",
                    )
                }),
            ),
            (
                "new_session",
                "new session",
                Arc::new(|_: &ClientSnapshot| vec![UiAction::NewSession { name: None }]),
            ),
            (
                "kill_session",
                "kill session",
                Arc::new(|snapshot: &ClientSnapshot| {
                    let mut actions = vec![UiAction::KillCurrentSession];
                    // Land on a neighbor instead of exiting with the corpse.
                    if let Some(next) =
                        rows::next_session_name(&snapshot.projects, &snapshot.session_name)
                    {
                        actions.push(UiAction::SwitchSession { name: next });
                    }
                    actions
                }),
            ),
            (
                "detach",
                "detach",
                Arc::new(|_: &ClientSnapshot| vec![UiAction::Detach]),
            ),
            (
                "command_mode",
                "command mode",
                Arc::new(|_: &ClientSnapshot| {
                    vec![UiAction::EnterMode {
                        name: COMMAND_MODE.into(),
                    }]
                }),
            ),
            (
                "scroll_mode",
                "scroll",
                Arc::new(|_: &ClientSnapshot| {
                    vec![UiAction::EnterMode {
                        name: crate::scroll_mode::SCROLL_MODE.into(),
                    }]
                }),
            ),
            (
                "help",
                "help",
                Arc::new(|_: &ClientSnapshot| {
                    vec![UiAction::OpenOverlay {
                        name: OVERLAY_HELP.into(),
                    }]
                }),
            ),
        ];

        for (action, description, handler) in entries {
            let strings = self.binding_strings(action);
            let chords: Vec<Vec<u8>> = strings
                .iter()
                .filter_map(|s| parse_key_chords(s))
                .flatten()
                .collect();
            if chords.is_empty() {
                continue;
            }
            host.register_keybinding(KeybindingSpec {
                chords,
                chord_text: strings.join(" / "),
                mode: None,
                description: description.to_string(),
                handler,
            })?;
        }
        Ok(())
    }
}

fn switch_or_note(name: Option<String>, note: &str) -> Vec<UiAction> {
    match name {
        Some(name) => vec![UiAction::SwitchSession { name }],
        None => vec![UiAction::SetStatusNote {
            text: note.to_string(),
            kind: NoteKind::Info,
            ttl_ms: NOOP_NOTE_TTL_MS,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_ext::{RuntimeBuilder, ThemePalette};

    fn runtime() -> ekko_ext::AppRuntime {
        RuntimeBuilder::new()
            .register_extension(KeybindingsExtension::new(&Config::default()))
            .build()
            .unwrap()
    }

    fn snapshot() -> ClientSnapshot {
        ClientSnapshot {
            session_name: "s1".into(),
            mode: ClientSnapshot::NORMAL_MODE.into(),
            cols: 80,
            rows: 24,
            grid_cols: 80,
            grid_rows: 23,
            scrollback: 0,
            projects: vec![
                crate::rows::test_group("a", &["s1", "s2"]),
                crate::rows::test_group("b", &["s3"]),
            ],
            status_note: None,
            keybindings: vec![],
            now_ms: 0,
            theme: ThemePalette::fallback(),
        }
    }

    fn expect(
        rt: &ekko_ext::AppRuntime,
        snap: &ClientSnapshot,
        bytes: &[u8],
        actions: Vec<UiAction>,
    ) {
        let spec = rt.match_keybinding(bytes, None).expect("chord bound");
        assert_eq!((spec.handler)(snap), actions);
    }

    #[test]
    fn default_chords_match_expected_bytes() {
        let rt = runtime();
        let snap = snapshot();
        expect(&rt, &snap, &[0x11], vec![UiAction::Detach]); // ctrl+q
        expect(
            &rt,
            &snap,
            b"\x1bn",
            vec![UiAction::NewSession { name: None }],
        );
        expect(
            &rt,
            &snap,
            b"\x1bj",
            vec![UiAction::SwitchSession { name: "s2".into() }],
        );
        expect(
            &rt,
            &snap,
            b"\x1bk",
            vec![UiAction::SwitchSession { name: "s3".into() }], // wraps back
        );
        expect(
            &rt,
            &snap,
            b"\x1bh",
            vec![UiAction::SwitchSession { name: "s3".into() }], // prev project
        );
        expect(
            &rt,
            &snap,
            b"\x1bl",
            vec![UiAction::SwitchSession { name: "s3".into() }], // next project
        );
        expect(
            &rt,
            &snap,
            b"\x1be",
            vec![UiAction::EnterMode {
                name: COMMAND_MODE.into(),
            }],
        );
        expect(
            &rt,
            &snap,
            b"\x1b/",
            vec![UiAction::OpenOverlay {
                name: OVERLAY_HELP.into(),
            }],
        );
    }

    #[test]
    fn arrow_variants_mirror_the_letter_chords() {
        let rt = runtime();
        let snap = snapshot();
        // alt+down (CSI 1;3B) navigates like alt+j.
        expect(
            &rt,
            &snap,
            b"\x1b[1;3B",
            vec![UiAction::SwitchSession { name: "s2".into() }],
        );
        expect(
            &rt,
            &snap,
            b"\x1b[1;3D",
            vec![UiAction::SwitchSession { name: "s3".into() }],
        );
    }

    #[test]
    fn kill_switches_to_a_neighbor_when_one_exists() {
        let rt = runtime();
        let snap = snapshot();
        expect(
            &rt,
            &snap,
            b"\x1bx",
            vec![
                UiAction::KillCurrentSession,
                UiAction::SwitchSession { name: "s2".into() },
            ],
        );
    }

    #[test]
    fn kill_of_last_session_only_kills() {
        let rt = runtime();
        let mut snap = snapshot();
        snap.projects = vec![crate::rows::test_group("a", &["s1"])];
        expect(&rt, &snap, b"\x1bx", vec![UiAction::KillCurrentSession]);
    }

    #[test]
    fn navigation_noops_surface_a_note() {
        let rt = runtime();
        let mut snap = snapshot();
        snap.projects = vec![crate::rows::test_group("a", &["s1"])];
        expect(
            &rt,
            &snap,
            b"\x1bj",
            vec![UiAction::SetStatusNote {
                text: "no other session".into(),
                kind: NoteKind::Info,
                ttl_ms: NOOP_NOTE_TTL_MS,
            }],
        );
        expect(
            &rt,
            &snap,
            b"\x1bl",
            vec![UiAction::SetStatusNote {
                text: "no other project".into(),
                kind: NoteKind::Info,
                ttl_ms: NOOP_NOTE_TTL_MS,
            }],
        );
    }

    #[test]
    fn control_nav_bytes_fall_through_to_the_pty() {
        let rt = runtime();
        // The old ctrl+h/j/k/l defaults must NOT be bound: those bytes are
        // backspace/newline/kill-line/clear-screen inside the terminal.
        for byte in [0x08u8, 0x0a, 0x0b, 0x0c, 0x0e, 0x05] {
            assert!(
                rt.match_keybinding(&[byte], None).is_none(),
                "byte {byte:#x}"
            );
        }
    }

    #[test]
    fn chord_does_not_match_multi_byte_input() {
        let rt = runtime();
        assert!(rt.match_keybinding(b"ab", None).is_none());
        assert!(rt.match_keybinding(b"\x1b[A", None).is_none());
    }

    #[test]
    fn colon_is_never_a_chord() {
        let rt = runtime();
        assert!(rt.match_keybinding(b":", None).is_none());
    }

    #[test]
    fn chords_do_not_match_inside_other_modes() {
        let rt = runtime();
        assert!(rt.match_keybinding(&[0x11], Some(COMMAND_MODE)).is_none());
    }

    #[test]
    fn config_overrides_replace_defaults() {
        let config: Config = toml::from_str("[keybinds]\ndetach = \"ctrl+d\"\n").unwrap();
        let rt = RuntimeBuilder::new()
            .register_extension(KeybindingsExtension::new(&config))
            .build()
            .unwrap();
        assert!(rt.match_keybinding(&[0x11], None).is_none());
        assert!(rt.match_keybinding(&[0x04], None).is_some());
    }

    #[test]
    fn config_can_bind_arrows_with_all_encodings() {
        let config: Config = toml::from_str("[keybinds]\nsession_next = \"down\"\n").unwrap();
        let rt = RuntimeBuilder::new()
            .register_extension(KeybindingsExtension::new(&config))
            .build()
            .unwrap();
        assert!(rt.match_keybinding(b"\x1b[B", None).is_some());
        assert!(rt.match_keybinding(b"\x1bOB", None).is_some());
    }
}
