mod client_session;
mod debug_npc;
mod discovery;
mod experiment;
mod logging;
mod persistence;
mod runtime;
mod server_state;
mod startup_commands;
mod sync;

use anyhow::{Context, Result};
use antfarm_core::{ReplayArtifact, config_string, config_u16, set_config_path};
use serde_json::json;
use std::{
    collections::HashMap,
    env,
    fs,
    path::PathBuf,
    sync::Arc,
};
use tokio::{net::TcpListener, sync::{Mutex, Notify}};

use crate::{
    client_session::{SNAPSHOT_DB_PATH, handle_client},
    discovery::start_mdns_registration,
    debug_npc::start_npc_debug_session_at_path,
    experiment::{
        condition_plan, datetime_seed, debug_log_path, load_server_config, maybe_create_run_context,
        persist_run_manifest, resolve_server_config,
    },
    logging::{emit_log, world_log_fields},
    persistence::{
        delete_all_named_gamestates, delete_named_gamestate, list_named_gamestates,
        load_startup_game, reset_world_state_preserve_gamestates, spawn_persistence_worker,
    },
    runtime::spawn_background_tasks,
    server_state::ServerState,
    startup_commands::run_startup_sc_commands,
};

fn print_help() {
    println!(
        "\
antfarm-server

Usage:
  antfarm-server [OPTIONS]

Options:
      -h, --help                   Show this help text and exit
      --server-config VALUE    Load server settings from a YAML file
      --condition VALUE        Select one named experiment condition from server.yaml
      --list-condition-plan    Print condition names and configured run counts, then exit
      --print-visualizations-json  Print experiment visualization specs as JSON and exit
      --replay-artifact VALUE  Replay one deterministic replay artifact and exit
      --reset-world            Clear live world snapshots and player profiles before startup
      --paused                 Start the simulation in the paused state
      --list-gamestates        List saved gamestate bookmarks and exit
      --load-gamestate VALUE   Start from a saved gamestate by id or exact label
      --delete-gamestate VALUE Delete a saved gamestate by id or exact label and exit
      --delete-all-gamestates  Delete all saved gamestates and exit
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
    let list_condition_plan = args.iter().any(|arg| arg == "--list-condition-plan");
    let print_visualizations_json =
        args.iter().any(|arg| arg == "--print-visualizations-json");
    let delete_all_gamestates = args.iter().any(|arg| arg == "--delete-all-gamestates");
    let mut server_config_path = None;
    let mut condition = None;
    let mut load_gamestate = None;
    let mut delete_gamestate = None;
    let mut replay_artifact_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--server-config" => {
                let path = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("--server-config requires a path"))?;
                server_config_path = Some(path.clone());
                index += 1;
            }
            "--condition" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("--condition requires a name"))?;
                condition = Some(value.clone());
                index += 1;
            }
            "--load-gamestate" => {
                let selector = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("--load-gamestate requires an id or exact label"))?;
                load_gamestate = Some(selector.clone());
                index += 1;
            }
            "--delete-gamestate" => {
                let selector = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("--delete-gamestate requires an id or exact label"))?;
                delete_gamestate = Some(selector.clone());
                index += 1;
            }
            "--replay-artifact" => {
                let path = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("--replay-artifact requires a path"))?;
                replay_artifact_path = Some(path.clone());
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    if let Some(path) = replay_artifact_path {
        return run_replay_artifact(PathBuf::from(path));
    }
    let loaded_server_config = load_server_config(server_config_path.as_deref())?;
    if list_condition_plan {
        for entry in condition_plan(&loaded_server_config.file)? {
            println!(
                "{}\t{}",
                entry.name.as_deref().unwrap_or("-"),
                entry.runs
            );
        }
        return Ok(());
    }
    if print_visualizations_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&loaded_server_config.file.experiment.visualizations)?
        );
        return Ok(());
    }
    let resolved_server_config = resolve_server_config(&loaded_server_config.file, condition.as_deref())?;
    let start_paused = start_paused || resolved_server_config.startup.paused;
    let reset_world = reset_world || resolved_server_config.startup.reset_world;
    let load_gamestate = load_gamestate.or(resolved_server_config.startup.load_gamestate.clone());
    let server_bind_host = config_string(
        &resolved_server_config.config,
        "network.bind_host",
        "0.0.0.0",
    );
    let server_port = config_u16(&resolved_server_config.config, "network.port", 14461);
    let server_addr = format!("{server_bind_host}:{server_port}");
    let snapshot_path = PathBuf::from(SNAPSHOT_DB_PATH);
    let mut experiment_context =
        maybe_create_run_context(
            loaded_server_config.path.as_deref(),
            &resolved_server_config.experiment,
            resolved_server_config.condition_name.as_deref(),
        )?;
    let mut startup_config_override = resolved_server_config.config.clone();
    if resolved_server_config.experiment.randomize_seed_from_datetime {
        let seed = datetime_seed()?;
        set_config_path(&mut startup_config_override, "world.seed", json!(seed))
            .map_err(anyhow::Error::msg)?;
        if let Some(context) = experiment_context.as_mut() {
            context.randomized_seed = Some(seed);
            persist_run_manifest(context)?;
        }
    }
    emit_log(
        "starting_server",
        json!({
            "addr": server_addr,
            "snapshot_db": snapshot_path.display().to_string(),
            "server_config": loaded_server_config.path.as_ref().map(|path| path.display().to_string()),
            "condition": resolved_server_config.condition_name,
            "reset_world": reset_world,
            "start_paused": start_paused,
            "list_gamestates": list_gamestates,
            "list_condition_plan": list_condition_plan,
            "load_gamestate": load_gamestate,
            "delete_gamestate": delete_gamestate,
            "delete_all_gamestates": delete_all_gamestates,
            "experiment_run_dir": experiment_context.as_ref().map(|ctx| ctx.run_dir.display().to_string()),
            "randomized_seed": experiment_context.as_ref().and_then(|ctx| ctx.randomized_seed),
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
    if let Some(selector) = delete_gamestate {
        let deleted = delete_named_gamestate(&snapshot_path, &selector)?;
        println!("deleted_gamestates={deleted}");
        return Ok(());
    }
    if delete_all_gamestates {
        let deleted = delete_all_named_gamestates(&snapshot_path)?;
        println!("deleted_gamestates={deleted}");
        return Ok(());
    }
    if reset_world {
        reset_world_state_preserve_gamestates(&snapshot_path)?;
        emit_log(
            "world_state_reset",
            json!({
                "snapshot_db": snapshot_path.display().to_string(),
                "preserved_named_gamestates": true,
            }),
        );
    }
    let persistence_tx = spawn_persistence_worker(snapshot_path.clone())?;
    let (initial_game, restored) =
        load_startup_game(
            &snapshot_path,
            start_paused,
            load_gamestate.as_deref(),
            &startup_config_override,
        )?;
    if let Some(context) = experiment_context.as_mut() {
        context.start_tick = initial_game.tick;
        persist_run_manifest(context)?;
    }

    emit_log(
        "server_start",
        json!({
            "addr": server_addr,
            "snapshot_db": snapshot_path.display().to_string(),
            "restored_snapshot": restored,
            "simulation_paused": initial_game.simulation_paused,
            "start_tick": initial_game.tick,
            "load_gamestate": load_gamestate,
            "server_config": loaded_server_config.path.as_ref().map(|path| path.display().to_string()),
            "condition": resolved_server_config.condition_name,
            "randomized_seed": experiment_context.as_ref().and_then(|ctx| ctx.randomized_seed),
            "world": world_log_fields(&initial_game),
        }),
    );

    let mdns_registration = match start_mdns_registration(&server_bind_host, server_port) {
        Ok(registration) => {
            emit_log(
                "mdns_advertisement_started",
                json!({
                    "service": registration.fullname(),
                    "bind_host": server_bind_host,
                    "port": server_port,
                }),
            );
            Some(registration)
        }
        Err(error) => {
            emit_log(
                "mdns_advertisement_failed",
                json!({
                    "bind_host": server_bind_host,
                    "port": server_port,
                    "error": error.to_string(),
                }),
            );
            None
        }
    };

    let listener = TcpListener::bind(&server_addr).await?;
    let tick_millis = experiment_context
        .as_ref()
        .map(|ctx| ctx.tick_millis)
        .unwrap_or(antfarm_core::TICK_MILLIS);
    let state = ServerState {
        game: Arc::new(Mutex::new(initial_game)),
        clients: Arc::new(Mutex::new(HashMap::new())),
        session_tokens: Arc::new(Mutex::new(HashMap::new())),
        persistence_tx,
        npc_debug: Arc::new(Mutex::new(None)),
        experiment: Arc::new(Mutex::new(experiment_context)),
        shutdown_notify: Arc::new(Notify::new()),
        tick_millis,
    };

    let auto_debug_session = {
        let experiment = state.experiment.lock().await;
        if let Some(context) = experiment.as_ref() {
            if context.debug_log {
                Some(start_npc_debug_session_at_path(&debug_log_path(context))?)
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(session) = auto_debug_session {
        {
            let mut game = state.game.lock().await;
            game.set_npc_debug_enabled(true);
            game.push_server_event(format!("NPC debug started: {}", session.path.display()));
            let _ = game.take_patch();
        }
        *state.npc_debug.lock().await = Some(session);
    }

    let npc_debug_dir = state
        .experiment
        .lock()
        .await
        .as_ref()
        .map(|ctx| ctx.run_dir.clone())
        .unwrap_or_else(|| PathBuf::from("data"));
    run_startup_sc_commands(
        &state,
        &snapshot_path,
        &npc_debug_dir,
        &resolved_server_config.startup.sc_commands,
    )
    .await?;
    {
        let game = state.game.lock().await;
        let mut experiment = state.experiment.lock().await;
        if let Some(context) = experiment.as_mut() {
            if context.replay_save {
                context.initial_snapshot = Some(game.snapshot());
                persist_run_manifest(context)?;
            }
        }
    }

    spawn_background_tasks(&state);

    emit_log("server_listening", json!({ "addr": server_addr }));

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                let (stream, _) = accept_result?;
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_client(stream, state).await {
                        emit_log("client_session_error", json!({ "error": error.to_string() }));
                    }
                });
            }
            _ = state.shutdown_notify.notified() => {
                emit_log("server_shutdown_requested", json!({ "reason": "experiment_completed" }));
                break;
            }
        }
    }

    drop(mdns_registration);

    Ok(())
}

fn run_replay_artifact(path: PathBuf) -> Result<()> {
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("read replay artifact {}", path.display()))?;
    let artifact = serde_json::from_str::<ReplayArtifact>(&raw)
        .with_context(|| format!("parse replay artifact {}", path.display()))?;
    let verification = artifact
        .replay()
        .context("replay deterministic artifact")?;
    emit_log(
        "replay_finished",
        json!({
            "artifact_path": path.display().to_string(),
            "matches_expected": verification.matches_expected,
            "initial_snapshot_hash": verification.initial_snapshot_hash,
            "expected_final_snapshot_hash": verification.expected_final_snapshot_hash,
            "actual_final_snapshot_hash": verification.actual_final_snapshot_hash,
            "final_tick": verification.final_tick,
        }),
    );
    println!("{}", serde_json::to_string_pretty(&verification)?);
    if verification.matches_expected {
        Ok(())
    } else {
        anyhow::bail!("replay final snapshot hash did not match expected hash");
    }
}
