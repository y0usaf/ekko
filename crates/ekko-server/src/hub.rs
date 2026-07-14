//! The hub: single-threaded owner of session routing, clients, and a one-entry
//! terminal-pane map. Everything else talks to it exclusively through
//! [`HubInstruction`] messages.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use ekko_config::Config;
use ekko_proto::{
    AttachRejectReason, ClientToServer, ExitReason, GridPayload, GridUpdate, ServerNotice,
    ServerToClient,
};
use ekko_pty::{PtyCommand, WinSize};
use interprocess::local_socket::Stream as LocalSocketStream;

use ekko_event::{EventKind, EventPayload, EventReturn, SessionExitReason};
use ekko_ext::AppRuntime;

use crate::client_io::{self, ClientHandle, ClientId};
use crate::terminal_pane::{PaneGeneration, PaneId, PaneKey, TerminalPane};

/// How often the `Heartbeat` lifecycle event fires (the resurrection
/// builtin uses it to refresh the manifest's `last_active_secs`).
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

pub enum HubInstruction {
    NewClient(LocalSocketStream),
    ClientMsg(ClientId, ClientToServer),
    ClientDisconnected(ClientId),
    ClientWriteFailed(ClientId),
    PtyBytes {
        pane: PaneKey,
        bytes: Vec<u8>,
    },
    PtyExited {
        pane: PaneKey,
        code: Option<i32>,
    },
    PtyThreadPanicked {
        pane: PaneKey,
        thread_name: String,
        message: String,
    },
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
    /// First successful client probe; copied into the live pane parser.
    host_colors: Option<ekko_proto::TerminalColors>,
    /// P1 keeps exactly one stable ID in this map. Generation distinguishes
    /// a retired incarnation from any later reuse of that ID.
    panes: HashMap<PaneId, TerminalPane>,
    primary_pane_id: PaneId,
    next_pane_generation: u64,
    epoch: u64,
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
        Self {
            session_name,
            config,
            runtime,
            hub_tx,
            clients: HashMap::new(),
            attached: HashMap::new(),
            next_client_id: 0,
            host_colors: None,
            panes: HashMap::new(),
            primary_pane_id: PaneId(1),
            next_pane_generation: 1,
            epoch: 0,
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
            let received = match self.primary_pane().and_then(TerminalPane::render_deadline) {
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

    fn primary_pane(&self) -> Option<&TerminalPane> {
        self.panes.get(&self.primary_pane_id)
    }

    fn primary_pane_mut(&mut self) -> Option<&mut TerminalPane> {
        self.panes.get_mut(&self.primary_pane_id)
    }

    fn mark_dirty(&mut self) {
        if let Some(pane) = self.primary_pane_mut() {
            pane.mark_dirty();
        }
    }

    fn next_pane_key(&mut self) -> PaneKey {
        let key = PaneKey {
            id: self.primary_pane_id,
            generation: PaneGeneration(self.next_pane_generation),
        };
        self.next_pane_generation = self
            .next_pane_generation
            .checked_add(1)
            .expect("pane generation exhausted");
        key
    }

    fn retire_primary_pane(&mut self, terminate_child: bool) {
        if let Some(pane) = self.panes.remove(&self.primary_pane_id) {
            pane.retire(terminate_child);
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
            HubInstruction::PtyBytes { pane, mut bytes } => self.on_pty_bytes(pane, &mut bytes),
            HubInstruction::PtyExited { pane, code } => self.on_pty_exited(pane, code),
            HubInstruction::PtyThreadPanicked {
                pane,
                thread_name,
                message,
            } => self.on_pty_thread_panicked(pane, thread_name, message),
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
        if let Some(colors) = terminal_colors
            && self.host_colors.is_none()
        {
            self.host_colors = Some(colors.clone());
            if let Some(pane) = self.primary_pane_mut() {
                pane.set_host_colors(colors);
            }
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

        if self.panes.is_empty() {
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
        if let Some(pane) = self.primary_pane_mut() {
            pane.force_dirty();
        }
        self.render_now();
        let (cols, rows) = self
            .primary_pane()
            .map(TerminalPane::size)
            .expect("attached session has a terminal pane");
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
        let Some((cols_now, rows_now)) = self.primary_pane().map(TerminalPane::size) else {
            return;
        };
        if (cols, rows) != (cols_now, rows_now) {
            self.resize(cols, rows);
            self.mark_dirty();
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        if let Some(pane) = self.primary_pane_mut() {
            pane.resize(cols, rows);
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
        let app_cursor = self
            .primary_pane()
            .is_some_and(TerminalPane::application_cursor);
        crate::input_compat::rewrite_cursor_keys(&mut bytes, app_cursor);
        self.write_to_pty(id, bytes);
    }

    fn write_to_pty(&mut self, id: ClientId, bytes: Vec<u8>) {
        if !self.attached.contains_key(&id) {
            return;
        }
        // Typing jumps the view back to the live screen, tmux-style.
        let scrolled = self
            .primary_pane()
            .is_some_and(|pane| pane.scrollback() > 0);
        if scrolled {
            if let Some(pane) = self.primary_pane_mut() {
                pane.set_scrollback(0);
            }
            self.mark_dirty();
        }
        if let Some(pane) = self.primary_pane() {
            pane.write(bytes);
        }
    }

    /// Forward pasted content, re-wrapped in bracketed-paste markers when the
    /// child has requested them (the client strips the host's markers). Any
    /// end marker embedded in the payload is removed so a malicious paste
    /// can't break out of the bracket.
    fn on_paste(&mut self, id: ClientId, bytes: Vec<u8>) {
        let bytes = if self
            .primary_pane()
            .is_some_and(TerminalPane::bracketed_paste)
        {
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
        let current = self
            .primary_pane()
            .map_or(0, |pane| pane.scrollback() as i64);
        let target = (current + i64::from(delta)).max(0) as usize;
        self.set_scrollback_view(id, target);
    }

    fn set_scrollback_view(&mut self, id: ClientId, rows: usize) {
        if !self.attached.contains_key(&id) {
            return;
        }
        let Some(pane) = self.primary_pane_mut() else {
            return;
        };
        // There is no history to scroll on the alternate screen.
        if pane.alternate_screen() && rows > 0 {
            return;
        }
        let before = pane.scrollback();
        pane.set_scrollback(rows);
        if pane.scrollback() != before {
            pane.mark_dirty();
        }
    }

    fn on_kill_session(&mut self, id: ClientId, name: &str) {
        if name != self.session_name {
            log::warn!("hub: ignoring KillSession for foreign session {name}");
            return;
        }
        self.send_to(id, ServerToClient::Exit(ExitReason::Normal));
        self.fire_session_exited(None, SessionExitReason::Killed);
        self.retire_primary_pane(true);
        self.finish_exit();
    }

    // -- PTY events ---------------------------------------------------------

    fn event_is_current(&self, key: PaneKey) -> bool {
        self.panes
            .get(&key.id)
            .is_some_and(|pane| pane.key() == key)
    }

    fn on_pty_bytes(&mut self, key: PaneKey, bytes: &mut [u8]) {
        let Some(pane) = self.panes.get_mut(&key.id) else {
            return;
        };
        if pane.key() != key {
            return;
        }
        let output = pane.process_bytes(bytes);
        pane.mark_dirty();

        if output.bells > 0 {
            self.send_to_attached(ServerToClient::Bell);
            self.dispatch(
                EventKind::Bell,
                EventPayload::Bell {
                    session_name: self.session_name.clone(),
                },
            );
        }
        if let Some(title) = output.title {
            self.send_to_attached(ServerToClient::Title(title));
        }
        if let Some(data) = output.clipboard_copy {
            self.send_to_attached(ServerToClient::ClipboardCopy(data));
        }
    }

    fn on_pty_exited(&mut self, key: PaneKey, code: Option<i32>) {
        if !self.event_is_current(key) {
            return;
        }
        log::info!("hub: pane {key:?} shell exited with code {code:?}");
        self.send_to_attached(ServerToClient::Exit(ExitReason::SessionExited(code)));
        self.fire_session_exited(code, SessionExitReason::ShellExited);
        self.retire_primary_pane(false);
        self.finish_exit();
    }

    fn on_pty_thread_panicked(&mut self, key: PaneKey, thread_name: String, message: String) {
        if !self.event_is_current(key) {
            return;
        }
        log::error!("hub: pane {key:?} thread '{thread_name}' panicked: {message}");
        self.send_to_attached(ServerToClient::Exit(ExitReason::ServerError(message)));
        self.crashed = true;
        self.fire_session_exited(None, SessionExitReason::Crashed);
        self.retire_primary_pane(true);
        self.finish_exit();
    }

    fn on_thread_panicked(&mut self, thread_name: String, message: String) {
        log::error!("hub: thread '{thread_name}' panicked: {message}");
        self.send_to_attached(ServerToClient::Exit(ExitReason::ServerError(message)));
        if let Some(id) = parse_client_thread_id(&thread_name) {
            self.on_client_disconnected(id);
        }
    }

    fn on_shutdown_signal(&mut self) {
        log::info!("hub: received shutdown signal");
        self.send_to_attached(ServerToClient::Exit(ExitReason::Normal));
        self.fire_session_exited(None, SessionExitReason::Shutdown);
        self.retire_primary_pane(true);
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
        let clients_need_full = !self.needs_full.is_empty();
        let Some(frame) = self
            .primary_pane_mut()
            .and_then(|pane| pane.prepare_render(clients_need_full))
        else {
            return;
        };

        self.epoch += 1;
        let mut dropped: Vec<ClientId> = Vec::new();
        for (&id, client) in &self.clients {
            if !self.attached.contains_key(&id) {
                continue;
            }
            let payload = if frame.full_for_all || self.needs_full.contains(&id) {
                GridPayload::Full(frame.rows.clone())
            } else if frame.steady {
                continue;
            } else {
                GridPayload::Rows(frame.patches.clone())
            };
            let update = GridUpdate {
                epoch: self.epoch,
                cols: frame.size.0,
                rows: frame.size.1,
                cursor: Some(frame.cursor),
                modes: frame.modes,
                scrollback: frame.scrollback,
                payload,
            };
            if client.tx.try_send(ServerToClient::Grid(update)).is_err() {
                log::debug!("hub: client {id}'s queue dropped a grid frame; full resync queued");
                dropped.push(id);
            }
        }
        self.needs_full = dropped.into_iter().collect();

        if let Some(pane) = self.panes.get_mut(&frame.pane.id) {
            pane.commit_render(&frame);
        }
    }

    // -- pty / session bookkeeping ---------------------------------------

    fn spawn_pty(
        &mut self,
        cwd: &std::path::Path,
        shell: &std::path::Path,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<()> {
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

        let key = self.next_pane_key();
        let exit_tx = self.hub_tx.clone();
        let mut cmd = PtyCommand::new(&shell).cwd(&cwd);
        cmd.env = env;
        let handle = ekko_pty::spawn_pty(
            cmd,
            WinSize { cols, rows },
            Box::new(move |code| {
                let _ = exit_tx.send(HubInstruction::PtyExited { pane: key, code });
            }),
        )?;
        let pane = TerminalPane::from_pty_handle(
            key,
            handle,
            rows,
            cols,
            self.config.general.scrollback_lines,
            self.host_colors.clone(),
            self.hub_tx.clone(),
        )?;
        debug_assert!(self.panes.is_empty());
        self.panes.insert(key.id, pane);
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
        self.retire_primary_pane(true);
        let _ = std::fs::remove_file(ekko_proto::socket_path(&self.session_name));
    }
}

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
    fn stale_generation_bytes_exits_and_panics_are_ignored() {
        let (hub_tx, _hub_rx) = crossbeam_channel::unbounded();
        let mut hub = Hub::new(
            "test".to_string(),
            Config::default(),
            hub_tx.clone(),
            AppRuntime::empty(),
        );
        let current = PaneKey {
            id: hub.primary_pane_id,
            generation: PaneGeneration(2),
        };
        let stale = PaneKey {
            id: current.id,
            generation: PaneGeneration(1),
        };
        let (pane, _peer) = TerminalPane::test_pane(current, hub_tx);
        hub.panes.insert(current.id, pane);

        hub.handle(HubInstruction::PtyBytes {
            pane: stale,
            bytes: b"stale".to_vec(),
        });
        hub.handle(HubInstruction::PtyExited {
            pane: stale,
            code: Some(9),
        });
        hub.handle(HubInstruction::PtyThreadPanicked {
            pane: stale,
            thread_name: "pty-reader-p1-g1".to_string(),
            message: "stale panic".to_string(),
        });

        assert_eq!(hub.panes.len(), 1);
        assert!(!hub.should_exit);
        assert_eq!(hub.primary_pane().unwrap().first_cell_for_test(), ' ');

        hub.primary_pane().unwrap().reserve_backlog_for_test(1);
        hub.handle(HubInstruction::PtyBytes {
            pane: current,
            bytes: b"x".to_vec(),
        });
        assert_eq!(hub.primary_pane().unwrap().first_cell_for_test(), 'x');
    }
}
