//! The hub: single-threaded owner of all session state (vt100 parser, the
//! PTY handle, and the set of attached clients). Everything else (listener,
//! per-client I/O, PTY reader/writer) talks to it exclusively through
//! [`HubInstruction`] messages, zellij-style.

use std::collections::{HashMap, HashSet};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use ekko_config::Config;
use ekko_proto::{
    AttachRejectReason, ClientToServer, CursorState, ExitReason, GridPayload, GridRow, GridUpdate,
    ServerNotice, ServerToClient, TermModes,
};
use ekko_pty::{PtyCommand, WinSize};
use interprocess::local_socket::Stream as LocalSocketStream;

use ekko_event::{EventKind, EventPayload, EventReturn, SessionExitReason};
use ekko_ext::AppRuntime;

use crate::client_io::{self, ClientHandle, ClientId};
use crate::grid;
use crate::pty_io;
use crate::pty_writer::{self, PtyWriterInstruction};
use crate::vt_compat::HvpToCup;

/// Minimum spacing between `GridUpdate` broadcasts while a child floods the
/// PTY; the ceiling on how long a visible change may wait for a frame.
const RENDER_TICK: Duration = Duration::from_millis(16);

/// Settle window after a visible change before broadcasting: children paint
/// a keystroke in several PTY writes (reedline repaints its prompt, TUIs
/// compose a frame), and broadcasting the first chunk immediately would
/// spend the tick budget on a half-drawn frame while the chunk carrying the
/// echoed character waits out a full tick.
const RENDER_SETTLE: Duration = Duration::from_millis(1);

/// How often the `Heartbeat` lifecycle event fires (the resurrection
/// builtin uses it to refresh the manifest's `last_active_secs`).
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

pub enum HubInstruction {
    NewClient(LocalSocketStream),
    ClientMsg(ClientId, ClientToServer),
    ClientDisconnected(ClientId),
    ClientWriteFailed(ClientId),
    PtyBytes(Vec<u8>),
    PtyExited(Option<i32>),
    ThreadPanicked {
        thread_name: String,
        message: String,
    },
    /// Periodic liveness tick from the heartbeat thread; the hub dispatches
    /// it as an extension event on its own thread so the `AppRuntime` stays
    /// single-owner.
    HeartbeatTick,
    Shutdown,
}

/// The fields of [`ClientToServer::Attach`], regrouped for `on_attach`.
struct AttachRequest {
    wire_version: u32,
    cols: u16,
    rows: u16,
    cwd: PathBuf,
    shell: Option<PathBuf>,
    force: bool,
    terminal_colors: Option<ekko_proto::TerminalColors>,
}

/// The live PTY plus everything needed to talk to it.
struct PtySession {
    /// Kept alive only so the fd stays open; all I/O goes through the raw fd
    /// handed to the reader/writer threads.
    _master_fd: OwnedFd,
    child_pid: ekko_pty::Pid,
    writer_tx: Sender<PtyWriterInstruction>,
    /// Bytes read from the PTY but not yet parsed by the hub. The reader
    /// thread stalls while this exceeds its cap, so a flooding child fills
    /// the kernel PTY buffer and blocks instead of growing our memory.
    backlog: Arc<AtomicUsize>,
}

pub struct Hub {
    session_name: String,
    config: Config,
    runtime: AppRuntime,
    hub_tx: Sender<HubInstruction>,
    clients: HashMap<ClientId, ClientHandle>,
    /// Attached clients and their terminal sizes (cols, rows). The session
    /// is sized to the smallest attached client, tmux-style.
    attached: HashMap<ClientId, (u16, u16)>,
    next_client_id: ClientId,
    /// The host terminal's colors from the most recent attach whose client
    /// probe succeeded; fed to [`TermEvents`] so the hub can answer the
    /// child's OSC 10/11/4 color queries. Kept on the hub so a PTY respawn
    /// (which rebuilds the parser) doesn't lose it.
    host_colors: Option<ekko_proto::TerminalColors>,
    parser: vt100::Parser<TermEvents>,
    /// Rewrites HVP finals to CUP before the parser (vt100 has no HVP arm).
    vt_compat: HvpToCup,
    pty: Option<PtySession>,
    epoch: u64,
    dirty: bool,
    /// When the next broadcast is due, armed by [`Self::mark_dirty`]. `None`
    /// while nothing visible has changed.
    render_deadline: Option<Instant>,
    /// Time of the last broadcast that actually sent frames. Steady frames
    /// (nothing visible changed) must not touch this: a query-only PTY chunk
    /// would otherwise consume the tick budget and defer the real repaint.
    last_render: Instant,
    /// The rows/cursor as of the last broadcast, so each render tick can send
    /// a sparse `Rows` diff instead of the whole screen.
    last_sent_rows: Vec<GridRow>,
    last_sent_cursor: Option<CursorState>,
    last_sent_size: (u16, u16),
    last_sent_modes: TermModes,
    last_sent_scrollback: u32,
    /// Force the next broadcast to be a `Full` payload for everyone
    /// (resize/respawn invalidates the diff base).
    force_full: bool,
    /// Clients whose outgoing queue dropped a grid frame; they get a `Full`
    /// resync on the next broadcast since their state may have diverged.
    needs_full: HashSet<ClientId>,
    should_exit: bool,
    crashed: bool,
}

