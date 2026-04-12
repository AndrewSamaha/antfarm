mod client_session;
mod logging;
mod persistence;
mod server_state;
mod sync;

use antfarm_core::TICK_MILLIS;
use anyhow::Result;
use serde_json::json;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::TcpListener, sync::Mutex, time};

use crate::{
    client_session::{SNAPSHOT_DB_PATH, handle_client},
    logging::{emit_log, world_log_fields},
    persistence::{load_startup_game, spawn_persistence_worker},
    server_state::{PersistMessage, ServerState},
    sync::broadcast_patch,
};

const SERVER_ADDR: &str = "127.0.0.1:7000";
const HEARTBEAT_INTERVAL_SECONDS: u64 = 30;

#[tokio::main]
async fn main() -> Result<()> {
    let snapshot_path = PathBuf::from(SNAPSHOT_DB_PATH);
    emit_log(
        "starting_server",
        json!({
            "addr": SERVER_ADDR,
            "snapshot_db": snapshot_path.display().to_string(),
        }),
    );
    let persistence_tx = spawn_persistence_worker(snapshot_path.clone())?;
    let (initial_game, restored) = load_startup_game(&snapshot_path)?;

    emit_log(
        "server_start",
        json!({
            "addr": SERVER_ADDR,
            "snapshot_db": snapshot_path.display().to_string(),
            "restored_snapshot": restored,
            "world": world_log_fields(&initial_game),
        }),
    );

    let listener = TcpListener::bind(SERVER_ADDR).await?;
    let state = ServerState {
        game: Arc::new(Mutex::new(initial_game)),
        clients: Arc::new(Mutex::new(HashMap::new())),
        session_tokens: Arc::new(Mutex::new(HashMap::new())),
        persistence_tx,
    };

    {
        let tick_state = state.clone();
        tokio::spawn(async move {
            let mut ticker = time::interval(Duration::from_millis(TICK_MILLIS));
            let mut last_snapshot_at = Instant::now();
            loop {
                ticker.tick().await;
                let (maybe_patch, maybe_snapshot) = {
                    let mut game = tick_state.game.lock().await;
                    game.tick();
                    let patch = game.take_patch();
                    let interval = Duration::from_secs_f64(game.snapshot_interval_seconds());
                    let snapshot = if last_snapshot_at.elapsed() >= interval {
                        last_snapshot_at = Instant::now();
                        Some(game.snapshot())
                    } else {
                        None
                    };
                    (patch, snapshot)
                };

                if let Some(snapshot) = maybe_snapshot {
                    let _ = tick_state.persistence_tx.send(PersistMessage::Save(snapshot));
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

    emit_log("server_listening", json!({ "addr": SERVER_ADDR }));

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_client(stream, state).await {
                emit_log("client_session_error", json!({ "error": error.to_string() }));
            }
        });
    }
}
