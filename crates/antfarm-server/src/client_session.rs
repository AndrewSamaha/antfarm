use antfarm_core::{ClientMessage, ServerMessage};
use anyhow::Result;
use serde_json::json;
use std::path::Path;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

use crate::{
    logging::{emit_log, world_log_fields},
    persistence::load_player_profile,
    server_state::{PersistMessage, ServerState},
    sync::{broadcast_full_sync, broadcast_patch, send_full_sync},
};

pub(crate) const SNAPSHOT_DB_PATH: &str = "data/antfarm.sqlite3";

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