impl Hub {
    pub fn new(
        session_name: String,
        config: Config,
        hub_tx: Sender<HubInstruction>,
        runtime: AppRuntime,
    ) -> Self {
        let scrollback = config.general.scrollback_lines;
        Self {
            session_name,
            config,
            runtime,
            hub_tx,
            clients: HashMap::new(),
            attached: HashMap::new(),
            next_client_id: 0,
            host_colors: None,
            parser: vt100::Parser::new_with_callbacks(24, 80, scrollback, TermEvents::default()),
            vt_compat: HvpToCup::default(),
            pty: None,
            epoch: 0,
            dirty: false,
            render_deadline: None,
            last_render: Instant::now() - RENDER_TICK,
            last_sent_rows: Vec::new(),
            last_sent_cursor: None,
            last_sent_size: (0, 0),
            last_sent_modes: TermModes::default(),
            last_sent_scrollback: 0,
            force_full: true,
            needs_full: HashSet::new(),
            should_exit: false,
            crashed: false,
        }
    }

    /// Runs the hub loop until a shutdown condition is reached. Always
    /// returns `Ok(())`; failures along the way are logged, not propagated,
    /// since a single-session daemon has nowhere else to report them.
    pub fn run(mut self, rx: Receiver<HubInstruction>) {
        loop {
            let received = match self.render_deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if deadline <= now {
                        // Check before receiving: a flooding child delivers
                        // instructions faster than the timeout would fire,
                        // and the frame must not starve behind them.
                        self.render_now();
                        continue;
                    }
                    match rx.recv_timeout(deadline - now) {
                        Ok(instr) => Some(instr),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None => match rx.recv() {
                    Ok(instr) => Some(instr),
                    Err(_) => break,
                },
            };

            match received {
                Some(instr) => self.handle(instr),
                None => self.render_now(),
            }

            if self.should_exit {
                break;
            }
        }
        self.shutdown_cleanup();
    }

    /// Note a visible state change and arm the broadcast deadline: at least
    /// [`RENDER_SETTLE`] from now (multi-chunk repaints land in one frame),
    /// at least [`RENDER_TICK`] since the last real broadcast (floods stay
    /// bounded). An armed deadline is never pushed back by later changes, so
    /// a continuous stream cannot starve the frame.
    fn mark_dirty(&mut self) {
        self.dirty = true;
        if self.render_deadline.is_none() {
            let now = Instant::now();
            self.render_deadline = Some((self.last_render + RENDER_TICK).max(now + RENDER_SETTLE));
        }
    }

    fn handle(&mut self, instr: HubInstruction) {
        match instr {
            HubInstruction::NewClient(stream) => self.on_new_client(stream),
            HubInstruction::ClientMsg(id, msg) => self.on_client_msg(id, msg),
            HubInstruction::ClientDisconnected(id) => self.on_client_disconnected(id),
            HubInstruction::ClientWriteFailed(id) => {
                log::info!("hub: evicting client {id} after a write failure");
                self.on_client_disconnected(id);
            }
            HubInstruction::PtyBytes(mut bytes) => self.on_pty_bytes(&mut bytes),
            HubInstruction::PtyExited(code) => self.on_pty_exited(code),
            HubInstruction::ThreadPanicked {
                thread_name,
                message,
            } => self.on_thread_panicked(thread_name, message),
            HubInstruction::HeartbeatTick => {
                self.dispatch(
                    EventKind::Heartbeat,
                    EventPayload::Heartbeat {
                        session_name: self.session_name.clone(),
                    },
                );
            }
            HubInstruction::Shutdown => self.on_shutdown_signal(),
        }
    }

    // -- connection lifecycle -------------------------------------------

    fn on_new_client(&mut self, stream: LocalSocketStream) {
        let id = self.next_client_id;
        self.next_client_id += 1;
        let handle = client_io::spawn(id, stream, self.hub_tx.clone());
        self.clients.insert(id, handle);
    }

    fn on_client_disconnected(&mut self, id: ClientId) {
        self.clients.remove(&id);
        self.needs_full.remove(&id);
        if self.attached.remove(&id).is_some() {
            self.resize_to_fit();
        }
    }

    fn send_to(&self, id: ClientId, msg: ServerToClient) {
        if let Some(client) = self.clients.get(&id)
            && client.tx.try_send(msg).is_err()
        {
            log::debug!("hub: client {id}'s outgoing queue is full/closed, dropping a message");
        }
    }

    fn send_to_attached(&self, msg: ServerToClient) {
        for &id in self.attached.keys() {
            self.send_to(id, msg.clone());
        }
    }

    /// Relay to exactly one attached client. Used for out-of-band requests
    /// like `ekko activate`, where one terminal should request attention, not
    /// every viewer.
    fn send_to_one_attached(&self, msg: ServerToClient) -> bool {
        let Some(id) = self.attached.keys().min().copied() else {
            return false;
        };
        self.send_to(id, msg);
        true
    }

