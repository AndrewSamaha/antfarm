use antfarm_core::{ClientMessage, ServerMessage};
use anyhow::Result;
use serde_json::json;
use std::path::Path;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

use crate::{
    debug_npc::{start_npc_debug_session, stop_npc_debug_session},
    logging::{emit_log, world_log_fields},
    persistence::{
        delete_all_named_gamestates, delete_named_gamestate, list_named_gamestates,
        load_named_gamestate, load_player_profile, save_named_gamestate,
    },
    server_state::{PersistMessage, ServerState},
    sync::{broadcast_full_sync, broadcast_patch, send_full_sync},
};

pub(crate) const SNAPSHOT_DB_PATH: &str = "data/antfarm.sqlite3";
const NPC_DEBUG_DIR: &str = "data";

pub(crate) async fn handle_client(stream: TcpStream, state: ServerState) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ServerMessage>();

    let mut player_id = None;

    let writer_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let payload = serde_json::to_string(&message)?;
            writer.write_all(payload.as_bytes()).await?;
            writer.write_all(b"\n").await?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let result: Result<()> = async {
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
                    tx.send(ServerMessage::Error {
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
                    tx.send(ServerMessage::Error {
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
            ClientMessage::RequestPheromoneMap { hive_id, channel } => {
                let map = {
                    let game = state.game.lock().await;
                    game.pheromone_map(hive_id, channel)
                };
                tx.send(ServerMessage::PheromoneMap(map))?;
            }
            ClientMessage::ConfigSet { path, value } => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
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
            ClientMessage::Give {
                target,
                resource,
                amount,
            } => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before changing inventory".to_string(),
                    })?;
                    continue;
                };

                let (maybe_patch, snapshot) = {
                    let mut game = state.game.lock().await;
                    game.give_resource(&target, &resource, amount)
                        .map_err(anyhow::Error::msg)?;
                    let patch = game.take_patch();
                    let snapshot = game.snapshot();
                    (patch, snapshot)
                };

                emit_log(
                    "sc_give",
                    json!({
                        "target": target,
                        "resource": resource,
                        "amount": amount,
                    }),
                );
                let _ = state.persistence_tx.send(PersistMessage::Save(snapshot));
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::FeedQueens { amount } => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before feeding queens".to_string(),
                    })?;
                    continue;
                };

                let (maybe_patch, snapshot) = {
                    let mut game = state.game.lock().await;
                    game.feed_queens(amount).map_err(anyhow::Error::msg)?;
                    let patch = game.take_patch();
                    let snapshot = game.snapshot();
                    (patch, snapshot)
                };

                emit_log("sc_feed_queens", json!({ "amount": amount }));
                let _ = state.persistence_tx.send(PersistMessage::Save(snapshot));
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::SetQueenEggs { eggs } => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before setting queen eggs".to_string(),
                    })?;
                    continue;
                };

                let (maybe_patch, snapshot) = {
                    let mut game = state.game.lock().await;
                    game.set_queen_eggs(eggs).map_err(anyhow::Error::msg)?;
                    let patch = game.take_patch();
                    let snapshot = game.snapshot();
                    (patch, snapshot)
                };

                emit_log("sc_set_queen_eggs", json!({ "eggs": eggs }));
                let _ = state.persistence_tx.send(PersistMessage::Save(snapshot));
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::Kill { selector } => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before killing NPCs".to_string(),
                    })?;
                    continue;
                };

                let (maybe_patch, snapshot) = {
                    let mut game = state.game.lock().await;
                    game.kill_by_selector(&selector).map_err(anyhow::Error::msg)?;
                    let patch = game.take_patch();
                    let snapshot = game.snapshot();
                    (patch, snapshot)
                };

                emit_log("sc_kill", json!({ "selector": selector }));
                let _ = state.persistence_tx.send(PersistMessage::Save(snapshot));
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::DigArea { width, height } => {
                let Some(id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before using dig".to_string(),
                    })?;
                    continue;
                };

                let (maybe_patch, snapshot) = {
                    let mut game = state.game.lock().await;
                    game.dig_area(id, width, height)
                        .map_err(anyhow::Error::msg)?;
                    let patch = game.take_patch();
                    let snapshot = game.snapshot();
                    (patch, snapshot)
                };

                emit_log(
                    "sc_dig",
                    json!({
                        "player_id": id,
                        "width": width,
                        "height": height,
                    }),
                );
                let _ = state.persistence_tx.send(PersistMessage::Save(snapshot));
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::PutArea {
                resource,
                width,
                height,
            } => {
                let Some(id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before using put".to_string(),
                    })?;
                    continue;
                };

                let logged_resource = resource.clone();
                let (maybe_patch, snapshot) = {
                    let mut game = state.game.lock().await;
                    game.put_area(id, &resource, width, height)
                        .map_err(anyhow::Error::msg)?;
                    let patch = game.take_patch();
                    let snapshot = game.snapshot();
                    (patch, snapshot)
                };

                emit_log(
                    "sc_put",
                    json!({
                        "player_id": id,
                        "resource": logged_resource,
                        "width": width,
                        "height": height,
                    }),
                );
                let _ = state.persistence_tx.send(PersistMessage::Save(snapshot));
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::SaveGameState { label } => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before saving game state".to_string(),
                    })?;
                    continue;
                };
                let label = label.trim();
                if label.is_empty() {
                    tx.send(ServerMessage::Error {
                        message: "save_gamestate label cannot be empty".to_string(),
                    })?;
                    continue;
                }
                let snapshot = {
                    let game = state.game.lock().await;
                    game.snapshot()
                };
                let save_id = save_named_gamestate(Path::new(SNAPSHOT_DB_PATH), label, &snapshot)?;
                let maybe_patch = {
                    let mut game = state.game.lock().await;
                    game.push_server_event(format!(
                        "Saved game state #{save_id}: {label} (tick {})",
                        snapshot.tick
                    ));
                    game.take_patch()
                };
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::ListGameStates => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before listing game states".to_string(),
                    })?;
                    continue;
                };
                let states = list_named_gamestates(Path::new(SNAPSHOT_DB_PATH))?;
                let summary = if states.is_empty() {
                    "Saved game states: none".to_string()
                } else {
                    let entries = states
                        .iter()
                        .take(5)
                        .map(|state| {
                            format!(
                                "#{} '{}' @ tick {} ({})",
                                state.id, state.label, state.tick, state.saved_at
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let suffix = if states.len() > 5 { ", ..." } else { "" };
                    format!("Saved game states: {entries}{suffix}")
                };
                let maybe_patch = {
                    let mut game = state.game.lock().await;
                    game.push_server_event(summary);
                    game.take_patch()
                };
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::LoadGameState { selector } => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before loading game state".to_string(),
                    })?;
                    continue;
                };
                let selector = selector.trim();
                if selector.is_empty() {
                    tx.send(ServerMessage::Error {
                        message: "load_gamestate requires an id or exact label".to_string(),
                    })?;
                    continue;
                }
                let Some(snapshot) = load_named_gamestate(Path::new(SNAPSHOT_DB_PATH), selector)? else {
                    tx.send(ServerMessage::Error {
                        message: format!("no saved game state matched: {selector}"),
                    })?;
                    continue;
                };
                let snapshot = {
                    let mut game = state.game.lock().await;
                    *game = antfarm_core::GameState::from_snapshot(snapshot);
                    game.push_server_event(format!("Loaded game state: {selector}"));
                    game.snapshot()
                };
                let _ = state.persistence_tx.send(PersistMessage::Save(snapshot.clone()));
                broadcast_full_sync(&state, &snapshot).await?;
            }
            ClientMessage::DeleteGameState { selector } => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before deleting game state".to_string(),
                    })?;
                    continue;
                };
                let selector = selector.trim();
                if selector.is_empty() {
                    tx.send(ServerMessage::Error {
                        message: "delete_gamestate requires an id or exact label".to_string(),
                    })?;
                    continue;
                }
                let deleted = delete_named_gamestate(Path::new(SNAPSHOT_DB_PATH), selector)?;
                if deleted == 0 {
                    tx.send(ServerMessage::Error {
                        message: format!("no saved game state matched: {selector}"),
                    })?;
                    continue;
                }
                emit_log(
                    "sc_delete_gamestate",
                    json!({ "selector": selector, "deleted": deleted }),
                );
                let maybe_patch = {
                    let mut game = state.game.lock().await;
                    game.push_server_event(format!(
                        "Deleted {deleted} saved game state(s): {selector}"
                    ));
                    game.take_patch()
                };
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::DeleteAllGameStates => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before deleting game states".to_string(),
                    })?;
                    continue;
                };
                let deleted = delete_all_named_gamestates(Path::new(SNAPSHOT_DB_PATH))?;
                emit_log("sc_delete_all_gamestates", json!({ "deleted": deleted }));
                let maybe_patch = {
                    let mut game = state.game.lock().await;
                    game.push_server_event(format!("Deleted {deleted} saved game state(s)"));
                    game.take_patch()
                };
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::SetSimulationPaused { paused } => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before changing simulation pause state".to_string(),
                    })?;
                    continue;
                };
                let (maybe_patch, snapshot) = {
                    let mut game = state.game.lock().await;
                    game.set_simulation_paused(paused);
                    let patch = game.take_patch();
                    let snapshot = game.snapshot();
                    (patch, snapshot)
                };
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
            ClientMessage::DebugNpcStart => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before starting NPC debug".to_string(),
                    })?;
                    continue;
                };
                if state.npc_debug.lock().await.is_some() {
                    tx.send(ServerMessage::Error {
                        message: "NPC debug is already active".to_string(),
                    })?;
                    continue;
                }
                let session = start_npc_debug_session(Path::new(NPC_DEBUG_DIR))?;
                let path = session.path.display().to_string();
                {
                    let mut game = state.game.lock().await;
                    game.set_npc_debug_enabled(true);
                    game.push_server_event(format!("NPC debug started: {path}"));
                }
                *state.npc_debug.lock().await = Some(session);
                let maybe_patch = {
                    let mut game = state.game.lock().await;
                    game.take_patch()
                };
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::DebugNpcStop => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before stopping NPC debug".to_string(),
                    })?;
                    continue;
                };
                let session = state.npc_debug.lock().await.take();
                let Some(session) = session else {
                    tx.send(ServerMessage::Error {
                        message: "NPC debug is not active".to_string(),
                    })?;
                    continue;
                };
                let path = session.path.display().to_string();
                {
                    let mut game = state.game.lock().await;
                    game.set_npc_debug_enabled(false);
                    game.take_npc_debug_events();
                    game.push_server_event(format!("NPC debug stopped: {path}"));
                }
                stop_npc_debug_session(session);
                let maybe_patch = {
                    let mut game = state.game.lock().await;
                    game.take_patch()
                };
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
            ClientMessage::DebugNpcStatus => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before checking NPC debug status".to_string(),
                    })?;
                    continue;
                };
                let active_path = state
                    .npc_debug
                    .lock()
                    .await
                    .as_ref()
                    .map(|session| session.path.display().to_string());
                let maybe_patch = {
                    let mut game = state.game.lock().await;
                    match active_path {
                        Some(path) => game.push_server_event(format!("NPC debug active: {path}")),
                        None => game.push_server_event("NPC debug inactive".to_string()),
                    }
                    game.take_patch()
                };
                if let Some(patch) = maybe_patch {
                    broadcast_patch(&state, &patch, None).await?;
                }
            }
        }
        }
        Ok(())
    }
    .await;

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
    result
}
