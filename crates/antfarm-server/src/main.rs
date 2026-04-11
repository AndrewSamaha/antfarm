use antfarm_core::{ClientMessage, GameState, JoinAck, ServerMessage, Snapshot, TICK_MILLIS};
use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
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

type ClientTx = tokio::sync::mpsc::UnboundedSender<ServerMessage>;

#[derive(Clone)]
struct ServerState {
    game: Arc<Mutex<GameState>>,
    clients: Arc<Mutex<HashMap<u8, ClientTx>>>,
    persistence_tx: mpsc::Sender<PersistMessage>,
}

enum PersistMessage {
    Save(Snapshot),
}

#[tokio::main]
async fn main() -> Result<()> {
    let snapshot_path = PathBuf::from(SNAPSHOT_DB_PATH);
    let persistence_tx = spawn_persistence_worker(snapshot_path.clone())?;
    let initial_game = load_startup_game(&snapshot_path)?;

    let listener = TcpListener::bind(SERVER_ADDR).await?;
    let state = ServerState {
        game: Arc::new(Mutex::new(initial_game)),
        clients: Arc::new(Mutex::new(HashMap::new())),
        persistence_tx,
    };

    {
        let tick_state = state.clone();
        tokio::spawn(async move {
            let mut ticker = time::interval(Duration::from_millis(TICK_MILLIS));
            let mut last_snapshot_at = Instant::now();
            loop {
                ticker.tick().await;
                let maybe_snapshot = {
                    let mut game = tick_state.game.lock().await;
                    game.tick();

                    let interval = Duration::from_secs_f64(game.snapshot_interval_seconds());
                    if last_snapshot_at.elapsed() >= interval {
                        last_snapshot_at = Instant::now();
                        Some(game.snapshot())
                    } else {
                        None
                    }
                };

                if let Some(snapshot) = maybe_snapshot {
                    let _ = tick_state.persistence_tx.send(PersistMessage::Save(snapshot));
                }

                if let Err(error) = broadcast_snapshot(&tick_state).await {
                    eprintln!("broadcast error: {error}");
                }
            }
        });
    }

    println!("antfarm-server listening on {SERVER_ADDR}");

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_client(stream, state).await {
                eprintln!("client disconnected with error: {error}");
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
            ClientMessage::Join { name } => {
                if player_id.is_some() {
                    continue;
                }

                let joined = {
                    let mut game = state.game.lock().await;
                    game.add_player(name)
                };

                match joined {
                    Ok((id, snapshot)) => {
                        state.clients.lock().await.insert(id, tx.clone());
                        player_id = Some(id);
                        tx.send(ServerMessage::Joined(JoinAck {
                            player_id: id,
                            snapshot,
                        }))?;
                        broadcast_snapshot(&state).await?;
                    }
                    Err(message) => {
                        tx.send(ServerMessage::Error { message })?;
                    }
                }
            }
            ClientMessage::Action(action) => {
                let Some(id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before sending actions".to_string(),
                    })?;
                    continue;
                };

                {
                    let mut game = state.game.lock().await;
                    game.apply_action(id, action);
                }
                broadcast_snapshot(&state).await?;
            }
            ClientMessage::ConfigSet { path, value } => {
                let Some(_id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before changing config".to_string(),
                    })?;
                    continue;
                };

                let result = {
                    let mut game = state.game.lock().await;
                    game.set_config_value(&path, value)
                };

                if let Err(message) = result {
                    tx.send(ServerMessage::Error { message })?;
                    continue;
                }

                broadcast_snapshot(&state).await?;
            }
            ClientMessage::WorldReset => {
                {
                    let mut game = state.game.lock().await;
                    game.world_reset();
                    let _ = state.persistence_tx.send(PersistMessage::Save(game.snapshot()));
                }
                broadcast_snapshot(&state).await?;
            }
        }
    }

    if let Some(id) = player_id {
        state.clients.lock().await.remove(&id);
        {
            let mut game = state.game.lock().await;
            game.remove_player(id);
        }
        broadcast_snapshot(&state).await?;
    }

    writer_task.abort();
    Ok(())
}

async fn broadcast_snapshot(state: &ServerState) -> Result<()> {
    let snapshot = {
        let game = state.game.lock().await;
        game.snapshot()
    };

    let clients = state.clients.lock().await;
    for tx in clients.values() {
        let _ = tx.send(ServerMessage::Snapshot(snapshot.clone()));
    }
    Ok(())
}

fn load_startup_game(path: &Path) -> Result<GameState> {
    if let Some(snapshot) = load_latest_snapshot(path)? {
        Ok(GameState::from_snapshot(snapshot))
    } else {
        Ok(GameState::new())
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
        ",
    )?;
    Ok(connection)
}

fn load_latest_snapshot(path: &Path) -> Result<Option<Snapshot>> {
    if !path.exists() {
        return Ok(None);
    }

    let connection = open_db(path)?;
    let mut statement = connection.prepare(
        "SELECT state_json FROM world_snapshots ORDER BY id DESC LIMIT 1",
    )?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let state_json: String = row.get(0)?;
    let snapshot = serde_json::from_str::<Snapshot>(&state_json)?;
    Ok(Some(snapshot))
}

fn save_snapshot(connection: &Connection, snapshot: &Snapshot) -> Result<()> {
    let state_json = serde_json::to_string(snapshot)?;
    connection.execute(
        "INSERT INTO world_snapshots (tick, state_json) VALUES (?1, ?2)",
        params![snapshot.tick as i64, state_json],
    )?;
    Ok(())
}

fn prune_snapshots(connection: &Connection) -> Result<()> {
    connection.execute(
        "
        DELETE FROM world_snapshots
        WHERE id NOT IN (
            SELECT id
            FROM world_snapshots
            ORDER BY id DESC
            LIMIT ?1
        )
        ",
        params![SNAPSHOT_RETENTION],
    )?;
    Ok(())
}