    // -- client messages --------------------------------------------------

    fn on_client_msg(&mut self, id: ClientId, msg: ClientToServer) {
        match msg {
            ClientToServer::Attach {
                wire_version,
                cols,
                rows,
                cwd,
                shell,
                force,
                terminal_colors,
            } => self.on_attach(
                id,
                AttachRequest {
                    wire_version,
                    cols,
                    rows,
                    cwd,
                    shell,
                    force,
                    terminal_colors,
                },
            ),
            ClientToServer::Detach => self.on_detach(id),
            ClientToServer::Resize { cols, rows } => self.on_resize(id, cols, rows),
            ClientToServer::Key(bytes) => self.on_input(id, bytes),
            ClientToServer::Paste(bytes) => self.on_paste(id, bytes),
            ClientToServer::Scroll { delta } => self.on_scroll(id, delta),
            ClientToServer::ScrollReset => self.set_scrollback_view(id, 0),
            ClientToServer::KillCurrentSession => {
                let name = self.session_name.clone();
                self.on_kill_session(id, &name);
            }
            ClientToServer::KillSession(name) => self.on_kill_session(id, &name),
            ClientToServer::Ping => self.send_to(id, ServerToClient::Pong),
            ClientToServer::Activate => self.on_activate(id),
        }
    }

    fn on_activate(&mut self, id: ClientId) {
        let delivered = self.send_to_one_attached(ServerToClient::Activate);
        self.send_to(id, ServerToClient::ActivateResult { delivered });
    }

    fn on_attach(&mut self, id: ClientId, req: AttachRequest) {
        let AttachRequest {
            wire_version,
            cols,
            rows,
            cwd,
            shell,
            force,
            terminal_colors,
        } = req;
        if wire_version != ekko_proto::WIRE_VERSION {
            self.send_to(
                id,
                ServerToClient::AttachRejected(AttachRejectReason::WrongWireVersion),
            );
            return;
        }
        // Adopt the client's probed host colors (first probe wins; a later
        // client that failed its probe must not clobber a good answer) and
        // refresh the live parser so color queries from an already-running
        // child are answered with them too.
        if terminal_colors.is_some() && self.host_colors.is_none() {
            self.host_colors = terminal_colors;
            self.parser.callbacks_mut().host_colors = self.host_colors.clone();
        }
        if force {
            let others: Vec<ClientId> =
                self.attached.keys().copied().filter(|&c| c != id).collect();
            for other in others {
                log::info!("hub: client {id} kicked client {other} (attach --force)");
                self.send_to(other, ServerToClient::Exit(ExitReason::Kicked));
                self.attached.remove(&other);
            }
        }

        if self.pty.is_none() {
            let shell = shell.unwrap_or_else(|| self.config.resolve_shell());
            match self.spawn_pty(&cwd, &shell, cols, rows) {
                Ok(()) => {
                    self.dispatch(
                        EventKind::SessionCreated,
                        EventPayload::SessionCreated {
                            session_name: self.session_name.clone(),
                            shell,
                            cwd,
                        },
                    );
                    self.spawn_heartbeat();
                }
                Err(e) => {
                    log::error!("hub: failed to spawn pty: {e}");
                    self.send_to(
                        id,
                        ServerToClient::AttachRejected(AttachRejectReason::SpawnFailed(
                            e.to_string(),
                        )),
                    );
                    return;
                }
            }
        }

        self.attached.insert(id, (cols, rows));
        self.resize_to_fit();
        self.send_to(
            id,
            ServerToClient::Attached {
                session_name: self.session_name.clone(),
                wire_version: ekko_proto::WIRE_VERSION,
            },
        );
        // Snapshot via the regular render path (`needs_full`) instead of a
        // manual send, so the diff bookkeeping stays consistent and other
        // clients don't get a redundant `Full` on the next tick.
        self.needs_full.insert(id);
        self.dirty = true;
        self.render_now();
        let (cols, rows) = {
            let screen = self.parser.screen();
            let size = screen.size();
            (size.1, size.0)
        };
        self.dispatch(
            EventKind::ClientAttached,
            EventPayload::ClientAttached {
                session_name: self.session_name.clone(),
                client_id: id,
                cols,
                rows,
            },
        );
    }

    fn on_detach(&mut self, id: ClientId) {
        if self.attached.remove(&id).is_none() {
            return;
        }
        self.send_to(id, ServerToClient::Exit(ExitReason::Detached));
        self.resize_to_fit();
        self.dispatch(
            EventKind::ClientDetached,
            EventPayload::ClientDetached {
                session_name: self.session_name.clone(),
                client_id: id,
            },
        );
    }

    fn on_resize(&mut self, id: ClientId, cols: u16, rows: u16) {
        let Some(size) = self.attached.get_mut(&id) else {
            return;
        };
        *size = (cols, rows);
        self.resize_to_fit();
    }

