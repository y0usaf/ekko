//! Server-owned state for one terminal pane.
//!
//! P1 deliberately keeps exactly one entry in the hub's pane map. This
//! object is the ownership boundary P2 can multiply: parser/callback state,
//! VT filtering, PTY resources, flow control, title, render scheduling, and
//! diff state all live here rather than as flattened hub fields.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Context;
use crossbeam_channel::Sender;
use ekko_proto::{CursorState, GridRow, TermModes};
use ekko_pty::{Pid, PtyHandle};

use crate::grid;
use crate::hub::HubInstruction;
use crate::pty_io::{self, PtyBacklog};
use crate::pty_writer::{self, PtyWriterInstruction};
use crate::vt_compat::HvpToCup;

const RENDER_TICK: Duration = Duration::from_millis(16);
const RENDER_SETTLE: Duration = Duration::from_millis(1);

/// Stable server-internal identity. It is intentionally absent from the P1
/// wire contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PaneId(pub(crate) u64);

/// Incarnation of a stable pane ID. Events from an older incarnation are
/// stale even when the ID has been reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PaneGeneration(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PaneKey {
    pub(crate) id: PaneId,
    pub(crate) generation: PaneGeneration,
}

pub(crate) fn pty_thread_name(role: &str, pane: PaneKey) -> String {
    format!("pty-{role}-p{}-g{}", pane.id.0, pane.generation.0)
}

/// Recover the identity stamped into reader/writer thread names so the global
/// panic hook can emit an identity-tagged hub instruction.
pub(crate) fn pane_key_from_pty_thread_name(name: &str) -> Option<PaneKey> {
    let suffix = name
        .strip_prefix("pty-reader-p")
        .or_else(|| name.strip_prefix("pty-writer-p"))?;
    let (id, generation) = suffix.split_once("-g")?;
    Some(PaneKey {
        id: PaneId(id.parse().ok()?),
        generation: PaneGeneration(generation.parse().ok()?),
    })
}

struct PtyIo {
    master_fd: Option<OwnedFd>,
    writer_tx: Sender<PtyWriterInstruction>,
    backlog: PtyBacklog,
    retired: Arc<AtomicBool>,
    reader_thread: Option<JoinHandle<()>>,
    writer_thread: Option<JoinHandle<()>>,
}

impl PtyIo {
    fn start(
        master_fd: OwnedFd,
        pane: PaneKey,
        hub_tx: Sender<HubInstruction>,
    ) -> anyhow::Result<Self> {
        // Both workers must periodically observe retirement. Non-blocking I/O
        // makes their join deterministic even if the child stops reading or
        // writing before the pane is removed.
        ekko_pty::set_nonblocking(master_fd.as_raw_fd(), true)
            .context("setting pane PTY non-blocking")?;
        let fd = master_fd.as_raw_fd();
        let backlog = PtyBacklog::default();
        let retired = Arc::new(AtomicBool::new(false));

        let reader_backlog = backlog.clone();
        let reader_retired = Arc::clone(&retired);
        let reader_tx = hub_tx;
        let reader_thread = std::thread::Builder::new()
            .name(pty_thread_name("reader", pane))
            .spawn(move || pty_io::run(fd, &reader_tx, &reader_backlog, &reader_retired, pane))
            .context("spawning pane PTY reader")?;

        let (writer_tx, writer_rx) = crossbeam_channel::unbounded();
        let writer_retired = Arc::clone(&retired);
        let writer_thread = match std::thread::Builder::new()
            .name(pty_thread_name("writer", pane))
            .spawn(move || pty_writer::run(&writer_rx, fd, &writer_retired))
        {
            Ok(thread) => thread,
            Err(error) => {
                retired.store(true, Ordering::Release);
                let _ = reader_thread.join();
                return Err(error).context("spawning pane PTY writer");
            }
        };

        Ok(Self {
            master_fd: Some(master_fd),
            writer_tx,
            backlog,
            retired,
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
        })
    }

    fn send(&self, instruction: PtyWriterInstruction) {
        let _ = self.writer_tx.send(instruction);
    }

    fn retire(&mut self) {
        self.retired.store(true, Ordering::Release);
        let _ = self.writer_tx.send(PtyWriterInstruction::Shutdown);
        if let Some(thread) = self.writer_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.reader_thread.take() {
            let _ = thread.join();
        }
        // Workers borrow only the raw number and are now joined, so closing
        // the owned descriptor cannot race either worker.
        self.master_fd.take();
    }
}

