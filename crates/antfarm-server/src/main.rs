mod client_session;
mod logging;
mod persistence;
mod runtime;
mod server_state;
mod sync;

use anyhow::Result;
use serde_json::json;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
};
use tokio::{net::TcpListener, sync::Mutex};

use crate::{
    client_session::{SNAPSHOT_DB_PATH, handle_client},
    logging::{emit_log, world_log_fields},
    persistence::{load_startup_game, spawn_persistence_worker},
    runtime::spawn_background_tasks,
    server_state::ServerState,
};

const SERVER_ADDR: &str = "127.0.0.1:7000";

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

    spawn_background_tasks(&state);

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
