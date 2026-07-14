//! The hub: single-threaded owner of session routing, canonical pane topology,
//! per-client focus, and terminal-pane resources. Everything else talks to it
//! exclusively through [`HubInstruction`] messages.

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
use crate::topology::{Direction, PaneTopology, Rect, SplitAxis, SplitRatio};

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
    /// First successful client probe; copied into every live pane parser.
    host_colors: Option<ekko_proto::TerminalColors>,
    panes: HashMap<PaneId, TerminalPane>,
    topology: Option<PaneTopology>,
    /// Focus is daemon-owned and exists only while a client is attached.
    focus: HashMap<ClientId, PaneId>,
    canvas_size: Option<(u16, u16)>,
    session_cwd: Option<PathBuf>,
    next_pane_id: u64,
    next_pane_generation: u64,
    epoch: u64,
    /// Clients whose outgoing queue dropped a grid frame; they get a `Full`
    /// resync on the next broadcast since their state may have diverged.
    needs_full: HashSet<ClientId>,
    should_exit: bool,
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
            topology: None,
            focus: HashMap::new(),
            canvas_size: None,
            session_cwd: None,
            next_pane_id: 1,
            next_pane_generation: 1,
            epoch: 0,
            needs_full: HashSet::new(),
            should_exit: false,
        }
    }

    /// Runs the hub loop until a shutdown condition is reached. Always
    /// returns `Ok(())`; failures along the way are logged, not propagated,
    /// since a single-session daemon has nowhere else to report them.
    pub fn run(mut self, rx: Receiver<HubInstruction>) {
        loop {
            let received = match self.next_render_deadline() {
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

    fn next_render_deadline(&self) -> Option<Instant> {
        self.panes
            .values()
            .filter_map(TerminalPane::render_deadline)
            .min()
    }

    fn topology(&self) -> Option<&PaneTopology> {
        self.topology.as_ref()
    }

    #[allow(dead_code)] // P2 internal seam; P3 adds the wire caller.
    fn canvas(&self) -> Option<Rect> {
        self.canvas_size.map(|(cols, rows)| Rect {
            x: 0,
            y: 0,
            cols,
            rows,
        })
    }

    fn focused_pane_id(&self, client: ClientId) -> Option<PaneId> {
        self.focus
            .get(&client)
            .copied()
            .filter(|pane| self.panes.contains_key(pane))
    }

    fn focused_pane(&self, client: ClientId) -> Option<&TerminalPane> {
        self.focused_pane_id(client)
            .and_then(|pane| self.panes.get(&pane))
    }

    fn focused_pane_mut(&mut self, client: ClientId) -> Option<&mut TerminalPane> {
        let pane = self.focused_pane_id(client)?;
        self.panes.get_mut(&pane)
    }

    fn next_pane_key(&mut self) -> PaneKey {
        let key = PaneKey {
            id: PaneId(self.next_pane_id),
            generation: PaneGeneration(self.next_pane_generation),
        };
        self.next_pane_id = self.next_pane_id.checked_add(1).expect("pane ID exhausted");
        self.next_pane_generation = self
            .next_pane_generation
            .checked_add(1)
            .expect("pane generation exhausted");
        key
    }

    fn retire_pane(&mut self, pane_id: PaneId, terminate_child: bool) {
        if let Some(pane) = self.panes.remove(&pane_id) {
            pane.retire(terminate_child);
        }
    }

    fn retire_all_panes(&mut self, terminate_child: bool) {
        let panes = std::mem::take(&mut self.panes);
        self.topology = None;
        self.focus.clear();
        for (_, pane) in panes {
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
        self.focus.remove(&id);
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

    fn send_to_pane_viewers(&self, pane: PaneId, msg: ServerToClient) {
        for (&id, &focused) in &self.focus {
            if focused == pane && self.attached.contains_key(&id) {
                self.send_to(id, msg.clone());
            }
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
            for pane in self.panes.values_mut() {
                pane.set_host_colors(colors.clone());
            }
        }
        if force {
            let others: Vec<ClientId> =
                self.attached.keys().copied().filter(|&c| c != id).collect();
            for other in others {
                log::info!("hub: client {id} kicked client {other} (attach --force)");
                self.send_to(other, ServerToClient::Exit(ExitReason::Kicked));
                self.attached.remove(&other);
                self.focus.remove(&other);
                self.needs_full.remove(&other);
            }
        }

        if self.panes.is_empty() {
            let shell = shell.unwrap_or_else(|| self.config.resolve_shell());
            match self.spawn_terminal(&cwd, &shell, cols, rows) {
                Ok(pane) => {
                    let pane_id = pane.key().id;
                    self.panes.insert(pane_id, pane);
                    self.topology = Some(PaneTopology::new(pane_id));
                    self.canvas_size = Some((cols, rows));
                    self.session_cwd = Some(cwd.clone());
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
        let focus = self
            .topology()
            .and_then(PaneTopology::first_leaf)
            .expect("attached session has a topology leaf");
        self.focus.insert(id, focus);
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
        if let Some(pane) = self.focused_pane_mut(id) {
            pane.force_dirty();
        }
        self.render_now();
        let (cols, rows) = self
            .focused_pane(id)
            .map(TerminalPane::size)
            .expect("attached session has a focused terminal pane");
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
        self.focus.remove(&id);
        self.needs_full.remove(&id);
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

    /// Resize the canonical canvas to the smallest attached client. Detached
    /// sessions keep their last geometry. A multi-pane canvas that cannot
    /// preserve minimum child dimensions is left at its last valid size.
    fn resize_to_fit(&mut self) {
        let Some((cols, rows)) = self
            .attached
            .values()
            .copied()
            .reduce(|(c1, r1), (c2, r2)| (c1.min(c2), r1.min(r2)))
        else {
            return;
        };
        if self.canvas_size == Some((cols, rows)) {
            return;
        }
        if let Err(error) = self.resize_canvas(cols, rows) {
            log::warn!("hub: rejecting {cols}x{rows} canvas resize: {error:?}");
        }
    }

    fn resolved_geometry(
        &self,
        canvas: Rect,
    ) -> Result<Vec<(PaneId, Rect)>, crate::topology::TopologyError> {
        let topology = self
            .topology()
            .ok_or(crate::topology::TopologyError::MissingLeaf)?;
        if topology.len() == 1 {
            topology.resolve(canvas)
        } else {
            topology.resolve_viable(canvas)
        }
    }

    fn resize_canvas(
        &mut self,
        cols: u16,
        rows: u16,
    ) -> Result<(), crate::topology::TopologyError> {
        let geometry = self.resolved_geometry(Rect {
            x: 0,
            y: 0,
            cols,
            rows,
        })?;
        self.canvas_size = Some((cols, rows));
        let mut resized = Vec::new();
        for (pane_id, rect) in geometry {
            let Some(pane) = self.panes.get_mut(&pane_id) else {
                continue;
            };
            if pane.size() != (rect.cols, rect.rows) {
                pane.resize(rect.cols, rect.rows);
                pane.mark_dirty();
                resized.push((rect.cols, rect.rows));
            }
        }
        for (cols, rows) in resized {
            self.dispatch(
                EventKind::PtyResized,
                EventPayload::PtyResized {
                    session_name: self.session_name.clone(),
                    cols,
                    rows,
                },
            );
        }
        Ok(())
    }

    fn on_input(&mut self, id: ClientId, mut bytes: Vec<u8>) {
        // Cursor keys arrive encoded for the host terminal's DECCKM state;
        // re-encode them for the child's (see `input_compat`).
        let app_cursor = self
            .focused_pane(id)
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
            .focused_pane(id)
            .is_some_and(|pane| pane.scrollback() > 0);
        if let Some(pane) = self.focused_pane_mut(id) {
            if scrolled {
                pane.set_scrollback(0);
                pane.mark_dirty();
            }
            pane.write(bytes);
        }
    }

    /// Forward pasted content, re-wrapped in bracketed-paste markers when the
    /// child has requested them (the client strips the host's markers). Any
    /// end marker embedded in the payload is removed so a malicious paste
    /// can't break out of the bracket.
    fn on_paste(&mut self, id: ClientId, bytes: Vec<u8>) {
        let bytes = if self
            .focused_pane(id)
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
            .focused_pane(id)
            .map_or(0, |pane| pane.scrollback() as i64);
        let target = (current + i64::from(delta)).max(0) as usize;
        self.set_scrollback_view(id, target);
    }

    fn set_scrollback_view(&mut self, id: ClientId, rows: usize) {
        if !self.attached.contains_key(&id) {
            return;
        }
        let Some(pane) = self.focused_pane_mut(id) else {
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
        self.retire_all_panes(true);
        self.finish_exit();
    }

    // -- canonical pane operations -----------------------------------------

    /// Transactionally split one client's focused leaf. Geometry is resolved
    /// before identity allocation, extension dispatch, or PTY spawn; only a
    /// fully wired child is inserted and committed to the canonical tree.
    #[allow(dead_code)] // P2 internal seam; P3 adds the wire caller.
    fn split_focused(
        &mut self,
        client: ClientId,
        axis: SplitAxis,
    ) -> anyhow::Result<Option<PaneId>> {
        if !self.attached.contains_key(&client) {
            return Ok(None);
        }
        let Some(target) = self.focused_pane_id(client) else {
            return Ok(None);
        };
        let Some(canvas) = self.canvas() else {
            return Ok(None);
        };
        let candidate = PaneId(self.next_pane_id);
        let Some(topology) = self.topology() else {
            return Ok(None);
        };
        let proposed = match topology.with_split(target, candidate, axis, SplitRatio::HALF) {
            Ok(proposed) => proposed,
            Err(_) => return Ok(None),
        };
        let geometry = match proposed.resolve_viable(canvas) {
            Ok(geometry) => geometry,
            Err(_) => return Ok(None),
        };
        let child_rect = geometry
            .iter()
            .find_map(|(pane, rect)| (*pane == candidate).then_some(*rect))
            .expect("proposed split contains its child");

        let cwd = self
            .session_cwd
            .clone()
            .expect("live pane set has a session cwd");
        let shell = self.config.resolve_shell();
        let pane = self.spawn_terminal(&cwd, &shell, child_rect.cols, child_rect.rows)?;
        let key = pane.key();
        debug_assert_eq!(key.id, candidate);

        self.panes.insert(key.id, pane);
        self.topology = Some(proposed);
        self.focus.insert(client, key.id);
        self.needs_full.insert(client);
        self.resize_canvas(canvas.cols, canvas.rows)
            .expect("prevalidated split geometry remains viable");
        if let Some(pane) = self.panes.get_mut(&key.id) {
            pane.force_dirty();
        }
        Ok(Some(key.id))
    }

    #[allow(dead_code)] // P2 internal seam; P3 adds the wire caller.
    fn focus_direction(&mut self, client: ClientId, direction: Direction) -> bool {
        if !self.attached.contains_key(&client) {
            return false;
        }
        let Some(current) = self.focused_pane_id(client) else {
            return false;
        };
        let Some(canvas) = self.canvas() else {
            return false;
        };
        let Some(next) = self
            .topology()
            .and_then(|topology| topology.neighbor(current, direction, canvas))
        else {
            return false;
        };
        self.focus.insert(client, next);
        self.needs_full.insert(client);
        if let Some(pane) = self.panes.get_mut(&next) {
            pane.force_dirty();
        }
        true
    }

    #[allow(dead_code)] // P2 internal seam; P3 adds the wire caller.
    fn close_focused(&mut self, client: ClientId) -> bool {
        if !self.attached.contains_key(&client) {
            return false;
        }
        let Some(pane) = self.focused_pane_id(client) else {
            return false;
        };
        if self.panes.len() > 1 {
            self.remove_non_last_pane(pane, true);
        } else {
            self.send_to_attached(ServerToClient::Exit(ExitReason::SessionExited(None)));
            self.fire_session_exited(None, SessionExitReason::ShellExited);
            self.retire_all_panes(true);
            self.finish_exit();
        }
        true
    }

    fn remove_non_last_pane(&mut self, pane_id: PaneId, terminate_child: bool) {
        debug_assert!(self.panes.len() > 1);
        let topology = self
            .topology
            .as_mut()
            .expect("multiple panes have a topology");
        if !topology.remove(pane_id) {
            return;
        }
        self.retire_pane(pane_id, terminate_child);

        let fallback = self
            .topology()
            .and_then(PaneTopology::first_leaf)
            .expect("removing a non-last pane leaves a sibling");
        let invalid_clients: Vec<ClientId> = self
            .focus
            .iter()
            .filter_map(|(&client, &focused)| {
                (!self.panes.contains_key(&focused)).then_some(client)
            })
            .collect();
        for client in invalid_clients {
            self.focus.insert(client, fallback);
            self.needs_full.insert(client);
        }
        if let Some((cols, rows)) = self.canvas_size {
            self.resize_canvas(cols, rows)
                .expect("removal cannot make child geometry less viable");
        }
        for client in self.focus.keys().copied().collect::<Vec<_>>() {
            if let Some(pane) = self.focused_pane_mut(client) {
                pane.force_dirty();
            }
        }
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
            self.send_to_pane_viewers(key.id, ServerToClient::Bell);
            self.dispatch(
                EventKind::Bell,
                EventPayload::Bell {
                    session_name: self.session_name.clone(),
                },
            );
        }
        if let Some(title) = output.title {
            self.send_to_pane_viewers(key.id, ServerToClient::Title(title));
        }
        if let Some(data) = output.clipboard_copy {
            self.send_to_pane_viewers(key.id, ServerToClient::ClipboardCopy(data));
        }
    }

    fn on_pty_exited(&mut self, key: PaneKey, code: Option<i32>) {
        if !self.event_is_current(key) {
            return;
        }
        log::info!("hub: pane {key:?} shell exited with code {code:?}");
        if self.panes.len() > 1 {
            self.remove_non_last_pane(key.id, false);
            return;
        }
        self.send_to_attached(ServerToClient::Exit(ExitReason::SessionExited(code)));
        self.fire_session_exited(code, SessionExitReason::ShellExited);
        self.retire_all_panes(false);
        self.finish_exit();
    }

    fn on_pty_thread_panicked(&mut self, key: PaneKey, thread_name: String, message: String) {
        if !self.event_is_current(key) {
            return;
        }
        log::error!("hub: pane {key:?} thread '{thread_name}' panicked: {message}");
        if self.panes.len() > 1 {
            self.remove_non_last_pane(key.id, true);
            return;
        }
        self.send_to_attached(ServerToClient::Exit(ExitReason::ServerError(message)));
        self.fire_session_exited(None, SessionExitReason::Crashed);
        self.retire_all_panes(true);
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
        self.retire_all_panes(true);
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
        let pane_ids = self
            .topology()
            .map(PaneTopology::leaves)
            .unwrap_or_default();
        for pane_id in pane_ids {
            let clients_need_full = self.needs_full.iter().any(|client| {
                self.focus.get(client).copied() == Some(pane_id)
                    && self.attached.contains_key(client)
            });
            let Some(frame) = self
                .panes
                .get_mut(&pane_id)
                .and_then(|pane| pane.prepare_render(clients_need_full))
            else {
                continue;
            };

            self.epoch += 1;
            for (&id, client) in &self.clients {
                if !self.attached.contains_key(&id) || self.focus.get(&id).copied() != Some(pane_id)
                {
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
                    log::debug!(
                        "hub: client {id}'s queue dropped a grid frame; full resync queued"
                    );
                    self.needs_full.insert(id);
                } else {
                    self.needs_full.remove(&id);
                }
            }

            if let Some(pane) = self.panes.get_mut(&frame.pane.id) {
                pane.commit_render(&frame);
            }
        }
    }

    // -- pty / session bookkeeping ---------------------------------------

    fn spawn_terminal(
        &mut self,
        cwd: &std::path::Path,
        shell: &std::path::Path,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<TerminalPane> {
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
        TerminalPane::from_pty_handle(
            key,
            handle,
            rows,
            cols,
            self.config.general.scrollback_lines,
            self.host_colors.clone(),
            self.hub_tx.clone(),
        )
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
        self.retire_all_panes(true);
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
    use std::time::Duration;

    use super::*;

    struct LiveHub {
        hub: Hub,
        rx: Receiver<HubInstruction>,
        _cwd: tempfile::TempDir,
    }

    impl LiveHub {
        fn new(cols: u16, rows: u16, clients: &[ClientId]) -> Self {
            let (hub_tx, rx) = crossbeam_channel::unbounded();
            let mut config = Config::default();
            config.general.default_shell = "/bin/sh".to_string();
            let mut hub = Hub::new("test".to_string(), config, hub_tx, AppRuntime::empty());
            let cwd = tempfile::tempdir().unwrap();
            let pane = hub
                .spawn_terminal(cwd.path(), std::path::Path::new("/bin/sh"), cols, rows)
                .unwrap();
            let pane_id = pane.key().id;
            hub.panes.insert(pane_id, pane);
            hub.topology = Some(PaneTopology::new(pane_id));
            hub.canvas_size = Some((cols, rows));
            hub.session_cwd = Some(cwd.path().to_path_buf());
            for &client in clients {
                hub.attached.insert(client, (cols, rows));
                hub.focus.insert(client, pane_id);
            }
            Self { hub, rx, _cwd: cwd }
        }

        fn assert_focus_is_live(&self) {
            assert_eq!(self.hub.focus.len(), self.hub.attached.len());
            assert!(
                self.hub
                    .focus
                    .values()
                    .all(|pane| self.hub.panes.contains_key(pane))
            );
        }
    }

    impl Drop for LiveHub {
        fn drop(&mut self) {
            self.hub.retire_all_panes(true);
        }
    }

    #[test]
    fn paste_end_markers_are_stripped() {
        assert_eq!(
            strip_paste_end_markers(b"safe\x1b[201~rm -rf /\x1b[201~x"),
            b"saferm -rf /x".to_vec()
        );
        assert_eq!(strip_paste_end_markers(b"plain"), b"plain".to_vec());
    }

    #[test]
    fn invalid_split_is_side_effect_free_and_spawns_no_child() {
        let mut live = LiveHub::new(3, 10, &[7]);
        let before_tree = live.hub.topology.clone();
        let before_focus = live.hub.focus.clone();
        let before_id = live.hub.next_pane_id;
        let before_generation = live.hub.next_pane_generation;

        assert_eq!(
            live.hub.split_focused(7, SplitAxis::Horizontal).unwrap(),
            None
        );

        assert_eq!(live.hub.topology, before_tree);
        assert_eq!(live.hub.focus, before_focus);
        assert_eq!(live.hub.next_pane_id, before_id);
        assert_eq!(live.hub.next_pane_generation, before_generation);
        assert_eq!(live.hub.panes.len(), 1, "no child PTY may leak");
        assert!(
            live.hub
                .panes
                .values()
                .all(TerminalPane::workers_live_for_test)
        );
    }

    #[test]
    fn successful_splits_have_monotonic_keys_live_resources_and_geometry_focus() {
        let mut live = LiveHub::new(80, 24, &[10, 20]);
        let initial = live.hub.focus[&10];
        let right = live
            .hub
            .split_focused(10, SplitAxis::Horizontal)
            .unwrap()
            .unwrap();
        assert_eq!(right.0, initial.0 + 1);
        assert_eq!(live.hub.focus[&10], right);
        assert_eq!(live.hub.focus[&20], initial);
        assert_eq!(live.hub.panes[&initial].size(), (40, 24));
        assert_eq!(live.hub.panes[&right].size(), (40, 24));
        assert!(live.hub.focus_direction(10, Direction::Left));
        assert_eq!(live.hub.focus[&10], initial);
        assert!(live.hub.focus_direction(10, Direction::Right));

        let down = live
            .hub
            .split_focused(10, SplitAxis::Vertical)
            .unwrap()
            .unwrap();
        assert_eq!(down.0, right.0 + 1);
        assert_eq!(live.hub.panes[&right].size(), (40, 12));
        assert_eq!(live.hub.panes[&down].size(), (40, 12));
        assert!(live.hub.focus_direction(10, Direction::Up));
        assert_eq!(live.hub.focus[&10], right);
        assert!(live.hub.focus_direction(10, Direction::Down));
        assert_eq!(live.hub.focus[&10], down);

        let keys: Vec<PaneKey> = live.hub.panes.values().map(TerminalPane::key).collect();
        assert_eq!(
            keys.iter().map(|key| key.id).collect::<HashSet<_>>().len(),
            3
        );
        assert_eq!(
            keys.iter()
                .map(|key| key.generation)
                .collect::<HashSet<_>>()
                .len(),
            3
        );
        assert!(
            live.hub
                .panes
                .values()
                .all(TerminalPane::workers_live_for_test)
        );
        live.assert_focus_is_live();
    }

    #[test]
    fn removal_repairs_every_focus_and_promotes_the_sibling() {
        let mut live = LiveHub::new(80, 24, &[10, 20]);
        let initial = live.hub.focus[&10];
        let child = live
            .hub
            .split_focused(10, SplitAxis::Horizontal)
            .unwrap()
            .unwrap();
        live.hub.focus.insert(20, child);

        assert!(live.hub.close_focused(10));

        assert_eq!(live.hub.topology().unwrap().leaves(), vec![initial]);
        assert_eq!(live.hub.panes.len(), 1);
        assert_eq!(live.hub.panes[&initial].size(), (80, 24));
        assert_eq!(live.hub.focus[&10], initial);
        assert_eq!(live.hub.focus[&20], initial);
        live.assert_focus_is_live();
    }

    #[test]
    fn non_last_child_exit_reclaims_only_that_pane_and_expands_sibling() {
        let mut live = LiveHub::new(80, 24, &[10, 20]);
        let initial = live.hub.focus[&10];
        let child = live
            .hub
            .split_focused(10, SplitAxis::Horizontal)
            .unwrap()
            .unwrap();
        live.hub.focus.insert(20, child);
        let child_key = live.hub.panes[&child].key();
        live.hub.panes[&child].write(b"exit\n".to_vec());

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_exit = false;
        while Instant::now() < deadline {
            let Ok(instruction) = live.rx.recv_timeout(Duration::from_millis(100)) else {
                continue;
            };
            match instruction {
                HubInstruction::PtyExited { pane, code } if pane == child_key => {
                    live.hub.on_pty_exited(pane, code);
                    saw_exit = true;
                    break;
                }
                other => live.hub.handle(other),
            }
        }
        assert!(saw_exit, "split child must report its shell exit");
        assert!(!live.hub.should_exit);
        assert_eq!(live.hub.panes.len(), 1);
        assert_eq!(live.hub.topology().unwrap().leaves(), vec![initial]);
        assert_eq!(live.hub.panes[&initial].size(), (80, 24));
        live.assert_focus_is_live();
    }

    #[test]
    fn attach_detach_disconnect_own_only_their_focus() {
        let mut live = LiveHub::new(80, 24, &[10, 20]);
        let initial = live.hub.focus[&10];
        let child = live
            .hub
            .split_focused(10, SplitAxis::Horizontal)
            .unwrap()
            .unwrap();
        assert_eq!(live.hub.focus[&10], child);
        assert_eq!(live.hub.focus[&20], initial);

        live.hub.on_detach(10);
        assert!(!live.hub.focus.contains_key(&10));
        assert_eq!(live.hub.focus[&20], initial);
        live.hub.on_client_disconnected(20);
        assert!(live.hub.focus.is_empty());
        assert_eq!(live.hub.panes.len(), 2, "detach preserves pane ownership");
    }

    #[test]
    fn pane_output_and_render_updates_reach_only_that_panes_viewers() {
        let mut live = LiveHub::new(80, 24, &[10, 20]);
        let initial = live.hub.focus[&10];
        let child = live
            .hub
            .split_focused(10, SplitAxis::Horizontal)
            .unwrap()
            .unwrap();
        assert_eq!(live.hub.focus[&20], initial);

        let (left_tx, left_rx) = crossbeam_channel::bounded(16);
        let (right_tx, right_rx) = crossbeam_channel::bounded(16);
        live.hub.clients.insert(20, ClientHandle { tx: left_tx });
        live.hub.clients.insert(10, ClientHandle { tx: right_tx });
        let child_key = live.hub.panes[&child].key();
        live.hub.on_pty_bytes(
            child_key,
            &mut b"RIGHT\x07\x1b]2;right-title\x07\x1b]52;c;Y29weQ==\x07".to_vec(),
        );
        live.hub.render_now();

        let right_messages: Vec<_> = right_rx.try_iter().collect();
        assert!(
            right_messages
                .iter()
                .any(|message| matches!(message, ServerToClient::Bell))
        );
        assert!(right_messages.iter().any(
            |message| matches!(message, ServerToClient::Title(title) if title == "right-title")
        ));
        assert!(right_messages.iter().any(
            |message| matches!(message, ServerToClient::ClipboardCopy(data) if data == b"Y29weQ==")
        ));
        assert!(right_messages.iter().any(|message| {
            matches!(message, ServerToClient::Grid(update) if match &update.payload {
                GridPayload::Full(rows) => rows.iter().any(|row| {
                    row.cells.iter().map(|cell| cell.ch).collect::<String>().contains("RIGHT")
                }),
                GridPayload::Rows(_) => false,
            })
        }));
        assert!(
            left_rx.try_iter().all(|message| !matches!(
                message,
                ServerToClient::Bell | ServerToClient::Title(_) | ServerToClient::ClipboardCopy(_)
            )),
            "another pane's viewer must not receive pane-local output"
        );
    }

    #[test]
    fn stale_generation_bytes_exits_and_panics_are_ignored_per_pane() {
        let mut live = LiveHub::new(80, 24, &[1]);
        let current = live.hub.panes.values().next().unwrap().key();
        let stale = PaneKey {
            id: current.id,
            generation: PaneGeneration(current.generation.0.saturating_sub(1)),
        };

        live.hub.handle(HubInstruction::PtyBytes {
            pane: stale,
            bytes: b"stale".to_vec(),
        });
        live.hub.handle(HubInstruction::PtyExited {
            pane: stale,
            code: Some(9),
        });
        live.hub.handle(HubInstruction::PtyThreadPanicked {
            pane: stale,
            thread_name: format!("pty-reader-p{}-g0", current.id.0),
            message: "stale panic".to_string(),
        });

        assert_eq!(live.hub.panes.len(), 1);
        assert!(!live.hub.should_exit);
        assert_eq!(live.hub.panes[&current.id].first_cell_for_test(), ' ');

        live.hub.panes[&current.id].reserve_backlog_for_test(1);
        live.hub.handle(HubInstruction::PtyBytes {
            pane: current,
            bytes: b"x".to_vec(),
        });
        assert_eq!(live.hub.panes[&current.id].first_cell_for_test(), 'x');
    }
}
