//! Workspace broadcast: the daemon's render pass, extracted from `hub.rs`.
//!
//! One free function, called from the hub's render deadline; `Hub` stays
//! the single owner of all state and this module only reads/mutates it
//! through `pub(crate)` accessors. Splitting it out keeps the hub file the
//! *state machine* (instructions in, transitions) and this file the
//! *projection* (state → per-client wire frames).

use std::collections::HashMap;

use ekko_proto::{
    GridPayload, GridUpdate, PaneGrid, PaneMeta, PaneRect, ServerToClient, WorkspaceUpdate,
};

use super::hub::Hub;
use super::terminal_pane::PaneId;
use super::topology::{PaneTopology, Rect};

/// Broadcast one workspace frame per attached client. Every frame carries
/// the complete canonical pane projection (metadata is cheap and makes
/// removal/topology recovery idempotent); grid payloads stay incremental
/// per pane. A client in `needs_full` (fresh attach or a dropped frame)
/// receives `Full` grids for every live pane.
pub(crate) fn render_now(hub: &mut Hub) {
    let Some(canvas) = hub.canvas() else {
        return;
    };
    let Ok(geometry) = hub.resolved_geometry(canvas) else {
        return;
    };
    let geometry: HashMap<PaneId, Rect> = geometry.into_iter().collect();
    let pane_ids = hub.topology().map(PaneTopology::leaves).unwrap_or_default();
    let resync = hub
        .needs_full
        .iter()
        .any(|client| hub.attached.contains_key(client));

    let mut frames = Vec::new();
    for pane_id in pane_ids {
        if let Some(frame) = hub
            .panes
            .get_mut(&pane_id)
            .and_then(|pane| pane.prepare_render(resync))
        {
            frames.push(frame);
        }
    }
    if frames.is_empty() && !hub.workspace_dirty {
        return;
    }
    hub.workspace_dirty = false;

    hub.epoch += 1;
    let epoch = hub.epoch;
    let metadata: Vec<PaneMeta> = hub
        .panes
        .iter()
        .filter_map(|(id, pane)| {
            let rect = geometry.get(id)?;
            Some(PaneMeta {
                id: id.0,
                rect: PaneRect {
                    x: rect.x,
                    y: rect.y,
                    cols: rect.cols,
                    rows: rect.rows,
                },
                title: pane.title().map(str::to_owned),
            })
        })
        .collect();

    let mut resync_failed = false;
    for (&id, client) in &hub.clients {
        let Some(&focused) = hub
            .focus
            .get(&id)
            .filter(|_| hub.attached.contains_key(&id))
        else {
            continue;
        };
        let full = hub.needs_full.contains(&id);
        let mut grids = Vec::new();
        for frame in &frames {
            let full_frame = full || frame.full_for_all;
            if frame.steady && !full_frame {
                continue;
            }
            let payload = if full_frame {
                GridPayload::Full(frame.rows.clone())
            } else {
                GridPayload::Rows(frame.patches.clone())
            };
            grids.push(PaneGrid {
                pane: frame.pane.id.0,
                update: GridUpdate {
                    epoch,
                    cols: frame.size.0,
                    rows: frame.size.1,
                    cursor: Some(frame.cursor),
                    modes: frame.modes,
                    scrollback: frame.scrollback,
                    history: frame.history,
                    payload,
                },
            });
        }
        let update = WorkspaceUpdate {
            epoch,
            panes: metadata.clone(),
            focused: focused.0,
            grids,
            border_style: hub.config.ui.pane_borders,
        };
        if client
            .tx
            .try_send(ServerToClient::Workspace(update))
            .is_err()
        {
            log::debug!("hub: client {id}'s queue dropped a workspace frame; full resync queued");
            resync_failed |= hub.needs_full.insert(id);
        } else {
            hub.needs_full.remove(&id);
        }
    }

    for frame in &frames {
        if let Some(pane) = hub.panes.get_mut(&frame.pane.id) {
            pane.commit_render(frame);
        }
    }
    // Schedule the resync retry promptly, but only on the transition into
    // `needs_full`: a persistently slow client otherwise waits for natural
    // pane activity (or eviction by its writer thread's write timeout).
    if resync_failed {
        for pane in hub.panes.values_mut() {
            pane.mark_dirty();
        }
    }
}
