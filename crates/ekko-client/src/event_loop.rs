//! Threads + main loop: stdin reader, socket reader, SIGWINCH watcher, and
//! the render/dispatch loop that ties them together. This is the host side
//! of the extension system: it builds snapshots, splits raw reads into key
//! tokens, routes each token through the registries (overlay -> paste ->
//! KeyInput gate -> mode -> mouse -> keybinding -> PTY), and interprets
//! returned `UiAction`s in exactly one place (`apply_ui_action`). It holds
//! no feature policy of its own.

use std::io::{Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use ekko_ext::{
    AppRuntime, ClientSnapshot, EventKind, EventPayload, EventReturn, KeyIntercept, ModeOutcome,
    NoteKind, OverlayOutcome, ResolvedLayout, ThemePalette, UiAction, fallback_group,
    resolve_layout,
};
use ekko_grid::ansi::AnsiRenderer;
use ekko_grid::cell_surface::CellSurface;
use ekko_proto::{ClientToServer, ExitReason, ServerToClient};
use ekko_tui::terminal_size;

use crate::ClientOutcome;
use crate::clipboard;
use crate::drawctx::{gc, to_cell_rect};
use crate::input::{PasteAccumulator, PasteFeed};
use crate::scene;
use crate::sessions;
use crate::spawn;
use crate::state::{ActiveOverlay, ClientState};

const SESSION_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);
const ANIMATION_INTERVAL: Duration = Duration::from_millis(80);
const IDLE_INTERVAL: Duration = Duration::from_secs(1);
const STATUS_NOTE_TTL: Duration = Duration::from_secs(4);
/// Bound on `InvokeCommand` -> command -> `InvokeCommand` chains so a
/// misbehaving extension can't recurse the action interpreter forever.
const MAX_ACTION_DEPTH: u8 = 4;

/// Events multiplexed onto the client's single input channel. The channel
/// (and the stdin/resize producer threads) outlives any one server
/// connection, so socket-sourced events carry the connection generation
/// that produced them: after a session switch, a straggling event from the
/// torn-down connection must not be mistaken for the live one.
pub(crate) enum Event {
    Stdin(Vec<u8>),
    Server(u64, ServerToClient),
    ServerClosed(u64),
    Resize,
}

/// Run the client loop for one server connection, reading events from the
/// process-wide channel. `generation` identifies this connection's socket
/// reader; `resume_mode` re-enters the mode that was active when a session
/// switch tore down the previous loop (a sticky leader map keeps its panel
/// across the reattach).
pub(crate) fn run_event_loop(
    send: Box<dyn Write + Send>,
    rx: &mpsc::Receiver<Event>,
    session_name: String,
    runtime: &AppRuntime,
    resume_mode: Option<String>,
    generation: u64,
) -> Result<ClientOutcome> {
    let palette = runtime
        .resolve_theme(None)
        .map(|theme| theme.palette)
        .unwrap_or_else(ThemePalette::fallback);

    let mut app = App {
        send,
        state: ClientState::new(session_name.clone()),
        paste: PasteAccumulator::default(),
        palette,
        surface: CellSurface::new(1, 1, gc(palette.term_fg), gc(palette.term_bg)),
        renderer: AnsiRenderer::default(),
        runtime,
        generation,
        last_size: (0, 0),
        last_sent_grid: (0, 0),
        last_session_refresh: Instant::now() - SESSION_REFRESH_INTERVAL,
        last_cursor_shape: 0,
        render_buf: Vec::with_capacity(64 * 1024),
    };
    app.refresh_sessions();
    app.runtime.dispatch(
        EventKind::SessionAttached,
        EventPayload::SessionAttached {
            session_name,
            wire_version: ekko_proto::WIRE_VERSION,
        },
    );
    app.runtime
        .dispatch(EventKind::ClientReady, EventPayload::Empty);
    if let Some(mode) = resume_mode {
        app.enter_mode(&mode);
        app.state.dirty = true;
    }

    let outcome = app.run(rx)?;
    Ok(outcome)
}