    /// Resize the session to the smallest attached client so every viewer
    /// sees the full grid. No-op while nothing is attached (a detached
    /// session keeps its last size).
    fn resize_to_fit(&mut self) {
        let Some((cols, rows)) = self
            .attached
            .values()
            .copied()
            .reduce(|(c1, r1), (c2, r2)| (c1.min(c2), r1.min(r2)))
        else {
            return;
        };
        let (rows_now, cols_now) = self.parser.screen().size();
        if (cols, rows) != (cols_now, rows_now) {
            self.resize(cols, rows);
            self.mark_dirty();
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.parser.screen_mut().set_size(rows, cols);
        // The diff base no longer matches the screen shape.
        self.force_full = true;
        if let Some(pty) = &self.pty {
            let _ = pty.writer_tx.send(PtyWriterInstruction::Resize(cols, rows));
        }
        self.dispatch(
            EventKind::PtyResized,
            EventPayload::PtyResized {
                session_name: self.session_name.clone(),
                cols,
                rows,
            },
        );
    }

    fn on_input(&mut self, id: ClientId, mut bytes: Vec<u8>) {
        // Cursor keys arrive encoded for the host terminal's DECCKM state;
        // re-encode them for the child's (see `input_compat`).
        crate::input_compat::rewrite_cursor_keys(
            &mut bytes,
            self.parser.screen().application_cursor(),
        );
        self.write_to_pty(id, bytes);
    }

    fn write_to_pty(&mut self, id: ClientId, bytes: Vec<u8>) {
        if !self.attached.contains_key(&id) {
            return;
        }
        // Typing jumps the view back to the live screen, tmux-style.
        if self.parser.screen().scrollback() > 0 {
            self.parser.screen_mut().set_scrollback(0);
            self.mark_dirty();
        }
        if let Some(pty) = &self.pty {
            let _ = pty.writer_tx.send(PtyWriterInstruction::Write(bytes));
        }
    }

    /// Forward pasted content, re-wrapped in bracketed-paste markers when the
    /// child has requested them (the client strips the host's markers). Any
    /// end marker embedded in the payload is removed so a malicious paste
    /// can't break out of the bracket.
    fn on_paste(&mut self, id: ClientId, bytes: Vec<u8>) {
        let bytes = if self.parser.screen().bracketed_paste() {
            let mut wrapped = Vec::with_capacity(bytes.len() + 12);
            wrapped.extend_from_slice(b"\x1b[200~");
            wrapped.extend(strip_paste_end_markers(&bytes));
            wrapped.extend_from_slice(b"\x1b[201~");
            wrapped
        } else {
            bytes
        };
        // Straight to the PTY: paste payload is literal text, and any escape
        // sequences within it must survive byte-for-byte.
        self.write_to_pty(id, bytes);
    }

    /// Move the scrollback view by `delta` lines (positive = back into
    /// history). vt100 clamps to the actual history length.
    fn on_scroll(&mut self, id: ClientId, delta: i32) {
        let current = self.parser.screen().scrollback() as i64;
        let target = (current + i64::from(delta)).max(0) as usize;
        self.set_scrollback_view(id, target);
    }

    fn set_scrollback_view(&mut self, id: ClientId, rows: usize) {
        if !self.attached.contains_key(&id) {
            return;
        }
        // There is no history to scroll on the alternate screen.
        if self.parser.screen().alternate_screen() && rows > 0 {
            return;
        }
        let before = self.parser.screen().scrollback();
        self.parser.screen_mut().set_scrollback(rows);
        if self.parser.screen().scrollback() != before {
            self.mark_dirty();
        }
    }

    fn on_kill_session(&mut self, id: ClientId, name: &str) {
        if name != self.session_name {
            log::warn!("hub: ignoring KillSession for foreign session {name}");
            return;
        }
        if let Some(pty) = &self.pty {
            let _ = ekko_pty::kill(pty.child_pid);
        }
        self.send_to(id, ServerToClient::Exit(ExitReason::Normal));
        self.fire_session_exited(None, SessionExitReason::Killed);
        self.finish_exit();
    }

    // -- PTY events ---------------------------------------------------------

    fn on_pty_bytes(&mut self, bytes: &mut [u8]) {
        self.vt_compat.rewrite_in_place(bytes);
        self.parser.process(bytes);
        if let Some(pty) = &self.pty {
            pty.backlog.fetch_sub(bytes.len(), Ordering::Release);
        }
        let replies = std::mem::take(&mut self.parser.callbacks_mut().replies);
        if !replies.is_empty()
            && let Some(pty) = &self.pty
        {
            let _ = pty.writer_tx.send(PtyWriterInstruction::Write(replies));
        }
        let bells = std::mem::take(&mut self.parser.callbacks_mut().audible);
        if bells > 0 {
            self.send_to_attached(ServerToClient::Bell);
            self.dispatch(
                EventKind::Bell,
                EventPayload::Bell {
                    session_name: self.session_name.clone(),
                },
            );
        }
        if let Some(title) = self.parser.callbacks_mut().title.take() {
            self.send_to_attached(ServerToClient::Title(title));
        }
        if let Some(data) = self.parser.callbacks_mut().clipboard_copy.take() {
            self.send_to_attached(ServerToClient::ClipboardCopy(data));
        }
        self.mark_dirty();
    }

    fn on_pty_exited(&mut self, code: Option<i32>) {
        log::info!("hub: shell exited with code {code:?}");
        self.send_to_attached(ServerToClient::Exit(ExitReason::SessionExited(code)));
        self.fire_session_exited(code, SessionExitReason::ShellExited);
        self.finish_exit();
    }

    fn on_thread_panicked(&mut self, thread_name: String, message: String) {
        log::error!("hub: thread '{thread_name}' panicked: {message}");
        self.send_to_attached(ServerToClient::Exit(ExitReason::ServerError(message)));
        if thread_name.starts_with("pty-") {
            self.crashed = true;
            self.fire_session_exited(None, SessionExitReason::Crashed);
            self.finish_exit();
        } else if let Some(id) = parse_client_thread_id(&thread_name) {
            self.on_client_disconnected(id);
        }
    }

    fn on_shutdown_signal(&mut self) {
        log::info!("hub: received shutdown signal");
        self.send_to_attached(ServerToClient::Exit(ExitReason::Normal));
        if let Some(pty) = &self.pty {
            let _ = ekko_pty::kill(pty.child_pid);
        }
        self.fire_session_exited(None, SessionExitReason::Shutdown);
        self.finish_exit();
    }

    fn finish_exit(&mut self) {
        self.should_exit = true;
    }

    /// Single funnel for every way a session can end: PTY exit, explicit
    /// kill, PTY-thread crash, or a shutdown signal.
    fn fire_session_exited(&mut self, exit_code: Option<i32>, reason: SessionExitReason) {
        self.dispatch(
            EventKind::SessionExited,
            EventPayload::SessionExited {
                session_name: self.session_name.clone(),
                exit_code,
                reason,
            },
        );
    }

    /// Dispatch an event and translate any `EmitNotice` returns into wire
    /// `Notice` messages for the attached client. The hub is the only place
    /// that knows both the event vocabulary and the wire vocabulary.
    fn dispatch(&mut self, kind: EventKind, payload: EventPayload) -> Vec<EventReturn> {
        let labeled = self.runtime.dispatch_labeled(kind, payload);
        let mut returns = Vec::with_capacity(labeled.len());
        for (label, value) in labeled {
            if let EventReturn::EmitNotice { level, message } = &value {
                self.send_to_attached(ServerToClient::Notice(ServerNotice {
                    source: label.clone(),
                    level: match level {
                        ekko_event::NoticeLevel::Info => ekko_proto::NoticeLevel::Info,
                        ekko_event::NoticeLevel::Warn => ekko_proto::NoticeLevel::Warn,
                    },
                    message: message.clone(),
                }));
            }
            returns.push(value);
        }
        returns
    }

    // -- rendering ------------------------------------------------------

    fn render_now(&mut self) {
        self.render_deadline = None;
        if !self.dirty {
            return;
        }

        let cursor_shape = self.parser.callbacks_mut().cursor_shape;
        let focus_reporting = self.parser.callbacks_mut().focus_reporting;
        let (rows, cursor, size, modes, scrollback) = {
            let screen = self.parser.screen();
            let (screen_rows, screen_cols) = screen.size();
            let mut cursor = grid::cursor_state(screen);
            cursor.shape = cursor_shape;
            let scrollback = screen.scrollback() as u32;
            // The cursor belongs to the live screen; hide it while the view
            // is scrolled back into history.
            if scrollback > 0 {
                cursor.visible = false;
            }
            let mut modes = grid::term_modes(screen);
            modes.focus_reporting = focus_reporting;
            (
                grid::screen_rows(screen),
                cursor,
                (screen_cols, screen_rows),
                modes,
                scrollback,
            )
        };

        let full_for_all =
            self.force_full || size != self.last_sent_size || self.last_sent_rows.is_empty();
        let patches: Vec<(u16, GridRow)> = if full_for_all {
            Vec::new()
        } else {
            rows.iter()
                .enumerate()
                .filter(|(index, row)| self.last_sent_rows.get(*index) != Some(row))
                .map(|(index, row)| (index as u16, row.clone()))
                .collect()
        };

        // Nothing visible changed (e.g. the PTY bytes were only terminal
        // queries): drop the frame entirely instead of broadcasting it.
        let steady = !full_for_all
            && patches.is_empty()
            && self.last_sent_cursor == Some(cursor)
            && self.last_sent_modes == modes
            && self.last_sent_scrollback == scrollback;
        // Nothing was sent, so the tick budget is untouched: the next real
        // change still broadcasts after only the settle window.
        if steady && self.needs_full.is_empty() {
            self.dirty = false;
            return;
        }

        self.epoch += 1;
        let mut dropped: Vec<ClientId> = Vec::new();
        for (&id, client) in &self.clients {
            if !self.attached.contains_key(&id) {
                continue;
            }
            let payload = if full_for_all || self.needs_full.contains(&id) {
                GridPayload::Full(rows.clone())
            } else if steady {
                // Only `needs_full` clients have anything to catch up on.
                continue;
            } else {
                GridPayload::Rows(patches.clone())
            };
            let update = GridUpdate {
                epoch: self.epoch,
                cols: size.0,
                rows: size.1,
                cursor: Some(cursor),
                modes,
                scrollback,
                payload,
            };
            if client.tx.try_send(ServerToClient::Grid(update)).is_err() {
                log::debug!("hub: client {id}'s queue dropped a grid frame; full resync queued");
                dropped.push(id);
            }
        }
        self.needs_full = dropped.into_iter().collect();

        self.last_sent_rows = rows;
        self.last_sent_cursor = Some(cursor);
        self.last_sent_size = size;
        self.last_sent_modes = modes;
        self.last_sent_scrollback = scrollback;
        self.force_full = false;
        self.dirty = false;
        self.last_render = Instant::now();
    }

    // -- pty / session bookkeeping ---------------------------------------

    fn spawn_pty(
        &mut self,
        cwd: &std::path::Path,
        shell: &std::path::Path,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<()> {
        self.parser = vt100::Parser::new_with_callbacks(
            rows,
            cols,
            self.config.general.scrollback_lines,
            TermEvents {
                host_colors: self.host_colors.clone(),
                ..TermEvents::default()
            },
        );
        // Fresh parser, fresh diff base, fresh compat-filter state.
        self.force_full = true;
        self.vt_compat = HvpToCup::default();

        let mut shell = shell.to_path_buf();
        let mut cwd = cwd.to_path_buf();
        // Stamp the pane with its own session so shells can tell they're
        // inside ekko (a stale outer value is dropped in ekko-pty); extension
        // overrides are appended after and win on duplicate keys.
        let mut env: Vec<(String, String)> =
            vec![("EKKO_SESSION_NAME".to_string(), self.session_name.clone())];
        for value in self.dispatch(
            EventKind::BeforePtySpawn,
            EventPayload::PtySpawn {
                session_name: self.session_name.clone(),
                shell: shell.clone(),
                cwd: cwd.clone(),
                cols,
                rows,
            },
        ) {
            if let EventReturn::PtySpawnOverride {
                shell: shell_override,
                cwd: cwd_override,
                env: extra_env,
            } = value
            {
                if let Some(value) = shell_override {
                    shell = value;
                }
                if let Some(value) = cwd_override {
                    cwd = value;
                }
                env.extend(extra_env);
            }
        }

        let hub_tx = self.hub_tx.clone();
        let mut cmd = PtyCommand::new(&shell).cwd(&cwd);
        cmd.env = env;
        let handle = ekko_pty::spawn_pty(
            cmd,
            WinSize { cols, rows },
            Box::new(move |code| {
                let _ = hub_tx.send(HubInstruction::PtyExited(code));
            }),
        )?;

        let fd = handle.master_fd.as_raw_fd();
        let (writer_tx, writer_rx) = crossbeam_channel::unbounded::<PtyWriterInstruction>();

        let backlog = Arc::new(AtomicUsize::new(0));
        let reader_hub_tx = self.hub_tx.clone();
        let reader_backlog = Arc::clone(&backlog);
        thread::Builder::new()
            .name("pty-reader".to_string())
            .spawn(move || pty_io::run(fd, &reader_hub_tx, &reader_backlog))?;

        thread::Builder::new()
            .name("pty-writer".to_string())
            .spawn(move || pty_writer::run(&writer_rx, fd))?;

        self.pty = Some(PtySession {
            _master_fd: handle.master_fd,
            child_pid: handle.child_pid,
            writer_tx,
            backlog,
        });
        Ok(())
    }

    fn spawn_heartbeat(&self) {
        let hub_tx = self.hub_tx.clone();
        if let Err(e) = thread::Builder::new()
            .name("heartbeat".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(HEARTBEAT_INTERVAL);
                    if hub_tx.send(HubInstruction::HeartbeatTick).is_err() {
                        return;
                    }
                }
            })
        {
            log::warn!("hub: failed to spawn heartbeat thread: {e}");
        }
    }