impl Drop for PtyIo {
    fn drop(&mut self) {
        self.retire();
    }
}

struct PtySession {
    child_pid: Pid,
    io: PtyIo,
}

impl PtySession {
    fn retire(mut self, terminate_child: bool) {
        if terminate_child {
            let _ = ekko_pty::kill(self.child_pid);
        }
        self.io.retire();
    }
}

#[derive(Default)]
struct RenderDiff {
    rows: Vec<GridRow>,
    cursor: Option<CursorState>,
    size: (u16, u16),
    modes: TermModes,
    scrollback: u32,
    force_full: bool,
}

struct RenderState {
    dirty: bool,
    deadline: Option<Instant>,
    last_render: Instant,
    diff: RenderDiff,
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            dirty: false,
            deadline: None,
            last_render: Instant::now() - RENDER_TICK,
            diff: RenderDiff {
                force_full: true,
                ..RenderDiff::default()
            },
        }
    }
}

pub(crate) struct PaneOutput {
    pub(crate) bells: usize,
    pub(crate) title: Option<String>,
    pub(crate) clipboard_copy: Option<Vec<u8>>,
}

pub(crate) struct RenderFrame {
    pub(crate) pane: PaneKey,
    pub(crate) rows: Vec<GridRow>,
    pub(crate) cursor: CursorState,
    pub(crate) size: (u16, u16),
    pub(crate) modes: TermModes,
    pub(crate) scrollback: u32,
    pub(crate) full_for_all: bool,
    pub(crate) patches: Vec<(u16, GridRow)>,
    pub(crate) steady: bool,
}

pub(crate) struct TerminalPane {
    key: PaneKey,
    parser: vt100::Parser<TermEvents>,
    vt_compat: HvpToCup,
    pty: PtySession,
    title: Option<String>,
    render: RenderState,
}

impl TerminalPane {
    pub(crate) fn from_pty_handle(
        key: PaneKey,
        handle: PtyHandle,
        rows: u16,
        cols: u16,
        scrollback: usize,
        host_colors: Option<ekko_proto::TerminalColors>,
        hub_tx: Sender<HubInstruction>,
    ) -> anyhow::Result<Self> {
        let child_pid = handle.child_pid;
        let io = match PtyIo::start(handle.master_fd, key, hub_tx) {
            Ok(io) => io,
            Err(error) => {
                let _ = ekko_pty::kill(child_pid);
                return Err(error);
            }
        };
        Ok(Self {
            key,
            parser: vt100::Parser::new_with_callbacks(
                rows,
                cols,
                scrollback,
                TermEvents {
                    host_colors,
                    ..TermEvents::default()
                },
            ),
            vt_compat: HvpToCup::default(),
            pty: PtySession { child_pid, io },
            title: None,
            render: RenderState::default(),
        })
    }

    pub(crate) fn key(&self) -> PaneKey {
        self.key
    }

    pub(crate) fn set_host_colors(&mut self, colors: ekko_proto::TerminalColors) {
        self.parser.callbacks_mut().host_colors = Some(colors);
    }

