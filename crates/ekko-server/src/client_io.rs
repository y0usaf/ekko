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
    ClientToServer, GridPayload, GridRow, GridUpdate, ServerToClient, read_msg, write_msg,
};
use interprocess::local_socket::Stream as LocalSocketStream;
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::traits::SendHalf as _;

use crate::hub::HubInstruction;

pub type ClientId = u64;

/// Depth of the per-client outgoing message queue. `Grid` frames are
/// coalesced by the writer thread (see [`coalesce_grid_frames`]) so this
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
        for msg in coalesce_grid_frames(batch) {
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

/// Collapse consecutive `Grid` frames in a batch by merging them — sparse
/// `Rows` diffs stack, so simply keeping the newest frame would lose the rows
/// carried only by earlier frames. Relative order with non-`Grid` messages
/// (e.g. `Exit`) is preserved.
fn coalesce_grid_frames(batch: Vec<ServerToClient>) -> Vec<ServerToClient> {
    let mut out = Vec::with_capacity(batch.len());
    let mut iter = batch.into_iter().peekable();
    while let Some(msg) = iter.next() {
        if let ServerToClient::Grid(update) = msg {
            let mut merged = update;
            while matches!(iter.peek(), Some(ServerToClient::Grid(_))) {
                let Some(ServerToClient::Grid(next)) = iter.next() else {
                    unreachable!("peeked Grid");
                };
                merged = merge_grid_updates(merged, next);
            }
            out.push(ServerToClient::Grid(merged));
        } else {
            out.push(msg);
        }
    }
    out
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
        payload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_proto::{ExitReason, GridPayload, GridUpdate, TermModes};

    fn grid(epoch: u64) -> ServerToClient {
        ServerToClient::Grid(GridUpdate {
            epoch,
            cols: 1,
            rows: 1,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            payload: GridPayload::Full(vec![]),
        })
    }

    #[test]
    fn coalesces_consecutive_grid_frames_keeping_the_last() {
        let batch = vec![grid(1), grid(2), grid(3)];
        let out = coalesce_grid_frames(batch);
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
        ServerToClient::Grid(GridUpdate {
            epoch,
            cols: 1,
            rows: 3,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            payload: GridPayload::Rows(patches),
        })
    }

    #[test]
    fn merges_stacked_row_diffs_without_losing_rows() {
        let batch = vec![
            rows_update(1, vec![(0, row('a'))]),
            rows_update(2, vec![(1, row('b'))]),
        ];
        let out = coalesce_grid_frames(batch);
        assert_eq!(
            out,
            vec![rows_update(2, vec![(0, row('a')), (1, row('b'))])]
        );
    }

    #[test]
    fn applies_row_diffs_onto_a_preceding_full_frame() {
        let full = ServerToClient::Grid(GridUpdate {
            epoch: 1,
            cols: 1,
            rows: 2,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            payload: GridPayload::Full(vec![row('a'), row('b')]),
        });
        let batch = vec![full, rows_update(2, vec![(1, row('z'))])];
        let out = coalesce_grid_frames(batch);
        let [ServerToClient::Grid(merged)] = out.as_slice() else {
            panic!("expected one grid frame");
        };
        let GridPayload::Full(rows) = &merged.payload else {
            panic!("expected merged full payload");
        };
        assert_eq!(merged.epoch, 2);
        assert_eq!(rows[0].cells[0].ch, 'a');
        assert_eq!(rows[1].cells[0].ch, 'z');
    }

    #[test]
    fn newer_full_frame_supersedes_older_diffs() {
        let full = ServerToClient::Grid(GridUpdate {
            epoch: 2,
            cols: 1,
            rows: 1,
            cursor: None,
            modes: TermModes::default(),
            scrollback: 0,
            payload: GridPayload::Full(vec![row('n')]),
        });
        let batch = vec![rows_update(1, vec![(0, row('a'))]), full.clone()];
        let out = coalesce_grid_frames(batch);
        assert_eq!(out, vec![full]);
    }

    #[test]
    fn preserves_order_around_non_grid_messages() {
        let batch = vec![
            grid(1),
            grid(2),
            ServerToClient::Pong,
            grid(3),
            ServerToClient::Exit(ExitReason::Detached),
        ];
        let out = coalesce_grid_frames(batch);
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