    /// Called once the hub loop exits, regardless of why.
    fn shutdown_cleanup(&mut self) {
        if let Some(pty) = &self.pty {
            let _ = pty.writer_tx.send(PtyWriterInstruction::Shutdown);
        }
        let _ = std::fs::remove_file(ekko_proto::socket_path(&self.session_name));
    }
}

/// Events collected while feeding bytes through the vt100 parser: bell
/// requests, replies owed to the child for terminal queries, plus state the
/// parser itself doesn't model (window title, OSC 52 clipboard writes,
/// DECSCUSR cursor shape, focus reporting). Programs probe their terminal
/// and block waiting for an answer on stdin (reedline won't paint its prompt
/// until its `CSI 6n` cursor-position query is answered), so the hub must
/// respond on the real terminal's behalf.
#[derive(Default)]
struct TermEvents {
    audible: usize,
    replies: Vec<u8>,
    /// The host terminal's colors (from the attaching client's OSC probe),
    /// used to answer the child's OSC 10/11/4 color queries. `None` falls
    /// back to the standard VGA palette so probing children never hang.
    host_colors: Option<ekko_proto::TerminalColors>,
    /// Latest OSC 0/2 title since the last drain.
    title: Option<String>,
    /// Latest OSC 52 write since the last drain (still base64-encoded).
    clipboard_copy: Option<Vec<u8>>,
    /// DECSCUSR shape (0 = terminal default). Persistent, not drained.
    cursor_shape: u8,
    /// Mode 1004: the child wants focus-in/out reports. Persistent.
    focus_reporting: bool,
}

