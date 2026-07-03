//! Command mode (`:`-line editing) and the stock command set, both
//! registered through the public API. The core only provides the
//! mechanisms: mode routing, the command registry, and `apply_ui_action`.

use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{
    ClientSnapshot, CommandInvocation, CommandOutput, CommandSpec, DrawContext, Extension,
    ExtensionHost, ExtensionManifest, ModeOutcome, ModeSpec, ModeState, NoteKind, OVERLAY_HELP,
    Rect, UiAction,
};
use ekko_tui::display_cell_width;

pub const COMMAND_MODE: &str = "command";

pub struct CommandModeExtension;

impl Extension for CommandModeExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.command-mode".into(),
            name: "command mode".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: ":command line editor and stock commands".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.register_mode(ModeSpec {
            name: COMMAND_MODE.into(),
            init_state: Arc::new(|| Box::new(String::new()) as ModeState),
            on_key: Arc::new(handle_key),
            render: Some(Arc::new(render_command_line)),
        })?;

        host.register_command(CommandSpec {
            name: "detach".into(),
            aliases: vec!["q".into(), "quit".into()],
            description: "detach from the session".into(),
            args_hint: String::new(),
            handler: Arc::new(|_| Ok(CommandOutput::action(UiAction::Detach))),
        })?;
        host.register_command(CommandSpec {
            name: "new".into(),
            aliases: vec![],
            description: "create and switch to a session".into(),
            args_hint: "[name]".into(),
            handler: Arc::new(|inv: CommandInvocation| {
                let name = inv.raw_args.split_whitespace().next().map(str::to_string);
                Ok(CommandOutput::action(UiAction::NewSession { name }))
            }),
        })?;
        host.register_command(CommandSpec {
            name: "switch".into(),
            aliases: vec![],
            description: "switch to a session".into(),
            args_hint: "<name>".into(),
            handler: Arc::new(|inv: CommandInvocation| {
                match inv.raw_args.split_whitespace().next() {
                    Some(name) => Ok(CommandOutput::action(UiAction::SwitchSession {
                        name: name.to_string(),
                    })),
                    None => Ok(CommandOutput::note("usage: switch <name>", NoteKind::Error)),
                }
            }),
        })?;
        host.register_command(CommandSpec {
            name: "kill".into(),
            aliases: vec![],
            description: "kill the current session".into(),
            args_hint: String::new(),
            handler: Arc::new(|_| Ok(CommandOutput::action(UiAction::KillCurrentSession))),
        })?;
        host.register_command(CommandSpec {
            name: "help".into(),
            aliases: vec![],
            description: "show keybinding and command help".into(),
            args_hint: String::new(),
            handler: Arc::new(|_| {
                Ok(CommandOutput::action(UiAction::OpenOverlay {
                    name: OVERLAY_HELP.into(),
                }))
            }),
        })?;
        Ok(())
    }
}

fn buffer(state: &ModeState) -> &String {
    state
        .downcast_ref::<String>()
        .expect("command mode state is a String")
}

fn buffer_mut(state: &mut ModeState) -> &mut String {
    state
        .downcast_mut::<String>()
        .expect("command mode state is a String")
}

fn handle_key(state: &mut ModeState, bytes: &[u8], _snapshot: &ClientSnapshot) -> ModeOutcome {
    match bytes {
        b"\x1b" => ModeOutcome::Exit,
        b"\r" | b"\n" => {
            let line = std::mem::take(buffer_mut(state));
            ModeOutcome::ExitWith(vec![UiAction::InvokeCommand { line }])
        }
        b"\x7f" | b"\x08" => {
            buffer_mut(state).pop();
            ModeOutcome::Continue
        }
        // Escape sequences (arrows, alt chords) and stray control bytes are
        // not line text; swallow them instead of polluting the buffer.
        [0x1b, ..] => ModeOutcome::Continue,
        _ => {
            if let Ok(text) = std::str::from_utf8(bytes) {
                buffer_mut(state).extend(text.chars().filter(|c| !c.is_control()));
            }
            ModeOutcome::Continue
        }
    }
}

