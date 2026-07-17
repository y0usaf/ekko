//! Per-client I/O: a reader ("router") thread that decodes `ClientToServer`
//! frames and forwards them to the hub, and a writer thread that encodes
//! `ServerToClient` frames from a bounded queue.
//!
//! Keeping socket writes off the hub thread means a single slow/stuck client
//! can never block the render loop or other clients; the hub only ever
//! pushes into a bounded channel and moves on.

use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use ekko_proto::{
    ClientToServer, GridPayload, GridRow, GridUpdate, PaneGrid, ServerToClient, WorkspaceUpdate,
    read_msg, write_msg,
};
use interprocess::local_socket::Stream as LocalSocketStream;
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::traits::SendHalf as _;

use crate::hub::HubInstruction;

pub type ClientId = u64;
/// Depth of the per-client outgoing message queue. `Workspace` frames are
/// coalesced by the writer thread (see [`coalesce_workspace_frames`]) so this
/// only needs to absorb a burst, not the full backlog of a slow client.
const CLIENT_QUEUE_DEPTH: usize = 128;

/// Bound on how long a single frame write may block before we give up on the
/// client and let the hub evict it.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Handle the hub keeps for a connected client: just the sender half of its
/// outgoing message queue. The reader/writer threads run independently and
/// report back to the hub via `HubInstruction`.
pub struct ClientHandle {
    pub tx: Sender<ServerToClient>,
}

/// Spawn the reader and writer threads for a freshly accepted connection.
pub fn spawn(
    id: ClientId,
    stream: LocalSocketStream,
    hub_tx: Sender<HubInstruction>,
) -> ClientHandle {
    let (recv_half, send_half) = stream.split();
    let _ = send_half.set_timeout(Some(WRITE_TIMEOUT));

    let (tx, rx) = crossbeam_channel::bounded::<ServerToClient>(CLIENT_QUEUE_DEPTH);

    let writer_hub_tx = hub_tx.clone();
    if let Err(e) = thread::Builder::new()
        .name(format!("client-writer-{id}"))
        .spawn(move || writer_loop(id, send_half, rx, writer_hub_tx))
    {
        log::error!("client-io: failed to spawn writer thread for client {id}: {e}");
    }

    if let Err(e) = thread::Builder::new()
        .name(format!("client-router-{id}"))
        .spawn(move || router_loop(id, recv_half, hub_tx))
    {
        log::error!("client-io: failed to spawn router thread for client {id}: {e}");
    }

    ClientHandle { tx }
}

fn writer_loop(
    id: ClientId,
    mut send_half: impl std::io::Write,
    rx: Receiver<ServerToClient>,
    hub_tx: Sender<HubInstruction>,
) {
    loop {
        let Ok(first) = rx.recv() else { return };
        let mut batch = vec![first];
        while let Ok(msg) = rx.try_recv() {
            batch.push(msg);
        }
        for msg in coalesce_workspace_frames(batch) {
            if let Err(e) = write_msg(&mut send_half, &msg) {
                log::debug!("client-writer[{id}]: write failed, evicting: {e}");
                let _ = hub_tx.send(HubInstruction::ClientWriteFailed(id));
                return;
            }
        }
    }
}

fn router_loop(id: ClientId, mut recv_half: impl std::io::Read, hub_tx: Sender<HubInstruction>) {
    loop {
        match read_msg::<_, ClientToServer>(&mut recv_half) {
            Ok(Some(msg)) => {
                if hub_tx.send(HubInstruction::ClientMsg(id, msg)).is_err() {
                    break;
                }
            }
            Ok(None) => break, // clean EOF
            Err(e) => {
                log::debug!("client-router[{id}]: read error: {e}");
                break;
            }
        }
    }
    let _ = hub_tx.send(HubInstruction::ClientDisconnected(id));
}

/// Collapse consecutive `Workspace` frames in a batch by merging them.
/// Metadata and focus are complete projections, so the newest wins; per-pane
/// grid payloads merge like standalone grid frames so a pane's sparse `Rows`
/// patches are never lost to a later frame that didn't touch that pane.
/// Relative order with non-`Workspace` messages (e.g. `Exit`) is preserved.
fn coalesce_workspace_frames(batch: Vec<ServerToClient>) -> Vec<ServerToClient> {
    let mut out = Vec::with_capacity(batch.len());
    let mut iter = batch.into_iter().peekable();
    while let Some(msg) = iter.next() {
        if let ServerToClient::Workspace(update) = msg {
            let mut merged = update;
            while matches!(iter.peek(), Some(ServerToClient::Workspace(_))) {
                let Some(ServerToClient::Workspace(next)) = iter.next() else {
                    unreachable!("peeked Workspace");
                };
                merged = merge_workspace_updates(merged, next);
            }
            out.push(ServerToClient::Workspace(merged));
        } else {
            out.push(msg);
        }
    }
    out
}