impl vt100::Callbacks for TermEvents {
    fn audible_bell(&mut self, _screen: &mut vt100::Screen) {
        self.audible += 1;
    }

    fn set_window_title(&mut self, _screen: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn copy_to_clipboard(&mut self, _screen: &mut vt100::Screen, _ty: &[u8], data: &[u8]) {
        self.clipboard_copy = Some(data.to_vec());
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first_param = params.first().and_then(|p| p.first()).copied();
        match (i1, c) {
            // DSR: operating status (5) and cursor position (6).
            (None, 'n') => match first_param {
                Some(5) => self.replies.extend_from_slice(b"\x1b[0n"),
                Some(6) => {
                    let (row, col) = screen.cursor_position();
                    let reply = format!("\x1b[{};{}R", row + 1, col + 1);
                    self.replies.extend_from_slice(reply.as_bytes());
                }
                _ => {}
            },
            // DECXCPR: extended cursor position (no page number).
            (Some(b'?'), 'n') if first_param == Some(6) => {
                let (row, col) = screen.cursor_position();
                let reply = format!("\x1b[?{};{}R", row + 1, col + 1);
                self.replies.extend_from_slice(reply.as_bytes());
            }
            // DA1: claim to be a VT102, like zellij.
            (None, 'c') => self.replies.extend_from_slice(b"\x1b[?6c"),
            // DA2: claim to be a tmux-ish virtual terminal.
            (Some(b'>'), 'c') => self.replies.extend_from_slice(b"\x1b[>84;0;0c"),
            // DECSCUSR: cursor shape (0-6), forwarded to the host terminal.
            (Some(b' '), 'q') => {
                self.cursor_shape = first_param.unwrap_or(0).min(6) as u8;
            }
            // DECSET/DECRST params vt100 doesn't model. The callback fires
            // once per unknown param but receives the full list, so scan it.
            (Some(b'?'), 'h') if params.iter().any(|p| p.first() == Some(&1004)) => {
                self.focus_reporting = true;
            }
            (Some(b'?'), 'l') if params.iter().any(|p| p.first() == Some(&1004)) => {
                self.focus_reporting = false;
            }
            _ => {}
        }
    }

    fn unhandled_osc(&mut self, _screen: &mut vt100::Screen, params: &[&[u8]]) {
        // Color queries (OSC 10/11 default fg/bg, OSC 4 palette). Programs
        // probe these at startup to theme themselves (nested ekko, phi,
        // neovim's background detection); vt100 doesn't model colors, so the
        // hub answers on the host terminal's behalf from the colors the
        // attaching client probed. Replies are ST-terminated; probers accept
        // BEL and ST alike.
        match params {
            [b"10", b"?"] => {
                let (r, g, b) = self.host_foreground();
                let reply = format!("\x1b]10;{}\x1b\\", osc_color_reply_body(r, g, b));
                self.replies.extend_from_slice(reply.as_bytes());
            }
            [b"11", b"?"] => {
                let (r, g, b) = self.host_background();
                let reply = format!("\x1b]11;{}\x1b\\", osc_color_reply_body(r, g, b));
                self.replies.extend_from_slice(reply.as_bytes());
            }
            [b"4", rest @ ..] => {
                // OSC 4 batches pairs: `4;idx;?;idx;?;...`.
                for pair in rest.chunks_exact(2) {
                    let [idx_bytes, b"?"] = pair else { continue };
                    let Some(idx) = std::str::from_utf8(idx_bytes)
                        .ok()
                        .and_then(|s| s.parse::<u8>().ok())
                    else {
                        continue;
                    };
                    let Some((r, g, b)) = self.host_palette_color(idx) else {
                        continue;
                    };
                    let reply = format!("\x1b]4;{};{}\x1b\\", idx, osc_color_reply_body(r, g, b));
                    self.replies.extend_from_slice(reply.as_bytes());
                }
            }
            _ => {}
        }
    }
}

impl TermEvents {
    fn host_foreground(&self) -> (u8, u8, u8) {
        self.host_colors
            .as_ref()
            .map(|c| c.foreground)
            .unwrap_or(FALLBACK_FOREGROUND)
    }