/// Draw the `:command` line over the bottom row of the frame; returns the
/// hardware cursor position at the end of the buffer.
fn render_command_line(
    ctx: &mut dyn DrawContext,
    state: &ModeState,
    snapshot: &ClientSnapshot,
) -> Option<(i32, i32)> {
    let (cols, rows) = ctx.size();
    if cols <= 0 || rows <= 0 {
        return None;
    }
    let theme = &snapshot.theme;
    let row = rows - 1;
    let bg = theme.warning;
    let fg = theme.status_fg;
    ctx.fill_rect(Rect::new(0, row, cols, 1), fg, bg);
    let text = format!(":{}", buffer(state));
    ctx.put_text(0, row, cols, fg, bg, &text);
    Some((display_cell_width(&text) as i32, row))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_ext::{CommandDispatch, RuntimeBuilder, ThemePalette};

    fn runtime() -> ekko_ext::AppRuntime {
        RuntimeBuilder::new()
            .register_extension(CommandModeExtension)
            .build()
            .unwrap()
    }

    fn snapshot() -> ClientSnapshot {
        ClientSnapshot {
            session_name: "s".into(),
            mode: COMMAND_MODE.into(),
            cols: 80,
            rows: 24,
            grid_cols: 80,
            grid_rows: 23,
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
    fn parses_quit_and_detach_aliases() {
        let rt = runtime();
        for line in [":q", "quit", ":detach"] {
            assert_eq!(
                rt.invoke_command(line),
                CommandDispatch::Invoked(vec![UiAction::Detach]),
                "line {line:?}"
            );
        }
    }

    #[test]
    fn parses_new_with_optional_name() {
        let rt = runtime();
        assert_eq!(
            rt.invoke_command(":new"),
            CommandDispatch::Invoked(vec![UiAction::NewSession { name: None }])
        );
        assert_eq!(
            rt.invoke_command(":new work"),
            CommandDispatch::Invoked(vec![UiAction::NewSession {
                name: Some("work".into())
            }])
        );
    }

    #[test]
    fn parses_switch_requiring_a_name() {
        let rt = runtime();
        assert_eq!(
            rt.invoke_command(":switch main"),
            CommandDispatch::Invoked(vec![UiAction::SwitchSession {
                name: "main".into()
            }])
        );
        assert!(matches!(
            rt.invoke_command(":switch"),
            CommandDispatch::Invoked(actions)
                if matches!(actions.as_slice(), [UiAction::SetStatusNote { kind: NoteKind::Error, .. }])
        ));
    }

    #[test]
    fn parses_kill_and_help() {
        let rt = runtime();
        assert_eq!(
            rt.invoke_command(":kill"),
            CommandDispatch::Invoked(vec![UiAction::KillCurrentSession])
        );
        assert_eq!(
            rt.invoke_command(":help"),
            CommandDispatch::Invoked(vec![UiAction::OpenOverlay {
                name: OVERLAY_HELP.into()
            }])
        );
    }

    #[test]
    fn empty_line_yields_empty_dispatch() {
        let rt = runtime();
        assert_eq!(rt.invoke_command(""), CommandDispatch::Empty);
        assert_eq!(rt.invoke_command(":"), CommandDispatch::Empty);
        assert_eq!(rt.invoke_command("   "), CommandDispatch::Empty);
    }

    #[test]
    fn unrecognized_command_is_not_found() {
        let rt = runtime();
        assert_eq!(
            rt.invoke_command(":bogus"),
            CommandDispatch::NotFound("bogus".into())
        );
    }

    #[test]
    fn mode_edits_buffer_and_submits_on_enter() {
        let rt = runtime();
        let spec = rt.mode(COMMAND_MODE).unwrap();
        let mut state = (spec.init_state)();
        let snap = snapshot();
        assert_eq!(
            (spec.on_key)(&mut state, b"ne", &snap),
            ModeOutcome::Continue
        );
        assert_eq!(
            (spec.on_key)(&mut state, b"ww", &snap),
            ModeOutcome::Continue
        );
        assert_eq!(
            (spec.on_key)(&mut state, b"\x7f", &snap),
            ModeOutcome::Continue
        );
        assert_eq!(
            (spec.on_key)(&mut state, b"\r", &snap),
            ModeOutcome::ExitWith(vec![UiAction::InvokeCommand { line: "new".into() }])
        );
    }

    #[test]
    fn mode_escape_cancels() {
        let rt = runtime();
        let spec = rt.mode(COMMAND_MODE).unwrap();
        let mut state = (spec.init_state)();
        assert_eq!(
            (spec.on_key)(&mut state, b"\x1b", &snapshot()),
            ModeOutcome::Exit
        );
    }
}