/// Merge two consecutive workspace updates into one equivalent update. The
/// newer frame's metadata and focus are authoritative (complete projections);
/// grids merge per pane: a pane untouched by the newer frame keeps its
/// earlier payload, a pane present in both stacks via [`merge_grid_updates`],
/// and a pane the newer metadata removed drops its moot patches.
fn merge_workspace_updates(older: WorkspaceUpdate, newer: WorkspaceUpdate) -> WorkspaceUpdate {
    let mut grids: Vec<PaneGrid> = older
        .grids
        .into_iter()
        .filter(|grid| newer.panes.iter().any(|meta| meta.id == grid.pane))
        .collect();
    for next in newer.grids {
        if let Some(existing) = grids.iter_mut().find(|grid| grid.pane == next.pane) {
            let previous = std::mem::replace(
                &mut existing.update,
                GridUpdate {
                    epoch: 0,
                    cols: 0,
                    rows: 0,
                    cursor: None,
                    modes: Default::default(),
                    scrollback: 0,
                    history: 0,
                    payload: GridPayload::Rows(Vec::new()),
                },
            );
            existing.update = merge_grid_updates(previous, next.update);
        } else {
            grids.push(next);
        }
    }
    WorkspaceUpdate {
        epoch: newer.epoch,
        panes: newer.panes,
        focused: newer.focused,
        grids,
        border_style: newer.border_style,
    }
}

