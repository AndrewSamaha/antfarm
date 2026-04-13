use antfarm_core::{
    FullSyncChunk, FullSyncComplete, FullSyncStart, PatchFrame, ServerMessage, Snapshot,
};
use anyhow::Result;

use crate::server_state::{ClientTx, ServerState};

pub(crate) const FULL_SYNC_ROWS_PER_CHUNK: i32 = 16;

pub(crate) async fn broadcast_patch(
    state: &ServerState,
    patch: &PatchFrame,
    exclude_player_id: Option<u8>,
) -> Result<()> {
    let clients = state.clients.lock().await;
    for (player_id, tx) in clients.iter() {
        if Some(*player_id) == exclude_player_id {
            continue;
        }
        let _ = tx.send(ServerMessage::Patch(patch.clone()));
    }
    Ok(())
}

pub(crate) async fn broadcast_full_sync(state: &ServerState, snapshot: &Snapshot) -> Result<()> {
    let clients = state.clients.lock().await;
    for (player_id, tx) in clients.iter() {
        send_full_sync(tx, *player_id, snapshot)?;
    }
    Ok(())
}

pub(crate) fn send_full_sync(tx: &ClientTx, player_id: u8, snapshot: &Snapshot) -> Result<()> {
    tx.send(ServerMessage::FullSyncStart(FullSyncStart {
        player_id,
        tick: snapshot.tick,
        world_width: snapshot.world.width(),
        world_height: snapshot.world.height(),
        total_rows: snapshot.world.height(),
    }))?;

    let mut row = 0;
    while row < snapshot.world.height() {
        let end = (row + FULL_SYNC_ROWS_PER_CHUNK).min(snapshot.world.height());
        let rows = (row..end).map(|y| snapshot.world.row_tiles(y)).collect();
        tx.send(ServerMessage::FullSyncChunk(FullSyncChunk {
            start_row: row,
            rows,
        }))?;
        row = end;
    }

    tx.send(ServerMessage::FullSyncComplete(FullSyncComplete {
        players: snapshot.players.clone(),
        npcs: snapshot.npcs.clone(),
        placed_art: snapshot.placed_art.clone(),
        event_log: snapshot.event_log.clone(),
        config: snapshot.config.clone(),
    }))?;
    Ok(())
}
