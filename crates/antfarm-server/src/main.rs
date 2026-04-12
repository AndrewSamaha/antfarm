use antfarm_core::{
    ClientMessage, FullSyncChunk, FullSyncComplete, FullSyncStart, GameState, PatchFrame, Player,
    ServerMessage, Snapshot, TICK_MILLIS, default_server_config,
};
use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time,
};

const SERVER_ADDR: &str = "127.0.0.1:7000";
const SNAPSHOT_DB_PATH: &str = "data/antfarm.sqlite3";
const SNAPSHOT_RETENTION: i64 = 10;
const HEARTBEAT_INTERVAL_SECONDS: u64 = 30;
const FULL_SYNC_ROWS_PER_CHUNK: i32 = 16;

type ClientTx = tokio::sync::mpsc::UnboundedSender<ServerMessage>;

#[derive(Clone)]
struct ServerState {
    game: Arc<Mutex<GameState>>,
    clients: Arc<Mutex<HashMap<u8, ClientTx>>>,
    session_tokens: Arc<Mutex<HashMap<u8, String>>>,
    persistence_tx: mpsc::Sender<PersistMessage>,
}

enum PersistMessage {
    Save(Snapshot),
    UpsertPlayerProfile { token: String, player: Player },
    ClearPlayerProfiles,
}

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

async fn broadcast_patch(
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

async fn broadcast_full_sync(state: &ServerState, snapshot: &Snapshot) -> Result<()> {
    let clients = state.clients.lock().await;
    for (player_id, tx) in clients.iter() {
        send_full_sync(tx, *player_id, snapshot)?;
    }
    Ok(())
}

fn send_full_sync(tx: &ClientTx, player_id: u8, snapshot: &Snapshot) -> Result<()> {
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
        let rows = (row..end)
            .map(|y| snapshot.world.row_tiles(y))
            .collect();
        tx.send(ServerMessage::FullSyncChunk(FullSyncChunk {
            start_row: row,
            rows,
        }))?;
        row = end;
    }

    tx.send(ServerMessage::FullSyncComplete(FullSyncComplete {
        players: snapshot.players.clone(),
        npcs: snapshot.npcs.clone(),
        event_log: snapshot.event_log.clone(),
        config: snapshot.config.clone(),
    }))?;
    Ok(())
}

fn load_startup_game(path: &Path) -> Result<(GameState, bool)> {
    if let Some(snapshot) = load_latest_snapshot(path)? {
        Ok((GameState::from_snapshot(snapshot), true))
    } else {
        let config = default_server_config();
        emit_log(
            "generating_world",
            json!({
                "source": "startup",
                "world": {
                    "seed": config.pointer("/world/seed").and_then(Value::as_u64),
                    "max_depth": config.pointer("/world/max_depth").and_then(Value::as_i64),
                    "gen_params": config.pointer("/world/gen_params").cloned(),
                }
            }),
        );
        Ok((GameState::from_config(config), false))
    }
}

fn spawn_persistence_worker(path: PathBuf) -> Result<mpsc::Sender<PersistMessage>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let (tx, rx) = mpsc::channel::<PersistMessage>();
    thread::spawn(move || {
        if let Err(error) = persistence_worker_loop(path, rx) {
            eprintln!("persistence worker failed: {error}");
        }
    });
    Ok(tx)
}

fn persistence_worker_loop(path: PathBuf, rx: mpsc::Receiver<PersistMessage>) -> Result<()> {
    let connection = open_db(&path)?;
    for message in rx {
        match message {
            PersistMessage::Save(snapshot) => {
                save_snapshot(&connection, &snapshot)?;
                prune_snapshots(&connection)?;
                emit_log(
                    "snapshot_saved",
                    json!({ "tick": snapshot.tick, "retention": SNAPSHOT_RETENTION }),
                );
            }
            PersistMessage::UpsertPlayerProfile { token, player } => {
                save_player_profile(&connection, &token, &player)?;
            }
            PersistMessage::ClearPlayerProfiles => {
                clear_player_profiles(&connection)?;
            }
        }
    }
    Ok(())
}

fn open_db(path: &Path) -> Result<Connection> {
    let connection = Connection::open(path)
        .with_context(|| format!("open sqlite database at {}", path.display()))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "busy_timeout", 5_000)?;
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS world_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tick INTEGER NOT NULL,
            saved_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            state_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS player_profiles (
            token TEXT PRIMARY KEY,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            player_json TEXT NOT NULL
        );
        ",
    )?;
    Ok(connection)
}

fn load_latest_snapshot(path: &Path) -> Result<Option<Snapshot>> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = open_db(path)?;
    let mut statement =
        connection.prepare("SELECT state_json FROM world_snapshots ORDER BY id DESC LIMIT 1")?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let state_json: String = row.get(0)?;
    Ok(Some(serde_json::from_str::<Snapshot>(&state_json)?))
}

fn save_snapshot(connection: &Connection, snapshot: &Snapshot) -> Result<()> {
    let state_json = serde_json::to_string(snapshot)?;
    connection.execute(
        "INSERT INTO world_snapshots (tick, state_json) VALUES (?1, ?2)",
        params![snapshot.tick as i64, state_json],
    )?;
    Ok(())
}

fn load_player_profile(path: &Path, token: &str) -> Result<Option<Player>> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = open_db(path)?;
    let mut statement =
        connection.prepare("SELECT player_json FROM player_profiles WHERE token = ?1 LIMIT 1")?;
    let mut rows = statement.query(params![token])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let player_json: String = row.get(0)?;
    Ok(Some(serde_json::from_str::<Player>(&player_json)?))
}

fn save_player_profile(connection: &Connection, token: &str, player: &Player) -> Result<()> {
    let player_json = serde_json::to_string(player)?;
    connection.execute(
        "
        INSERT INTO player_profiles (token, player_json, updated_at)
        VALUES (?1, ?2, CURRENT_TIMESTAMP)
        ON CONFLICT(token) DO UPDATE SET
            player_json = excluded.player_json,
            updated_at = CURRENT_TIMESTAMP
        ",
        params![token, player_json],
    )?;
    Ok(())
}

fn clear_player_profiles(connection: &Connection) -> Result<()> {
    connection.execute("DELETE FROM player_profiles", [])?;
    Ok(())
}

fn prune_snapshots(connection: &Connection) -> Result<()> {
    connection.execute(
        "
        DELETE FROM world_snapshots
        WHERE id NOT IN (
            SELECT id FROM world_snapshots ORDER BY id DESC LIMIT ?1
        )
        ",
        params![SNAPSHOT_RETENTION],
    )?;
    Ok(())
}

fn emit_log(event: &str, fields: Value) {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let mut object = serde_json::Map::new();
    object.insert("ts_ms".to_string(), Value::from(ts_ms));
    object.insert("event".to_string(), Value::from(event));
    if let Value::Object(extra) = fields {
        for (key, value) in extra {
            object.insert(key, value);
        }
    }
    println!("{}", Value::Object(object));
}

fn world_log_fields(game: &GameState) -> Value {
    json!({
        "tick": game.tick,
        "width": game.world.width(),
        "height": game.world.height(),
        "seed": game.config.pointer("/world/seed").and_then(Value::as_u64),
        "max_depth": game.config.pointer("/world/max_depth").and_then(Value::as_i64),
        "gen_params": game.config.pointer("/world/gen_params").cloned(),
    })
}