    fn host_background(&self) -> (u8, u8, u8) {
        self.host_colors
            .as_ref()
            .map(|c| c.background)
            .unwrap_or(FALLBACK_BACKGROUND)
    }

    /// Palette color for `idx`. Indices 0-15 come from the host's probed
    /// palette (VGA fallback per-entry); higher indices aren't tracked and
    /// yield `None` (no reply, matching a terminal without OSC 4 support).
    fn host_palette_color(&self, idx: u8) -> Option<(u8, u8, u8)> {
        if idx >= 16 {
            return None;
        }
        let i = idx as usize;
        Some(
            self.host_colors
                .as_ref()
                .and_then(|c| c.palette[i])
                .unwrap_or(FALLBACK_PALETTE[i]),
        )
    }
}

/// XParseColor-style reply body, 16-bit per channel like xterm reports.
fn osc_color_reply_body(r: u8, g: u8, b: u8) -> String {
    let expand = |c: u8| u16::from(c) * 0x0101;
    format!("rgb:{:04x}/{:04x}/{:04x}", expand(r), expand(g), expand(b))
}

/// VGA defaults, mirroring `ekko_tui::default_terminal_colors`; used when the
/// outermost client's own probe got no answer (SSH, CI).
const FALLBACK_BACKGROUND: (u8, u8, u8) = (0x00, 0x00, 0x00);
const FALLBACK_FOREGROUND: (u8, u8, u8) = (0xc0, 0xc0, 0xc0);
const FALLBACK_PALETTE: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0x80, 0x00, 0x00),
    (0x00, 0x80, 0x00),
    (0x80, 0x80, 0x00),
    (0x00, 0x00, 0x80),
    (0x80, 0x00, 0x80),
    (0x00, 0x80, 0x80),
    (0xc0, 0xc0, 0xc0),
    (0x80, 0x80, 0x80),
    (0xff, 0x00, 0x00),
    (0x00, 0xff, 0x00),
    (0xff, 0xff, 0x00),
    (0x00, 0x00, 0xff),
    (0xff, 0x00, 0xff),
    (0x00, 0xff, 0xff),
    (0xff, 0xff, 0xff),
];

