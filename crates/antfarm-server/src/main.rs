mod logging;
mod persistence;
mod server_state;
mod sync;

use antfarm_core::{ClientMessage, TICK_MILLIS};
use anyhow::Result;
use serde_json::json;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time,
};

use crate::{
    logging::{emit_log, world_log_fields},
    persistence::{load_player_profile, load_startup_game, spawn_persistence_worker},
    server_state::{PersistMessage, ServerState},
    sync::{broadcast_full_sync, broadcast_patch, send_full_sync},
};

const SERVER_ADDR: &str = "127.0.0.1:7000";
const SNAPSHOT_DB_PATH: &str = "data/antfarm.sqlite3";
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

async fn handle_client(stream: TcpStream, state: ServerState) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<antfarm_core::ServerMessage>();

    let mut player_id = None;

    let writer_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let payload = serde_json::to_string(&message)?;
            writer.write_all(payload.as_bytes()).await?;
            writer.write_all(b"\n").await?;
        }
        Ok::<(), anyhow::Error>(())
    });

    while let Some(line) = lines.next_line().await? {
        let message: ClientMessage = serde_json::from_str(&line)?;
        match message {
            ClientMessage::Join { name, token } => {
                if player_id.is_some() {
                    continue;
                }

                let existing_player_id = {
                    let sessions = state.session_tokens.lock().await;
                    sessions
                        .iter()
                        .find_map(|(id, session_token)| (session_token == &token).then_some(*id))
                };
                if existing_player_id.is_some() {
                    tx.send(antfarm_core::ServerMessage::Error {
                        message: "client token already connected".to_string(),
                    })?;
                    continue;
                }

                let restored_player = load_player_profile(Path::new(SNAPSHOT_DB_PATH), &token)?;
                let restored = restored_player.is_some();

                let (id, snapshot, join_patch) = {
                    let mut game = state.game.lock().await;
                    let (id, snapshot) = game
                        .add_player(name, restored_player)
                        .map_err(anyhow::Error::msg)?;
                    let patch = game.take_patch();
                    (id, snapshot, patch)
                };

                state.clients.lock().await.insert(id, tx.clone());
                state
                    .session_tokens
                    .lock()
                    .await
                    .insert(id, token.clone());
                player_id = Some(id);
                emit_log(
                    "player_join",
                    json!({
                        "player_id": id,
                        "name": snapshot.players.iter().find(|player| player.id == id).map(|player| player.name.clone()),
                        "connected_players": snapshot.players.len(),
                        "restored": restored,
                    }),
                );

                if let Some(player) = snapshot.players.iter().find(|player| player.id == id) {
                    let _ = state.persistence_tx.send(PersistMessage::UpsertPlayerProfile {
                        token: token.clone(),
                        player: player.clone(),
                    });
                }

                send_full_sync(&tx, id, &snapshot)?;
                if let Some(patch) = join_patch {
                    broadcast_patch(&state, &patch, Some(id)).await?;
                }
            }
            ClientMessage::Action(action) => {
                let Some(id) = player_id else {
                    tx.send(antfarm_core::ServerMessage::Error {
                        message: "Join before sending actions".to_string(),
                    })?;
                    continue;
                };

                let maybe_patch = {
                    let mut game = state.game.lock().await;
                    game.apply_action(id, action);
                    game.take_patch()
                };
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::ConfigSet { path, value } => {
                let Some(_id) = player_id else {
                    tx.send(antfarm_core::ServerMessage::Error {
                        message: "Join before changing config".to_string(),
                    })?;
                    continue;
                };

                let logged_value = value.clone();
                let (maybe_patch, snapshot) = {
                    let mut game = state.game.lock().await;
                    game.set_config_value(&path, value)
                        .map_err(anyhow::Error::msg)?;
                    let patch = game.take_patch();
                    let snapshot = game.snapshot();
                    (patch, snapshot)
                };

                emit_log("sc_set", json!({ "path": path, "value": logged_value }));
                let _ = state.persistence_tx.send(PersistMessage::Save(snapshot));
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::WorldReset { seed } => {
                let snapshot = {
                    let mut game = state.game.lock().await;
                    game.world_reset(seed);
                    let snapshot = game.snapshot();
                    let _ = game.take_patch();
                    emit_log(
                        "world_reset",
                        json!({
                            "seed_override": seed,
                            "world": world_log_fields(&game),
                        }),
                    );
                    snapshot
                };
                let _ = state.persistence_tx.send(PersistMessage::Save(snapshot.clone()));
                let _ = state.persistence_tx.send(PersistMessage::ClearPlayerProfiles);
                broadcast_full_sync(&state, &snapshot).await?;
            }
        }
    }

    if let Some(id) = player_id {
        state.clients.lock().await.remove(&id);
        let token = state.session_tokens.lock().await.remove(&id);
        let (maybe_patch, departed_player) = {
            let mut game = state.game.lock().await;
            let departed_player = game.players.get(&id).cloned();
            let player_name = departed_player.as_ref().map(|player| player.name.clone());
            game.remove_player(id);
            emit_log(
                "player_leave",
                json!({
                    "player_id": id,
                    "name": player_name,
                    "connected_players": game.players.len(),
                }),
            );
            (game.take_patch(), departed_player)
        };
        if let (Some(token), Some(player)) = (token, departed_player) {
            let _ = state
                .persistence_tx
                .send(PersistMessage::UpsertPlayerProfile { token, player });
        }
        if let Some(patch) = maybe_patch {
            broadcast_patch(&state, &patch, None).await?;
        }
    }

    writer_task.abort();
    Ok(())
}
