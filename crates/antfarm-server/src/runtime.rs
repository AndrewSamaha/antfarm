use antfarm_core::TICK_MILLIS;
use serde_json::json;
use std::time::{Duration, Instant};
use tokio::time;

use crate::{
    debug_npc::send_npc_debug_events,
    logging::emit_log,
    server_state::{PersistMessage, ServerState},
    sync::broadcast_patch,
};

pub(crate) const HEARTBEAT_INTERVAL_SECONDS: u64 = 30;

pub(crate) fn spawn_background_tasks(state: &ServerState) {
    {
        let tick_state = state.clone();
        tokio::spawn(async move {
            let mut ticker = time::interval(Duration::from_millis(TICK_MILLIS));
            let mut last_snapshot_at = Instant::now();
            loop {
                ticker.tick().await;
                let (maybe_patch, maybe_snapshot, npc_debug_events) = {
                    let mut game = tick_state.game.lock().await;
                    if !game.simulation_paused {
                        game.tick();
                    }
                    let patch = game.take_patch();
                    let npc_debug_events = game.take_npc_debug_events();
                    let interval = Duration::from_secs_f64(game.snapshot_interval_seconds());
                    let snapshot = if last_snapshot_at.elapsed() >= interval {
                        last_snapshot_at = Instant::now();
                        Some(game.snapshot())
                    } else {
                        None
                    };
                    (patch, snapshot, npc_debug_events)
                };

                if let Some(snapshot) = maybe_snapshot {
                    let _ = tick_state.persistence_tx.send(PersistMessage::Save(snapshot));
                }
                if !npc_debug_events.is_empty() {
                    let session = tick_state.npc_debug.lock().await.clone();
                    if let Some(session) = session {
                        send_npc_debug_events(&session, npc_debug_events);
                    }
                }
                if let Some(patch) = maybe_patch {
                    if let Err(error) = broadcast_patch(&tick_state, &patch, None).await {
                        emit_log("patch_broadcast_error", json!({ "error": error.to_string() }));
                    }
                }
            }
        });
    }

    {
        let heartbeat_state = state.clone();
        tokio::spawn(async move {
            let mut heartbeat = time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECONDS));
            loop {
                heartbeat.tick().await;
                let (tick, players, npcs) = {
                    let game = heartbeat_state.game.lock().await;
                    (game.tick, game.players.len(), game.npcs.len())
                };
                emit_log(
                    "heartbeat",
                    json!({
                        "tick": tick,
                        "connected_players": players,
                        "npc_count": npcs,
                    }),
                );
            }
        });
    }
}