/// Remove any embedded bracketed-paste end markers from a paste payload so
/// the wrapped paste can't be broken out of.
fn strip_paste_end_markers(bytes: &[u8]) -> Vec<u8> {
    const END: &[u8] = b"\x1b[201~";
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(END) {
            i += END.len();
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn parse_client_thread_id(thread_name: &str) -> Option<ClientId> {
    thread_name
        .rsplit_once('-')
        .and_then(|(_, id)| id.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_end_markers_are_stripped() {
        assert_eq!(
            strip_paste_end_markers(b"safe\x1b[201~rm -rf /\x1b[201~x"),
            b"saferm -rf /x".to_vec()
        );
        assert_eq!(strip_paste_end_markers(b"plain"), b"plain".to_vec());
    }

    #[test]
    fn term_events_track_cursor_shape_and_focus_reporting() {
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 0, TermEvents::default());
        parser.process(b"\x1b[5 q\x1b[?1004h");
        assert_eq!(parser.callbacks_mut().cursor_shape, 5);
        assert!(parser.callbacks_mut().focus_reporting);
        parser.process(b"\x1b[?1004l\x1b[0 q");
        assert_eq!(parser.callbacks_mut().cursor_shape, 0);
        assert!(!parser.callbacks_mut().focus_reporting);
    }

    #[test]
    fn term_events_answer_osc_color_queries_with_host_colors() {
        let mut parser = vt100::Parser::new_with_callbacks(
            24,
            80,
            0,
            TermEvents {
                host_colors: Some(ekko_proto::TerminalColors {
                    background: (0x1e, 0x1e, 0x2e),
                    foreground: (0xcd, 0xd6, 0xf4),
                    palette: {
                        let mut palette = [None; 16];
                        palette[1] = Some((0xf3, 0x8b, 0xa8));
                        palette
                    },
                }),
                ..TermEvents::default()
            },
        );
        parser.process(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b]4;1;?\x1b\\");
        let replies = std::mem::take(&mut parser.callbacks_mut().replies);
        let replies = String::from_utf8(replies).unwrap();
        assert_eq!(
            replies,
            "\x1b]10;rgb:cdcd/d6d6/f4f4\x1b\\\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\\x1b]4;1;rgb:f3f3/8b8b/a8a8\x1b\\"
        );
    }

    #[test]
    fn term_events_answer_osc_color_queries_with_vga_fallback() {
        // No host colors (outermost client's probe failed): the child still
        // gets an answer so it never stalls on its own probe timeout.
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 0, TermEvents::default());
        parser.process(b"\x1b]11;?\x1b\\\x1b]4;9;?\x07");
        let replies = std::mem::take(&mut parser.callbacks_mut().replies);
        let replies = String::from_utf8(replies).unwrap();
        assert_eq!(
            replies,
            "\x1b]11;rgb:0000/0000/0000\x1b\\\x1b]4;9;rgb:ffff/0000/0000\x1b\\"
        );
    }

    #[test]
    fn term_events_ignore_osc_color_sets_and_out_of_range_queries() {
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 0, TermEvents::default());
        // Setting a color (not a query) and querying an untracked index
        // must produce no reply.
        parser.process(b"\x1b]10;#ffffff\x1b\\\x1b]4;42;?\x1b\\");
        assert!(parser.callbacks_mut().replies.is_empty());
    }

    #[test]
    fn term_events_capture_title_and_clipboard() {
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 0, TermEvents::default());
        parser.process(b"\x1b]2;my title\x07\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(parser.callbacks_mut().title.take().unwrap(), "my title");
        assert_eq!(
            parser.callbacks_mut().clipboard_copy.take().unwrap(),
            b"aGVsbG8=".to_vec()
        );
    }
}
