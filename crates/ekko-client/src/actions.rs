//! The `UiAction` interpreter — the client's single write path — plus the
//! scrollback editor takeover. Extracted from `event_loop.rs`: the loop
//! file stays the *event router* (stdin/socket/resize in, dispatches out);
//! this file is what returned actions *do*. Every `UiAction` an extension
//! can return resolves here and nowhere else, so the extension surface is
//! auditable end to end in one module.

use std::time::Duration;

use ekko_err::Result;
use ekko_ext::{EventKind, EventPayload, NoteKind, UiAction};
use ekko_proto::ClientToServer;

use crate::ClientOutcome;
use crate::event_loop::{App, MAX_ACTION_DEPTH, STATUS_NOTE_TTL};
use crate::spawn;

impl App<'_> {
    pub(crate) fn open_scrollback_in_editor(&mut self, text: &str) {
        let editor = std::env::var("VISUAL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| std::env::var("EDITOR").ok())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "vi".to_string());
        let path = std::env::temp_dir().join(format!(
            "ekko-scrollback-{}-{}.txt",
            self.state.session_name,
            std::process::id()
        ));
        let outcome = std::fs::write(&path, text).map_err(|e| e.to_string());
        let status = outcome.and_then(|_| {
            let mut parts = editor.split_whitespace();
            let program = parts.next().unwrap_or("vi");
            let args: Vec<&str> = parts.collect();
            self.raw_guard.with_suspended(|| {
                std::process::Command::new(program)
                    .args(args)
                    .arg(&path)
                    .status()
                    .map_err(|e| e.to_string())
            })
        });
        let _ = std::fs::remove_file(&path);
        // The editor scribbled over the tty: repaint everything.
        self.renderer.invalidate();
        self.state.dirty = true;
        match status {
            Ok(exit) if exit.success() => {}
            Ok(exit) => self.state.set_note(
                format!("{editor} exited with {exit}"),
                NoteKind::Error,
                STATUS_NOTE_TTL,
            ),
            Err(error) => self.state.set_note(
                format!("editor failed: {error}"),
                NoteKind::Error,
                STATUS_NOTE_TTL,
            ),
        }
    }

    // ── UiAction interpreter: the single write path ─────────────────────────

    pub(crate) fn apply_ui_actions(
        &mut self,
        actions: Vec<UiAction>,
        depth: u8,
    ) -> Result<Option<ClientOutcome>> {
        if depth > MAX_ACTION_DEPTH {
            self.state.set_note(
                "action recursion limit reached",
                NoteKind::Error,
                STATUS_NOTE_TTL,
            );
            return Ok(None);
        }
        let mut outcome = None;
        for action in actions {
            if let Some(value) = self.apply_ui_action(action, depth)? {
                outcome.get_or_insert(value);
            }
        }
        Ok(outcome)
    }

    pub(crate) fn apply_ui_action(
        &mut self,
        action: UiAction,
        depth: u8,
    ) -> Result<Option<ClientOutcome>> {
        match action {
            UiAction::Detach => {
                if let Some(reason) = self.runtime.dispatch_cancelable(
                    EventKind::BeforeSessionDetach,
                    EventPayload::BeforeSessionDetach {
                        session_name: self.state.session_name.clone(),
                    },
                ) {
                    self.state.set_note(
                        format!("detach canceled: {reason}"),
                        NoteKind::Error,
                        STATUS_NOTE_TTL,
                    );
                    return Ok(None);
                }
                let _ = self.send_message(ClientToServer::Detach);
                self.runtime
                    .dispatch(EventKind::SessionDetached, EventPayload::Empty);
                Ok(Some(ClientOutcome::Exited))
            }
            UiAction::Quit => Ok(Some(ClientOutcome::Exited)),
            UiAction::NewSession { name } => {
                let name = name.unwrap_or_else(|| crate::next_session_name(self.runtime));
                match spawn::spawn_daemon(&name) {
                    Ok(()) => Ok(Some(ClientOutcome::SwitchTo {
                        name,
                        mode: self.mode_carry(),
                    })),
                    Err(err) => {
                        self.state.set_note(
                            format!("failed to spawn session: {err}"),
                            NoteKind::Error,
                            STATUS_NOTE_TTL,
                        );
                        Ok(None)
                    }
                }
            }
            UiAction::SwitchSession { name } => {
                if name == self.state.session_name {
                    return Ok(None);
                }
                if let Some(reason) = self.runtime.dispatch_cancelable(
                    EventKind::BeforeSessionSwitch,
                    EventPayload::SessionSwitch {
                        from: self.state.session_name.clone(),
                        to: name.clone(),
                    },
                ) {
                    self.state.set_note(
                        format!("switch canceled: {reason}"),
                        NoteKind::Error,
                        STATUS_NOTE_TTL,
                    );
                    return Ok(None);
                }
                self.runtime.dispatch(
                    EventKind::SessionSwitched,
                    EventPayload::SessionSwitch {
                        from: self.state.session_name.clone(),
                        to: name.clone(),
                    },
                );
                Ok(Some(ClientOutcome::SwitchTo {
                    name,
                    mode: self.mode_carry(),
                }))
            }
            UiAction::KillCurrentSession => {
                // Never fire-and-forget silently: if the request can't even
                // reach the daemon the session survives, and the user must
                // know instead of finding a ghost in the sidebar later.
                if let Err(error) = self.send_message(ClientToServer::KillCurrentSession) {
                    self.state.set_note(
                        format!("kill request failed: {error}"),
                        NoteKind::Error,
                        STATUS_NOTE_TTL,
                    );
                }
                Ok(None)
            }
            UiAction::EnterMode { name } => {
                self.enter_mode(&name);
                Ok(None)
            }
            UiAction::ExitMode => {
                self.exit_mode();
                Ok(None)
            }
            UiAction::OpenOverlay { name } => {
                self.open_overlay(&name);
                Ok(None)
            }
            UiAction::CloseOverlay => {
                self.state.overlay = None;
                Ok(None)
            }
            UiAction::SetStatusNote { text, kind, ttl_ms } => {
                self.state
                    .set_note(text, kind, Duration::from_millis(ttl_ms));
                Ok(None)
            }
            UiAction::ForwardKey { bytes } => {
                self.send_message(ClientToServer::Key(bytes))?;
                Ok(None)
            }
            UiAction::Scroll { delta } => {
                self.send_message(ClientToServer::Scroll { delta })?;
                Ok(None)
            }
            UiAction::ScrollToBottom => {
                self.send_message(ClientToServer::ScrollReset)?;
                Ok(None)
            }
            UiAction::SearchScrollback { query } => {
                self.send_message(ClientToServer::SearchScrollback { query })?;
                Ok(None)
            }
            UiAction::SearchMatchJump { forward } => {
                if let Some(search) = &mut self.state.search {
                    let len = search.matches.len();
                    if len > 0 {
                        search.current = if forward {
                            (search.current + 1) % len
                        } else {
                            (search.current + len - 1) % len
                        };
                        self.scroll_to_current_match();
                        self.state.dirty = true;
                    }
                }
                Ok(None)
            }
            UiAction::SearchClear => {
                self.state.search = None;
                self.state.dirty = true;
                Ok(None)
            }
            UiAction::EditScrollback => {
                self.send_message(ClientToServer::DumpScrollback)?;
                Ok(None)
            }
            UiAction::SplitRight => {
                self.send_message(ClientToServer::SplitPane {
                    direction: ekko_proto::SplitDirection::Right,
                })?;
                Ok(None)
            }
            UiAction::SplitDown => {
                self.send_message(ClientToServer::SplitPane {
                    direction: ekko_proto::SplitDirection::Down,
                })?;
                Ok(None)
            }
            UiAction::FocusPaneDirection { direction } => {
                let direction = match direction {
                    ekko_ext::PaneDirection::Left => ekko_proto::Direction::Left,
                    ekko_ext::PaneDirection::Right => ekko_proto::Direction::Right,
                    ekko_ext::PaneDirection::Up => ekko_proto::Direction::Up,
                    ekko_ext::PaneDirection::Down => ekko_proto::Direction::Down,
                };
                self.send_message(ClientToServer::FocusDirection { direction })?;
                Ok(None)
            }
            UiAction::CloseFocusedPane => {
                self.send_message(ClientToServer::CloseFocusedPane)?;
                Ok(None)
            }
            UiAction::ToggleSurface { name } => {
                if self.runtime.surface(&name).is_none() {
                    self.state.set_note(
                        format!("unknown surface: {name}"),
                        NoteKind::Error,
                        STATUS_NOTE_TTL,
                    );
                    return Ok(None);
                }
                if !self.state.hidden_surfaces.remove(&name) {
                    self.state.hidden_surfaces.insert(name);
                }
                // The terminal pane grows/shrinks with the toggle; the next
                // loop pass resizes the session grid via
                // `apply_resize_if_changed`.
                self.state.dirty = true;
                Ok(None)
            }
            UiAction::InvokeCommand { line } => {
                use ekko_ext::CommandDispatch;
                match self.runtime.invoke_command(&line) {
                    CommandDispatch::Empty => Ok(None),
                    CommandDispatch::NotFound(line) => {
                        self.state.set_note(
                            format!("unknown command: {line}"),
                            NoteKind::Error,
                            STATUS_NOTE_TTL,
                        );
                        Ok(None)
                    }
                    CommandDispatch::Canceled(reason) => {
                        self.state.set_note(
                            format!("command canceled: {reason}"),
                            NoteKind::Error,
                            STATUS_NOTE_TTL,
                        );
                        Ok(None)
                    }
                    CommandDispatch::Failed(message) => {
                        self.state
                            .set_note(message, NoteKind::Error, STATUS_NOTE_TTL);
                        Ok(None)
                    }
                    CommandDispatch::Invoked(actions) => self.apply_ui_actions(actions, depth + 1),
                }
            }
            UiAction::Custom { name, payload } => {
                match self.runtime.resolve_custom_action(&name, &payload) {
                    Ok(Some(actions)) => self.apply_ui_actions(actions, depth + 1),
                    Ok(None) => {
                        self.state.set_note(
                            format!("unknown custom action: {name}"),
                            NoteKind::Error,
                            STATUS_NOTE_TTL,
                        );
                        Ok(None)
                    }
                    Err(error) => {
                        self.state.set_note(
                            format!("custom action '{name}' failed: {error:#}"),
                            NoteKind::Error,
                            STATUS_NOTE_TTL,
                        );
                        Ok(None)
                    }
                }
            }
        }
    }
}