    pub(crate) fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (cols, rows)
    }

    pub(crate) fn resize(&mut self, cols: u16, rows: u16) {
        self.parser.screen_mut().set_size(rows, cols);
        self.render.diff.force_full = true;
        self.pty.io.send(PtyWriterInstruction::Resize(cols, rows));
    }

    pub(crate) fn application_cursor(&self) -> bool {
        self.parser.screen().application_cursor()
    }

    pub(crate) fn bracketed_paste(&self) -> bool {
        self.parser.screen().bracketed_paste()
    }

    pub(crate) fn scrollback(&self) -> usize {
        self.parser.screen().scrollback()
    }

    pub(crate) fn alternate_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    pub(crate) fn set_scrollback(&mut self, rows: usize) {
        self.parser.screen_mut().set_scrollback(rows);
    }

    pub(crate) fn write(&self, bytes: Vec<u8>) {
        self.pty.io.send(PtyWriterInstruction::Write(bytes));
    }

    pub(crate) fn process_bytes(&mut self, bytes: &mut [u8]) -> PaneOutput {
        self.vt_compat.rewrite_in_place(bytes);
        self.parser.process(bytes);
        self.pty.io.backlog.release(bytes.len());

        let replies = std::mem::take(&mut self.parser.callbacks_mut().replies);
        if !replies.is_empty() {
            self.write(replies);
        }
        let bells = std::mem::take(&mut self.parser.callbacks_mut().audible);
        let title_changed = self.parser.callbacks_mut().title.take();
        let has_title_change = title_changed.is_some();
        if let Some(title) = title_changed {
            self.title = Some(title);
        }
        PaneOutput {
            bells,
            title: has_title_change.then(|| self.title.clone()).flatten(),
            clipboard_copy: self.parser.callbacks_mut().clipboard_copy.take(),
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.render.dirty = true;
        if self.render.deadline.is_none() {
            let now = Instant::now();
            self.render.deadline =
                Some((self.render.last_render + RENDER_TICK).max(now + RENDER_SETTLE));
        }
    }

    pub(crate) fn force_dirty(&mut self) {
        self.render.dirty = true;
    }

    pub(crate) fn render_deadline(&self) -> Option<Instant> {
        self.render.deadline
    }

    pub(crate) fn prepare_render(&mut self, clients_need_full: bool) -> Option<RenderFrame> {
        self.render.deadline = None;
        if !self.render.dirty {
            return None;
        }

        let cursor_shape = self.parser.callbacks_mut().cursor_shape;
        let focus_reporting = self.parser.callbacks_mut().focus_reporting;
        let screen = self.parser.screen();
        let (screen_rows, screen_cols) = screen.size();
        let mut cursor = grid::cursor_state(screen);
        cursor.shape = cursor_shape;
        let scrollback = screen.scrollback() as u32;
        if scrollback > 0 {
            cursor.visible = false;
        }
        let mut modes = grid::term_modes(screen);
        modes.focus_reporting = focus_reporting;
        let rows = grid::screen_rows(screen);
        let size = (screen_cols, screen_rows);

        let full_for_all = self.render.diff.force_full
            || size != self.render.diff.size
            || self.render.diff.rows.is_empty();
        let patches = if full_for_all {
            Vec::new()
        } else {
            rows.iter()
                .enumerate()
                .filter(|(index, row)| self.render.diff.rows.get(*index) != Some(row))
                .map(|(index, row)| (index as u16, row.clone()))
                .collect()
        };
        let steady = !full_for_all
            && patches.is_empty()
            && self.render.diff.cursor == Some(cursor)
            && self.render.diff.modes == modes
            && self.render.diff.scrollback == scrollback;
        if steady && !clients_need_full {
            self.render.dirty = false;
            return None;
        }

        Some(RenderFrame {
            pane: self.key,
            rows,
            cursor,
            size,
            modes,
            scrollback,
            full_for_all,
            patches,
            steady,
        })
    }

    pub(crate) fn commit_render(&mut self, frame: &RenderFrame) {
        if frame.pane != self.key {
            return;
        }
        self.render.diff.rows.clone_from(&frame.rows);
        self.render.diff.cursor = Some(frame.cursor);
        self.render.diff.size = frame.size;
        self.render.diff.modes = frame.modes;
        self.render.diff.scrollback = frame.scrollback;
        self.render.diff.force_full = false;
        self.render.dirty = false;
        self.render.last_render = Instant::now();
    }

    pub(crate) fn retire(self, terminate_child: bool) {
        self.pty.retire(terminate_child);
    }

    #[cfg(test)]
    pub(crate) fn test_pane(
        key: PaneKey,
        hub_tx: Sender<HubInstruction>,
    ) -> (Self, std::os::unix::net::UnixStream) {
        use std::os::fd::OwnedFd;
        let (master, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let handle = PtyHandle {
            master_fd: OwnedFd::from(master),
            child_pid: Pid::from_raw(std::process::id() as i32),
            terminal_id: 0,
        };
        (
            Self::from_pty_handle(key, handle, 2, 8, 16, None, hub_tx).unwrap(),
            peer,
        )
    }

    #[cfg(test)]
    pub(crate) fn reserve_backlog_for_test(&self, bytes: usize) {
        self.pty.io.backlog.reserve_for_test(bytes);
    }

    #[cfg(test)]
    pub(crate) fn first_cell_for_test(&self) -> char {
        self.parser
            .screen()
            .cell(0, 0)
            .and_then(|cell| cell.contents().chars().next())
            .unwrap_or(' ')
    }
}

/// Parser callbacks and terminal state that vt100 does not model directly.
#[derive(Default)]
struct TermEvents {
    audible: usize,
    replies: Vec<u8>,
    host_colors: Option<ekko_proto::TerminalColors>,
    title: Option<String>,
    clipboard_copy: Option<Vec<u8>>,
    cursor_shape: u8,
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
            (None, 'n') => match first_param {
                Some(5) => self.replies.extend_from_slice(b"\x1b[0n"),
                Some(6) => {
                    let (row, col) = screen.cursor_position();
                    self.replies
                        .extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
                }
                _ => {}
            },
            (Some(b'?'), 'n') if first_param == Some(6) => {
                let (row, col) = screen.cursor_position();
                self.replies
                    .extend_from_slice(format!("\x1b[?{};{}R", row + 1, col + 1).as_bytes());
            }
            (None, 'c') => self.replies.extend_from_slice(b"\x1b[?6c"),
            (Some(b'>'), 'c') => self.replies.extend_from_slice(b"\x1b[>84;0;0c"),
            (Some(b' '), 'q') => self.cursor_shape = first_param.unwrap_or(0).min(6) as u8,
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
        match params {
            [b"10", b"?"] => {
                let (r, g, b) = self.host_foreground();
                self.replies.extend_from_slice(
                    format!("\x1b]10;{}\x1b\\", osc_color_reply_body(r, g, b)).as_bytes(),
                );
            }
            [b"11", b"?"] => {
                let (r, g, b) = self.host_background();
                self.replies.extend_from_slice(
                    format!("\x1b]11;{}\x1b\\", osc_color_reply_body(r, g, b)).as_bytes(),
                );
            }
            [b"4", rest @ ..] => {
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
                    self.replies.extend_from_slice(
                        format!("\x1b]4;{};{}\x1b\\", idx, osc_color_reply_body(r, g, b))
                            .as_bytes(),
                    );
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

fn osc_color_reply_body(r: u8, g: u8, b: u8) -> String {
    let expand = |c: u8| u16::from(c) * 0x0101;
    format!("rgb:{:04x}/{:04x}/{:04x}", expand(r), expand(g), expand(b))
}

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

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn pty_thread_names_round_trip_pane_identity() {
        let key = PaneKey {
            id: PaneId(42),
            generation: PaneGeneration(7),
        };
        for role in ["reader", "writer"] {
            let name = pty_thread_name(role, key);
            assert_eq!(pane_key_from_pty_thread_name(&name), Some(key));
        }
        assert_eq!(pane_key_from_pty_thread_name("client-writer-42"), None);
    }

    #[test]
    fn retirement_joins_workers_and_closes_the_master_fd() {
        let key = PaneKey {
            id: PaneId(1),
            generation: PaneGeneration(1),
        };
        let (master, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let (hub_tx, _hub_rx) = crossbeam_channel::unbounded();
        let mut io = PtyIo::start(OwnedFd::from(master), key, hub_tx).unwrap();
        let writer = io.writer_tx.clone();

        io.retire();

        assert!(writer.send(PtyWriterInstruction::Write(vec![1])).is_err());
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut byte = [0];
        assert_eq!(peer.read(&mut byte).unwrap(), 0, "master fd must be closed");
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
        let replies =
            String::from_utf8(std::mem::take(&mut parser.callbacks_mut().replies)).unwrap();
        assert_eq!(
            replies,
            "\x1b]10;rgb:cdcd/d6d6/f4f4\x1b\\\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\\x1b]4;1;rgb:f3f3/8b8b/a8a8\x1b\\"
        );
    }

    #[test]
    fn term_events_answer_osc_color_queries_with_vga_fallback() {
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 0, TermEvents::default());
        parser.process(b"\x1b]11;?\x1b\\\x1b]4;9;?\x07");
        let replies =
            String::from_utf8(std::mem::take(&mut parser.callbacks_mut().replies)).unwrap();
        assert_eq!(
            replies,
            "\x1b]11;rgb:0000/0000/0000\x1b\\\x1b]4;9;rgb:ffff/0000/0000\x1b\\"
        );
    }

    #[test]
    fn term_events_ignore_osc_color_sets_and_out_of_range_queries() {
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 0, TermEvents::default());
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