/// Merge two consecutive grid updates into one equivalent update. The hub
/// guarantees a `Full` frame after any grid dimension change, so two `Rows`
/// frames in a row always share dimensions and simply stack (the client
/// applies patches in order, later entries winning).
fn merge_grid_updates(older: GridUpdate, newer: GridUpdate) -> GridUpdate {
    let payload = match (older.payload, newer.payload) {
        // A newer full snapshot supersedes everything before it.
        (_, GridPayload::Full(rows)) => GridPayload::Full(rows),
        (GridPayload::Full(mut rows), GridPayload::Rows(patches)) => {
            for (index, row) in patches {
                let index = index as usize;
                if index >= rows.len() {
                    rows.resize_with(index + 1, || GridRow { cells: Vec::new() });
                }
                rows[index] = row;
            }
            GridPayload::Full(rows)
        }
        (GridPayload::Rows(mut patches), GridPayload::Rows(newer_patches)) => {
            patches.extend(newer_patches);
            GridPayload::Rows(patches)
        }
    };
    GridUpdate {
        epoch: newer.epoch,
        cols: newer.cols,
        rows: newer.rows,
        cursor: newer.cursor,
        modes: newer.modes,
        scrollback: newer.scrollback,
        history: newer.history,
        payload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_proto::{ExitReason, PaneBorderStyle, PaneMeta, PaneRect, TermModes};

    fn update(epoch: u64, payload: GridPayload) -> GridUpdate {
        GridUpdate {
            epoch,
            cols: 1,
            rows: 1,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            history: 0,
            payload,
        }
    }

    fn workspace(epoch: u64, panes: &[u64], grids: Vec<PaneGrid>) -> ServerToClient {
        ServerToClient::Workspace(WorkspaceUpdate {
            epoch,
            panes: panes
                .iter()
                .map(|&id| PaneMeta {
                    id,
                    rect: PaneRect {
                        x: 0,
                        y: 0,
                        cols: 1,
                        rows: 1,
                    },
                    title: None,
                })
                .collect(),
            focused: panes[0],
            grids,
            border_style: PaneBorderStyle::None,
        })
    }

    fn grid(epoch: u64) -> ServerToClient {
        workspace(
            epoch,
            &[1],
            vec![PaneGrid {
                pane: 1,
                update: update(epoch, GridPayload::Full(vec![])),
            }],
        )
    }

    #[test]
    fn coalesces_consecutive_workspace_frames_keeping_the_last() {
        let batch = vec![grid(1), grid(2), grid(3)];
        let out = coalesce_workspace_frames(batch);
        assert_eq!(out, vec![grid(3)]);
    }

    fn row(ch: char) -> ekko_proto::GridRow {
        ekko_proto::GridRow {
            cells: vec![ekko_proto::GridCell {
                ch,
                extra: Vec::new(),
                fg: ekko_proto::WireColor::Default,
                bg: ekko_proto::WireColor::Default,
                attrs: 0,
            }],
        }
    }

    fn rows_update(epoch: u64, patches: Vec<(u16, ekko_proto::GridRow)>) -> ServerToClient {
        workspace(
            epoch,
            &[1],
            vec![PaneGrid {
                pane: 1,
                update: update(epoch, GridPayload::Rows(patches)),
            }],
        )
    }

    #[test]
    fn merges_stacked_row_diffs_without_losing_rows() {
        let batch = vec![
            rows_update(1, vec![(0, row('a'))]),
            rows_update(2, vec![(1, row('b'))]),
        ];
        let out = coalesce_workspace_frames(batch);
        assert_eq!(
            out,
            vec![rows_update(2, vec![(0, row('a')), (1, row('b'))])]
        );
    }

    #[test]
    fn applies_row_diffs_onto_a_preceding_full_frame() {
        let full = workspace(
            1,
            &[1],
            vec![PaneGrid {
                pane: 1,
                update: GridUpdate {
                    rows: 2,
                    payload: GridPayload::Full(vec![row('a'), row('b')]),
                    ..update(1, GridPayload::Rows(vec![]))
                },
            }],
        );
        let batch = vec![full, rows_update(2, vec![(1, row('z'))])];
        let out = coalesce_workspace_frames(batch);
        let [ServerToClient::Workspace(merged)] = out.as_slice() else {
            panic!("expected one workspace frame");
        };
        let GridPayload::Full(rows) = &merged.grids[0].update.payload else {
            panic!("expected merged full payload");
        };
        assert_eq!(merged.epoch, 2);
        assert_eq!(merged.grids[0].update.epoch, 2);
        assert_eq!(rows[0].cells[0].ch, 'a');
        assert_eq!(rows[1].cells[0].ch, 'z');
    }

    #[test]
    fn newer_full_frame_supersedes_older_diffs() {
        let full = workspace(
            2,
            &[1],
            vec![PaneGrid {
                pane: 1,
                update: update(2, GridPayload::Full(vec![row('n')])),
            }],
        );
        let batch = vec![rows_update(1, vec![(0, row('a'))]), full.clone()];
        let out = coalesce_workspace_frames(batch);
        assert_eq!(out, vec![full]);
    }

    #[test]
    fn merge_stacks_live_panes_and_drops_removed_panes_patches() {
        let older = workspace(
            1,
            &[1, 2, 3],
            vec![
                PaneGrid {
                    pane: 1,
                    update: update(1, GridPayload::Rows(vec![(0, row('a'))])),
                },
                PaneGrid {
                    pane: 2,
                    update: update(1, GridPayload::Rows(vec![(3, row('x'))])),
                },
                PaneGrid {
                    pane: 3,
                    update: update(1, GridPayload::Rows(vec![(5, row('q'))])),
                },
            ],
        );
        let newer = workspace(
            2,
            &[1, 2],
            vec![PaneGrid {
                pane: 2,
                update: update(2, GridPayload::Rows(vec![(4, row('y'))])),
            }],
        );
        let out = coalesce_workspace_frames(vec![older, newer]);
        let [ServerToClient::Workspace(merged)] = out.as_slice() else {
            panic!("expected one workspace frame");
        };
        // Topology: pane 3 is gone, and only the newest metadata survives.
        assert_eq!(merged.panes.len(), 2);
        assert_eq!(merged.focused, 1);
        assert_eq!(merged.epoch, 2);
        // Pane 1 (untouched) keeps its earlier rows; pane 2's rows stack;
        // pane 3's patches vanish with it.
        assert_eq!(merged.grids.len(), 2);
        let GridPayload::Rows(pane1) = &merged.grids[0].update.payload else {
            panic!("expected carried rows for pane 1");
        };
        assert_eq!(pane1[0].1.cells[0].ch, 'a');
        let GridPayload::Rows(pane2) = &merged.grids[1].update.payload else {
            panic!("expected stacked rows for pane 2");
        };
        let chars: Vec<char> = pane2.iter().map(|(_, row)| row.cells[0].ch).collect();
        assert_eq!(chars, vec!['x', 'y']);
    }

    #[test]
    fn preserves_order_around_non_workspace_messages() {
        let batch = vec![
            grid(1),
            grid(2),
            ServerToClient::Pong,
            grid(3),
            ServerToClient::Exit(ExitReason::Detached),
        ];
        let out = coalesce_workspace_frames(batch);
        assert_eq!(
            out,
            vec![
                grid(2),
                ServerToClient::Pong,
                grid(3),
                ServerToClient::Exit(ExitReason::Detached),
            ]
        );
    }
}