pub(crate) fn spawn_stdin_reader(tx: mpsc::Sender<Event>) {
    thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(Event::Stdin(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
}

pub(crate) fn spawn_socket_reader(
    tx: mpsc::Sender<Event>,
    mut recv: Box<dyn Read + Send>,
    generation: u64,
) {
    thread::spawn(move || {
        loop {
            match ekko_proto::read_msg::<_, ServerToClient>(&mut recv) {
                Ok(Some(msg)) => {
                    if tx.send(Event::Server(generation, msg)).is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    let _ = tx.send(Event::ServerClosed(generation));
                    break;
                }
                Err(_) => {
                    let _ = tx.send(Event::ServerClosed(generation));
                    break;
                }
            }
        }
    });
}

pub(crate) fn spawn_resize_watcher(tx: mpsc::Sender<Event>) {
    thread::spawn(move || {
        let Ok(mut signals) = signal_hook::iterator::Signals::new([signal_hook::consts::SIGWINCH])
        else {
            return;
        };
        let mut last_sent = Instant::now() - RESIZE_DEBOUNCE;
        for _ in signals.forever() {
            let now = Instant::now();
            if now.duration_since(last_sent) < RESIZE_DEBOUNCE {
                thread::sleep(RESIZE_DEBOUNCE);
            }
            last_sent = Instant::now();
            if tx.send(Event::Resize).is_err() {
                break;
            }
        }
    });
}

struct App<'a> {
    send: Box<dyn Write + Send>,
    state: ClientState,
    paste: PasteAccumulator,
    palette: ThemePalette,
    surface: CellSurface,
    renderer: AnsiRenderer,
    runtime: &'a AppRuntime,
    /// Connection generation; socket events from other generations are stale.
    generation: u64,
    last_size: (u16, u16),
    /// The grid size last reported to the server: the terminal pane the
    /// layout leaves over, not the raw frame. Attach reports the raw frame
    /// (the runtime doesn't exist yet), so this starts at (0, 0) and the
    /// first loop pass corrects the session size.
    last_sent_grid: (u16, u16),
    last_session_refresh: Instant,
    /// Last DECSCUSR shape pushed to the host terminal (0 = default).
    last_cursor_shape: u8,
    /// Reusable frame buffer: each frame is composed here in full, then
    /// pushed to the terminal with a single locked write + flush instead of
    /// many small locked writes.
    render_buf: Vec<u8>,
}

impl App<'_> {
    fn run(&mut self, rx: &mpsc::Receiver<Event>) -> Result<ClientOutcome> {
        loop {
            if let Some(outcome) = self.drain_pending(rx)? {
                return Ok(outcome);
            }

            self.refresh_if_due();
            self.state.expire_note();
            self.apply_resize_if_changed()?;

            if self.state.dirty {
                self.render()?;
                self.state.dirty = false;
            }

            let animating = self.wants_animation();
            let timeout = if animating {
                ANIMATION_INTERVAL
            } else {
                IDLE_INTERVAL
            };
            match rx.recv_timeout(timeout) {
                Ok(event) => {
                    if let Some(outcome) = self.handle_event(event)? {
                        return Ok(outcome);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if animating {
                        if self.runtime.has_subscribers(EventKind::Tick) {
                            self.runtime.dispatch(
                                EventKind::Tick,
                                EventPayload::Tick {
                                    now_ms: now_millis(),
                                },
                            );
                        }
                        self.state.dirty = true;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(ClientOutcome::Exited),
            }
        }
    }

    fn drain_pending(&mut self, rx: &mpsc::Receiver<Event>) -> Result<Option<ClientOutcome>> {
        loop {
            match rx.try_recv() {
                Ok(event) => {
                    if let Some(outcome) = self.handle_event(event)? {
                        return Ok(Some(outcome));
                    }
                }
                Err(TryRecvError::Empty) => return Ok(None),
                Err(TryRecvError::Disconnected) => return Ok(Some(ClientOutcome::Exited)),
            }
        }
    }

    /// Dispatch one event. Handlers mark `state.dirty` themselves when they
    /// change something visible; a keystroke that merely forwards to the PTY
    /// must not trigger a client-side frame (the redraw comes back as a
    /// `Grid` update).
    fn handle_event(&mut self, event: Event) -> Result<Option<ClientOutcome>> {
        match event {
            Event::Stdin(bytes) => self.handle_stdin(&bytes),
            // Stale socket events from a torn-down connection (the reader
            // thread of the session we just switched away from) are dropped;
            // in particular its `ServerClosed` must not kill the live loop.
            Event::Server(generation, _) | Event::ServerClosed(generation)
                if generation != self.generation =>
            {
                Ok(None)
            }
            Event::Server(_, msg) => Ok(self.handle_server_message(msg)),
            Event::ServerClosed(_) => {
                self.state
                    .set_note("connection lost", NoteKind::Error, STATUS_NOTE_TTL);
                Ok(Some(ClientOutcome::Exited))
            }
            Event::Resize => Ok(None),
        }
    }

    fn refresh_if_due(&mut self) {
        if self.last_session_refresh.elapsed() >= SESSION_REFRESH_INTERVAL {
            self.refresh_sessions();
        }
    }

    fn refresh_sessions(&mut self) {
        let sessions = sessions::scan_sessions();
        self.state.projects = match self.runtime.session_grouper() {
            Some(grouper) => (grouper.group)(sessions),
            None => fallback_group(sessions),
        };
        self.last_session_refresh = Instant::now();
        self.state.dirty = true;
        self.runtime
            .dispatch(EventKind::SessionListRefreshed, EventPayload::Empty);
    }

    /// Whether any registered surface wants the animation tick right now
    /// (or an extension subscribes to `Tick`).
    fn wants_animation(&self) -> bool {
        if self.runtime.has_subscribers(EventKind::Tick) {
            return true;
        }
        // Building a snapshot clones the session list; skip it entirely when
        // no surface registered a tick predicate (the common case).
        if self
            .runtime
            .surfaces()
            .iter()
            .all(|spec| spec.wants_tick.is_none())
        {
            return false;
        }
        let snapshot = self.snapshot();
        self.runtime.visible_surfaces(&snapshot).iter().any(|spec| {
            spec.wants_tick
                .as_ref()
                .is_some_and(|wants| wants(&snapshot))
        })
    }

    fn apply_resize_if_changed(&mut self) -> Result<()> {
        let size = terminal_size();
        if size != self.last_size {
            self.last_size = size;
            self.runtime.dispatch(
                EventKind::Resize,
                EventPayload::Resize {
                    cols: size.0,
                    rows: size.1,
                },
            );
            self.state.dirty = true;
        }
        // The session grid gets the pane the layout leaves over, never the
        // raw frame: chrome (sidebar, statusbar) is client-local and must
        // not count toward the PTY's size. Recomputed every pass, not just
        // on frame changes — surface visibility can change at a fixed frame
        // size and the pane moves with it.
        let pane = self.terminal_pane_size();
        if pane != self.last_sent_grid {
            self.last_sent_grid = pane;
            self.send_message(ClientToServer::Resize {
                cols: pane.0,
                rows: pane.1,
            })?;
            self.state.dirty = true;
        }
        Ok(())
    }

    /// The terminal pane's size under the current layout.
    fn terminal_pane_size(&self) -> (u16, u16) {
        let snapshot = self.snapshot();
        let surfaces = self.runtime.visible_surfaces(&snapshot);
        let layout = resolve_layout(self.last_size.0, self.last_size.1, &surfaces);
        pane_grid_size(&layout)
    }

    fn snapshot(&self) -> ClientSnapshot {
        let (cols, rows) = self.last_size;
        // Snapshot pane fields describe the focused pane (input routing
        // target); the full pane projection lands with the P4 API surface.
        let focused = self.state.workspace.focused_grid();
        ClientSnapshot {
            session_name: self.state.session_name.clone(),
            mode: self.state.mode.clone(),
            cols,
            rows,
            grid_cols: focused.map_or(0, |grid| grid.cols),
            grid_rows: focused.map_or(0, |grid| grid.rows),
            scrollback: focused.map_or(0, |grid| grid.scrollback),
            projects: self.state.projects.clone(),
            status_note: self
                .state
                .status_note
                .as_ref()
                .map(|note| ekko_ext::StatusNote {
                    text: note.text.clone(),
                    kind: note.kind,
                }),
            keybindings: self.runtime.keybinding_infos(),
            now_ms: now_millis(),
            hidden_surfaces: self.state.hidden_surfaces.iter().cloned().collect(),
            theme: self.palette,
        }
    }

    fn render(&mut self) -> Result<()> {
        let (cols, rows) = terminal_size();
        self.last_size = (cols, rows);
        self.surface.resize(
            i32::from(cols),
            i32::from(rows),
            gc(self.palette.term_fg),
            gc(self.palette.term_bg),
        );
        let snapshot = self.snapshot();
        let surfaces = self.runtime.visible_surfaces(&snapshot);
        let layout = resolve_layout(cols, rows, &surfaces);
        let cursor = scene::render_frame(
            &mut self.surface,
            &layout,
            self.runtime,
            &mut self.state,
            &snapshot,
        );
        let hardware_cursor = cursor.map(|(col, row)| ekko_grid::ansi::HardwareCursor { col, row });
        self.render_buf.clear();
        self.renderer.render(
            &mut self.render_buf,
            &mut self.surface,
            to_cell_rect(layout.terminal),
            hardware_cursor,
        )?;
        // Mirror the child's DECSCUSR cursor shape on the host terminal. A
        // mode drawing its own cursor (e.g. the `:` line) gets the default.
        let desired_shape = if self.state.in_normal_mode() {
            self.state
                .workspace
                .focused_grid()
                .and_then(|grid| grid.cursor)
                .map(|c| c.shape)
                .unwrap_or(0)
        } else {
            0
        };
        if desired_shape != self.last_cursor_shape {
            use std::io::Write as _;
            let _ = write!(self.render_buf, "\x1b[{desired_shape} q");
            self.last_cursor_shape = desired_shape;
        }
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&self.render_buf)?;
        stdout.flush()?;
        Ok(())
    }

    fn send_message(&mut self, msg: ClientToServer) -> Result<()> {
        ekko_proto::write_msg(&mut self.send, &msg)?;
        Ok(())
    }

    fn handle_server_message(&mut self, msg: ServerToClient) -> Option<ClientOutcome> {
        match msg {
            ServerToClient::Workspace(update) => {
                // A full snapshot replaces that pane wholesale; drop the
                // renderer's scroll-detection history so unrelated content
                // can't be mistaken for a shift.
                if update
                    .grids
                    .iter()
                    .any(|grid| matches!(grid.update.payload, ekko_proto::GridPayload::Full(_)))
                {
                    self.renderer.invalidate();
                }
                let scrollback_before = self
                    .state
                    .workspace
                    .focused_grid()
                    .map_or(0, |grid| grid.scrollback);
                let selection_on_focused =
                    self.state.selection_pane == Some(self.state.workspace.focused);
                if self.state.workspace.apply(update) {
                    // Drop the highlight with its pane; otherwise keep it
                    // glued to its content as the scrollback view moves.
                    let selection_pane_gone = self
                        .state
                        .selection_pane
                        .is_some_and(|pane| !self.state.workspace.panes.contains_key(&pane));
                    if selection_pane_gone {
                        self.state.selection.clear();
                        self.state.selection_pane = None;
                    } else if selection_on_focused {
                        let scrollback_after = self
                            .state
                            .workspace
                            .focused_grid()
                            .map_or(0, |grid| grid.scrollback);
                        let shift = i64::from(scrollback_after) - i64::from(scrollback_before);
                        if shift != 0 && self.state.selection.is_active() {
                            self.state.selection.shift_rows(shift as i32);
                        }
                    }
                    self.state.dirty = true;
                    if self.runtime.has_subscribers(EventKind::GridUpdated) {
                        let focused = self.state.workspace.focused_grid();
                        self.runtime.dispatch(
                            EventKind::GridUpdated,
                            EventPayload::GridUpdated {
                                epoch: self.state.workspace.epoch,
                                cols: focused.map_or(0, |grid| grid.cols),
                                rows: focused.map_or(0, |grid| grid.rows),
                            },
                        );
                    }
                }
                None
            }
            ServerToClient::Title(title) => {
                // Forward the child's title to the host terminal, minus any
                // control bytes that could smuggle a second escape.
                let clean: String = title.chars().filter(|c| !c.is_control()).collect();
                let mut stdout = std::io::stdout();
                let _ = stdout.write_all(format!("\x1b]2;{clean}\x07").as_bytes());
                let _ = stdout.flush();
                None
            }
            ServerToClient::ClipboardCopy(data) => {
                let mut stdout = std::io::stdout();
                let _ = stdout.write_all(&clipboard::osc52_set_clipboard_base64(&data));
                let _ = stdout.flush();
                None
            }
            ServerToClient::Bell => {
                // Flush explicitly: a bell no longer triggers a frame, so
                // nothing else would push the byte out of the stdout buffer.
                let mut stdout = std::io::stdout();
                let _ = stdout.write_all(b"\x07");
                let _ = stdout.flush();
                self.runtime.dispatch(
                    EventKind::Bell,
                    EventPayload::Bell {
                        session_name: self.state.session_name.clone(),
                    },
                );
                None
            }
            // `ekko activate`: ask the host terminal for attention/focus.
            // On Wayland the terminal owns the surface, so BEL is the
            // portable handoff (e.g. foot `bell.urgent=yes`).
            ServerToClient::Activate => {
                let mut stdout = std::io::stdout();
                let _ = stdout.write_all(b"\x07");
                let _ = stdout.flush();
                None
            }
            ServerToClient::Notice(notice) => {
                let kind = match notice.level {
                    ekko_proto::NoticeLevel::Info => NoteKind::Ok,
                    ekko_proto::NoticeLevel::Warn => NoteKind::Error,
                };
                self.state.set_note(notice.message, kind, STATUS_NOTE_TTL);
                None
            }
            ServerToClient::Exit(reason) => Some(self.map_exit_reason(reason)),
            ServerToClient::Attached { .. }
            | ServerToClient::AttachRejected(_)
            | ServerToClient::ActivateResult { .. }
            | ServerToClient::Pong => None,
        }
    }

    fn map_exit_reason(&mut self, reason: ExitReason) -> ClientOutcome {
        match reason {
            ExitReason::Normal | ExitReason::Detached => ClientOutcome::Exited,
            ExitReason::Kicked => {
                self.state
                    .set_note("kicked by another client", NoteKind::Error, STATUS_NOTE_TTL);
                ClientOutcome::Exited
            }
            ExitReason::SessionExited(_) => ClientOutcome::Exited,
            ExitReason::ServerError(message) => {
                self.state
                    .set_note(message, NoteKind::Error, STATUS_NOTE_TTL);
                ClientOutcome::Exited
            }
        }
    }

    // ── Input pipeline ──────────────────────────────────────────────────────

    /// One raw read can batch several keypresses (autorepeat, fast typing,
    /// mouse-report bursts, paste markers adjacent to their payload). Each
    /// decoded token runs the pipeline separately; consecutive PTY-bound
    /// tokens coalesce back into a single `Key` message.
    fn handle_stdin(&mut self, bytes: &[u8]) -> Result<Option<ClientOutcome>> {
        let mut pty_bytes: Vec<u8> = Vec::new();
        for token in crate::input::split_key_tokens(bytes) {
            if let Some(outcome) = self.handle_key_token(token, &mut pty_bytes)? {
                // Tokens after a session-changing outcome are dropped; they
                // were aimed at a client state that no longer exists.
                self.flush_pty(&mut pty_bytes)?;
                return Ok(Some(outcome));
            }
        }
        self.flush_pty(&mut pty_bytes)?;
        Ok(None)
    }

    /// Sends accumulated PTY-bound bytes. Called before any token dispatches
    /// to a handler so PTY writes keep their order relative to handler
    /// effects (e.g. a `ForwardKey` action).
    fn flush_pty(&mut self, pty_bytes: &mut Vec<u8>) -> Result<()> {
        if !pty_bytes.is_empty() {
            self.send_message(ClientToServer::Key(std::mem::take(pty_bytes)))?;
        }
        Ok(())
    }

    fn handle_key_token(
        &mut self,
        token: &[u8],
        pty_bytes: &mut Vec<u8>,
    ) -> Result<Option<ClientOutcome>> {
        // 1. An open overlay intercepts everything — except an overlay
        // attached to the active mode (`OverlaySpec::attach_mode`), which is
        // render-only while that mode runs: its lifecycle is host-managed
        // and input keeps flowing to the mode. Opened outside its mode
        // (explicit `OpenOverlay`), an attached overlay behaves like any
        // other and owns the keys.
        if let Some(name) = self.state.overlay.as_ref().map(|o| o.name.clone())
            && self
                .runtime
                .overlay(&name)
                .is_none_or(|spec| spec.attach_mode.as_deref() != Some(self.state.mode.as_str()))
        {
            self.flush_pty(pty_bytes)?;
            let outcome = self.handle_overlay_input(token)?;
            self.state.dirty = true;
            return Ok(outcome);
        }

        // 2. Bracketed paste framing (core mechanism).
        match self.paste.feed(token) {
            PasteFeed::InProgress => return Ok(None),
            PasteFeed::Complete(payload) => {
                self.flush_pty(pty_bytes)?;
                self.send_message(ClientToServer::Paste(payload))?;
                return Ok(None);
            }
            PasteFeed::NotPaste => {}
        }

        // 3. Focus reports (mode 1004) are terminal state, not user input:
        // forward them only when the child asked for them, never let them
        // reach the keybinding/mode pipeline.
        if crate::input::is_focus_event(token) {
            if self
                .state
                .workspace
                .focused_grid()
                .is_some_and(|grid| grid.modes.focus_reporting)
            {
                self.flush_pty(pty_bytes)?;
                self.send_message(ClientToServer::Key(token.to_vec()))?;
            }
            return Ok(None);
        }

        // 4. KeyInput gate: extensions may consume or rewrite the bytes.
        let mut token = token.to_vec();
        if self.runtime.has_subscribers(EventKind::KeyInput) {
            for value in self.runtime.dispatch(
                EventKind::KeyInput,
                EventPayload::KeyInput {
                    bytes: token.clone(),
                },
            ) {
                match value {
                    EventReturn::KeyIntercept(KeyIntercept::Consume) => return Ok(None),
                    EventReturn::KeyIntercept(KeyIntercept::Transform(replacement)) => {
                        token = replacement;
                    }
                    _ => {}
                }
            }
        }

        // 5. The active mode intercepts raw bytes.
        if !self.state.in_normal_mode() {
            self.flush_pty(pty_bytes)?;
            self.state.dirty = true;
            return self.handle_mode_input(&token);
        }

        // 6. Mouse reports route to the surface owning the hit region, or to
        //    the terminal pane (forwarding, selection, wheel scroll).
        if let Some(mouse) = crate::input::parse_sgr_mouse(&token) {
            self.flush_pty(pty_bytes)?;
            self.state.dirty = true;
            return self.handle_mouse(mouse);
        }

        // 7. Keybinding registry.
        if let Some(handler) = self
            .runtime
            .match_keybinding(&token, None)
            .map(|spec| spec.handler.clone())
        {
            self.flush_pty(pty_bytes)?;
            self.state.dirty = true;
            let actions = handler(&self.snapshot());
            return self.apply_ui_actions(actions, 0);
        }

        // 8. Fall through to the PTY.
        pty_bytes.extend_from_slice(&token);
        Ok(None)
    }

    fn handle_overlay_input(&mut self, bytes: &[u8]) -> Result<Option<ClientOutcome>> {
        let Some(name) = self.state.overlay.as_ref().map(|o| o.name.clone()) else {
            return Ok(None);
        };
        let Some(spec) = self.runtime.overlay(&name) else {
            self.state.overlay = None;
            return Ok(None);
        };
        let handle_key = spec.handle_key.clone();
        let outcome = match self.state.overlay.as_mut() {
            Some(active) => handle_key(&mut active.state, bytes),
            None => return Ok(None),
        };
        match outcome {
            OverlayOutcome::None => Ok(None),
            OverlayOutcome::Close => {
                self.state.overlay = None;
                Ok(None)
            }
            OverlayOutcome::CloseWith(actions) => {
                self.state.overlay = None;
                self.apply_ui_actions(actions, 0)
            }
        }
    }

    fn handle_mode_input(&mut self, bytes: &[u8]) -> Result<Option<ClientOutcome>> {
        // Mode-scoped registry bindings match before the mode's own key
        // handler, so any extension (or user config) can extend a mode's
        // vocabulary without owning the mode. The mode stays active unless
        // an action says otherwise (`UiAction::ExitMode` / `EnterMode`).
        if let Some(handler) = self
            .runtime
            .match_keybinding(bytes, Some(self.state.mode.as_str()))
            .map(|spec| spec.handler.clone())
        {
            let actions = handler(&self.snapshot());
            return self.apply_ui_actions(actions, 0);
        }

        let Some(spec) = self.runtime.mode(&self.state.mode) else {
            // The active mode is no longer registered; drop back to normal.
            self.exit_mode();
            return Ok(None);
        };
        let on_key = spec.on_key.clone();
        let snapshot = self.snapshot();
        let outcome = match self.state.mode_state.as_mut() {
            Some(mode_state) => on_key(mode_state, bytes, &snapshot),
            None => {
                self.exit_mode();
                return Ok(None);
            }
        };
        match outcome {
            ModeOutcome::Continue => Ok(None),
            ModeOutcome::ContinueWith(actions) => self.apply_ui_actions(actions, 0),
            ModeOutcome::Exit => {
                self.exit_mode();
                Ok(None)
            }
            ModeOutcome::ExitWith(actions) => {
                self.exit_mode();
                self.apply_ui_actions(actions, 0)
            }
        }
    }

    fn handle_mouse(&mut self, mouse: crate::input::MouseEvent) -> Result<Option<ClientOutcome>> {
        let (cols, rows) = terminal_size();
        let mut terminal_rect = None;
        let actions = {
            let snapshot = self.snapshot();
            let surfaces = self.runtime.visible_surfaces(&snapshot);
            let layout = resolve_layout(cols, rows, &surfaces);
            let region = layout
                .regions
                .iter()
                .find(|region| region.rect.contains_cell(mouse.col, mouse.row));
            match region {
                Some(region) => {
                    let Some(on_mouse) = self
                        .runtime
                        .surface(&region.name)
                        .and_then(|spec| spec.on_mouse.clone())
                    else {
                        return Ok(None);
                    };
                    let event = ekko_ext::SurfaceMouseEvent {
                        kind: mouse.kind.into(),
                        col: mouse.col - region.rect.col,
                        row: mouse.row - region.rect.row,
                        region_cols: region.rect.cols,
                        region_rows: region.rect.rows,
                    };
                    on_mouse(&event, &snapshot)
                }
                None if layout.terminal.contains_cell(mouse.col, mouse.row) => {
                    terminal_rect = Some(layout.terminal);
                    Vec::new()
                }
                None => return Ok(None),
            }
        };
        if let Some(terminal) = terminal_rect {
            return self.handle_terminal_mouse(mouse, terminal);
        }
        self.apply_ui_actions(actions, 0)
    }

    /// Mouse input over the terminal region. The hit names the target pane:
    /// any press inside a pane focuses it (server-confirmed) before the event
    /// is handled, and all coordinates become pane-local. Precedence within
    /// the hit pane: a mouse-aware child on the live screen gets the event
    /// verbatim; otherwise the wheel scrolls (history, or arrow keys on the
    /// alternate screen) and the left button drives selection + OSC 52 copy.
    fn handle_terminal_mouse(
        &mut self,
        mouse: crate::input::MouseEvent,
        terminal: ekko_ext::Rect,
    ) -> Result<Option<ClientOutcome>> {
        use crate::input::MouseKind;

        // Resolve the hit pane and pane-local coordinates from the client's
        // workspace cache (topology projection of the last frame).
        let hit = self.state.workspace.panes.iter().find_map(|(&id, pane)| {
            let col = mouse.col - terminal.col - i32::from(pane.rect.x);
            let row = mouse.row - terminal.row - i32::from(pane.rect.y);
            (col >= 0
                && col < i32::from(pane.rect.cols)
                && row >= 0
                && row < i32::from(pane.rect.rows))
            .then_some((id, col, row, pane.grid.modes, pane.grid.scrollback))
        });
        let Some((pane_id, local_col, local_row, modes, scrollback)) = hit else {
            return Ok(None);
        };
        // A mouse hit focuses its pane before anything is forwarded, so a
        // click-into + child-mouse-report sequence reaches the right PTY.
        if self.state.workspace.focused != pane_id {
            self.send_message(ClientToServer::FocusPane { pane: pane_id })?;
            self.state.workspace.focused = pane_id;
        }
        let live = scrollback == 0;

        if live && modes.mouse_mode != ekko_proto::MouseMode::None {
            if let Some(bytes) =
                crate::input::encode_mouse_for_child(&mouse, &modes, local_col, local_row)
            {
                self.send_message(ClientToServer::Key(bytes))?;
            }
            return Ok(None);
        }

        match mouse.kind {
            MouseKind::WheelUp | MouseKind::WheelDown => {
                let up = mouse.kind == MouseKind::WheelUp;
                if modes.alt_screen && live {
                    // No history on the alternate screen: translate the
                    // wheel to arrow keys so pagers/editors still scroll.
                    let arrow: &[u8] = match (up, modes.app_cursor) {
                        (true, true) => b"\x1bOA",
                        (true, false) => b"\x1b[A",
                        (false, true) => b"\x1bOB",
                        (false, false) => b"\x1b[B",
                    };
                    self.send_message(ClientToServer::Key(arrow.repeat(3)))?;
                } else {
                    let delta = if up { 3 } else { -3 };
                    self.send_message(ClientToServer::Scroll { delta })?;
                }
            }
            MouseKind::LeftPress => {
                self.state
                    .selection
                    .set(ekko_grid::selection::SelectionPoint {
                        row: local_row as u16,
                        col: local_col as u16,
                    });
                self.state.selection_pane = Some(pane_id);
            }
            MouseKind::LeftDrag => {
                if self.state.selection.is_active() && self.state.selection_pane == Some(pane_id) {
                    self.state
                        .selection
                        .update_focus(ekko_grid::selection::SelectionPoint {
                            row: local_row as u16,
                            col: local_col as u16,
                        });
                }
            }
            MouseKind::LeftRelease => {
                // The release report is the authoritative final hovered cell;
                // motion events can be coalesced before the button comes up.
                if self.state.selection.is_active() && self.state.selection_pane == Some(pane_id) {
                    self.state
                        .selection
                        .update_focus(ekko_grid::selection::SelectionPoint {
                            row: local_row as u16,
                            col: local_col as u16,
                        });
                }
                self.state.selection.end_drag();
                if let Some(range) = self.state.selection.normalized() {
                    let text = self
                        .state
                        .workspace
                        .panes
                        .get(&pane_id)
                        .map(|pane| pane.grid.selected_text(range))
                        .unwrap_or_default();
                    if !text.is_empty() {
                        let mut stdout = std::io::stdout();
                        let _ = stdout.write_all(&clipboard::osc52_set_clipboard(text.as_bytes()));
                        let _ = stdout.flush();
                        self.state.set_note(
                            format!("copied {} chars", text.chars().count()),
                            NoteKind::Ok,
                            STATUS_NOTE_TTL,
                        );
                    }
                } else {
                    // A plain click without a drag clears any old highlight.
                    self.state.selection.clear();
                    self.state.selection_pane = None;
                }
            }
            MouseKind::Other => {}
        }
        Ok(None)
    }

    // ── UiAction interpreter: the single write path ─────────────────────────

    fn apply_ui_actions(
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

    fn apply_ui_action(&mut self, action: UiAction, depth: u8) -> Result<Option<ClientOutcome>> {
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
                let _ = self.send_message(ClientToServer::KillCurrentSession);
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
        }
    }

    /// The mode to re-enter after a session switch. A sticky mode-scoped
    /// action (a leader map walking sessions) runs without `ExitMode`, so
    /// the mode it was invoked from survives the reattach; actions that
    /// exited the mode first carry nothing.
    fn mode_carry(&self) -> Option<String> {
        (!self.state.in_normal_mode()).then(|| self.state.mode.clone())
    }

    fn enter_mode(&mut self, name: &str) {
        let Some(spec) = self.runtime.mode(name) else {
            self.state.set_note(
                format!("unknown mode: {name}"),
                NoteKind::Error,
                STATUS_NOTE_TTL,
            );
            return;
        };
        let init_state = spec.init_state.clone();
        let from = std::mem::replace(&mut self.state.mode, name.to_string());
        self.state.mode_state = Some(init_state());
        // Mode-attached overlays follow the mode: the previous mode's
        // attached overlay closes with it (a mode-to-mode transition skips
        // exit_mode), and the new mode's attached overlay opens — without
        // stomping an overlay that is already open for another reason.
        self.close_attached_overlay(&from);
        if self.state.overlay.is_none()
            && let Some(attached) = self
                .runtime
                .overlay_attached_to(name)
                .map(|o| o.name.clone())
        {
            self.open_overlay(&attached);
        }
        self.runtime.dispatch(
            EventKind::ModeChanged,
            EventPayload::ModeChanged {
                from,
                to: name.to_string(),
            },
        );
    }

    fn exit_mode(&mut self) {
        if self.state.in_normal_mode() {
            return;
        }
        let from = std::mem::replace(
            &mut self.state.mode,
            ClientSnapshot::NORMAL_MODE.to_string(),
        );
        self.state.mode_state = None;
        self.close_attached_overlay(&from);
        self.runtime.dispatch(
            EventKind::ModeChanged,
            EventPayload::ModeChanged {
                from,
                to: ClientSnapshot::NORMAL_MODE.to_string(),
            },
        );
    }

    /// Close the open overlay if it is attached to `mode` (see
    /// [`ekko_ext::OverlaySpec::attach_mode`]).
    fn close_attached_overlay(&mut self, mode: &str) {
        if let Some(open) = self.state.overlay.as_ref().map(|o| o.name.clone())
            && self
                .runtime
                .overlay(&open)
                .is_some_and(|spec| spec.attach_mode.as_deref() == Some(mode))
        {
            self.state.overlay = None;
            self.state.dirty = true;
        }
    }

    fn open_overlay(&mut self, name: &str) {
        let Some(spec) = self.runtime.overlay(name) else {
            self.state.set_note(
                format!("unknown overlay: {name}"),
                NoteKind::Error,
                STATUS_NOTE_TTL,
            );
            return;
        };
        let init_state = spec.init_state.clone();
        let build_payload = spec.build_payload.clone();
        let payload = build_payload.map(|build| build(&self.runtime.registry_view()));
        self.state.overlay = Some(ActiveOverlay {
            name: name.to_string(),
            state: init_state(payload),
        });
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// PTY dimensions for a resolved layout: the terminal pane, floored at 1x1
/// so a frame fully consumed by chrome can't ask the server for a zero-size
/// grid.
fn pane_grid_size(layout: &ResolvedLayout) -> (u16, u16) {
    (
        layout.terminal.cols.max(1) as u16,
        layout.terminal.rows.max(1) as u16,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_ext::Rect;

    #[test]
    fn pane_grid_size_reports_the_terminal_pane_not_the_frame() {
        let layout = ResolvedLayout {
            regions: Vec::new(),
            terminal: Rect::new(30, 0, 90, 39),
        };
        assert_eq!(pane_grid_size(&layout), (90, 39));
    }

    #[test]
    fn pane_grid_size_floors_a_degenerate_pane_at_one_cell() {
        let layout = ResolvedLayout {
            regions: Vec::new(),
            terminal: Rect::new(0, 0, 0, 0),
        };
        assert_eq!(pane_grid_size(&layout), (1, 1));
    }
}
