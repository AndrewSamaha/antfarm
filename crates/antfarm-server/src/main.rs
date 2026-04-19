mod client_session;
mod debug_npc;
mod logging;
mod persistence;
mod runtime;
mod server_state;
mod sync;

use anyhow::Result;
use serde_json::json;
use std::{
    env,
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::Arc,
};
use tokio::{net::TcpListener, sync::Mutex};

use crate::{
    client_session::{SNAPSHOT_DB_PATH, handle_client},
    logging::{emit_log, world_log_fields},
    persistence::{list_named_gamestates, load_startup_game, spawn_persistence_worker},
    runtime::spawn_background_tasks,
    server_state::ServerState,
};

const SERVER_ADDR: &str = "127.0.0.1:7000";

fn print_help() {
    println!(
        "\
antfarm-server

Usage:
  antfarm-server [OPTIONS]

Options:
  -h, --help                   Show this help text and exit
      --reset-world            Delete the world database before startup
      --paused                 Start the simulation in the paused state
      --list-gamestates        List saved gamestate bookmarks and exit
      --load-gamestate VALUE   Start from a saved gamestate by id or exact label
"
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let show_help = args.iter().any(|arg| arg == "-h" || arg == "--help");
    if show_help {
        print_help();
        return Ok(());
    }
    let reset_world = args.iter().any(|arg| arg == "--reset-world");
    let start_paused = args.iter().any(|arg| arg == "--paused");
    let list_gamestates = args.iter().any(|arg| arg == "--list-gamestates");
    let mut load_gamestate = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--load-gamestate" {
            let selector = args
                .get(index + 1)
                .ok_or_else(|| anyhow::anyhow!("--load-gamestate requires an id or exact label"))?;
            load_gamestate = Some(selector.clone());
            index += 1;
        }
        index += 1;
    }
    let snapshot_path = PathBuf::from(SNAPSHOT_DB_PATH);
    emit_log(
        "starting_server",
        json!({
            "addr": SERVER_ADDR,
            "snapshot_db": snapshot_path.display().to_string(),
            "reset_world": reset_world,
            "start_paused": start_paused,
            "list_gamestates": list_gamestates,
            "load_gamestate": load_gamestate,
        }),
    );
    if list_gamestates {
        let states = list_named_gamestates(&snapshot_path)?;
        for state in states {
            println!(
                "{}\t{}\t{}\ttick={}",
                state.id, state.saved_at, state.label, state.tick
            );
        }
        return Ok(());
    }
    if reset_world && snapshot_path.exists() {
        fs::remove_file(&snapshot_path)?;
        emit_log(
            "world_db_deleted",
            json!({
                "snapshot_db": snapshot_path.display().to_string(),
            }),
        );
    }
    let persistence_tx = spawn_persistence_worker(snapshot_path.clone())?;
    let (initial_game, restored) =
        load_startup_game(&snapshot_path, start_paused, load_gamestate.as_deref())?;

    emit_log(
        "server_start",
        json!({
            "addr": SERVER_ADDR,
            "snapshot_db": snapshot_path.display().to_string(),
            "restored_snapshot": restored,
            "simulation_paused": initial_game.simulation_paused,
            "load_gamestate": load_gamestate,
            "world": world_log_fields(&initial_game),
        }),
    );

    let listener = TcpListener::bind(SERVER_ADDR).await?;
    let state = ServerState {
        game: Arc::new(Mutex::new(initial_game)),
        clients: Arc::new(Mutex::new(HashMap::new())),
        session_tokens: Arc::new(Mutex::new(HashMap::new())),
        persistence_tx,
        npc_debug: Arc::new(Mutex::new(None)),
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
