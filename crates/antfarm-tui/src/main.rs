mod app;
mod art;
mod client_files;
mod commands;
mod discovery;
mod input;
mod modals;
mod network;
mod render;

use crate::{
    app::{App, handle_server_message},
    client_files::{ephemeral_client_config, load_command_history, load_or_create_client_config},
    discovery::{DiscoveryUpdate, probe_localhost_server, spawn_mdns_discovery},
    input::handle_event,
    network::{
        Connection, RECONNECT_ATTEMPT_TIMEOUT, connect_session, offline_snapshot,
        recv_server_message, tokio_stream_event,
    },
    render::draw,
};
use anyhow::Result;
use antfarm_core::{ClientMessage, GameState, ReplayArtifact, TICK_MILLIS};
use crossterm::{
    event::EventStream,
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::DefaultTerminal;
use std::{env, fs, io, path::PathBuf, time::Duration};
use tokio::time::{self, timeout};

struct ClientRuntimeOptions {
    player_name: String,
    dev_mode: bool,
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let terminal = ratatui::init();

    let result = if args.first().is_some_and(|arg| arg == "--replay") {
        let path = args
            .get(1)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("--replay requires a replay artifact path"))?;
        run_replay_app(terminal, PathBuf::from(path)).await
    } else {
        let options = parse_client_options(&args)?;
        run_app(terminal, options).await
    };

    ratatui::restore();
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    result
}

async fn run_replay_app(mut terminal: DefaultTerminal, replay_path: PathBuf) -> Result<()> {
    let raw = fs::read_to_string(&replay_path)?;
    let artifact = serde_json::from_str::<ReplayArtifact>(&raw)?;
    let mut snapshot = artifact.initial_snapshot.clone();
    snapshot.simulation_paused = true;
    let mut app = App::new_replay(snapshot, 100);
    app.set_info(format!("Loaded replay {}", replay_path.display()));
    let mut game = GameState::from_replay_snapshot(artifact.initial_snapshot.clone());
    sync_replay_pheromone_map(&mut app, &game);
    let mut events = EventStream::new();
    let mut redraw = time::interval(Duration::from_millis(33));
    let mut playback = time::interval(Duration::from_millis(TICK_MILLIS));
    redraw.tick().await;
    playback.tick().await;
    let mut finished = false;

    loop {
        terminal.draw(|frame| draw(frame, &app))?;

        tokio::select! {
            _ = redraw.tick() => app.tick_animation(),
            _ = playback.tick(), if !app.snapshot.simulation_paused && !finished => {
                if game.tick >= artifact.expected_final_tick {
                    finished = true;
                    app.snapshot.simulation_paused = true;
                    app.set_info("Replay finished");
                    continue;
                }
                game.tick();
                app.snapshot = game.snapshot();
                app.snapshot.simulation_paused = false;
                sync_replay_pheromone_map(&mut app, &game);
                if game.tick >= artifact.expected_final_tick {
                    finished = true;
                    app.snapshot.simulation_paused = true;
                    app.set_info("Replay finished");
                }
            }
            maybe_event = tokio_stream_event(&mut events) => {
                if let Some(event) = maybe_event? {
                    if handle_event(event, &mut app, None).await? {
                        break;
                    }
                    sync_replay_pheromone_map(&mut app, &game);
                }
            }
        }
    }

    Ok(())
}

fn parse_client_options(args: &[String]) -> Result<ClientRuntimeOptions> {
    let mut dev_mode = false;
    let mut player_name = None;
    let mut port = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dev" => dev_mode = true,
            "--port" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("--port requires a value"))?;
                let parsed = raw
                    .parse::<u16>()
                    .map_err(|_| anyhow::anyhow!("--port must be a valid u16"))?;
                if parsed == 0 {
                    return Err(anyhow::anyhow!("--port must be greater than zero"));
                }
                index += 1;
                port = Some(parsed);
            }
            value if value.starts_with('-') => {
                return Err(anyhow::anyhow!("unknown client option: {value}"));
            }
            value => {
                if player_name.is_some() {
                    return Err(anyhow::anyhow!(
                        "expected at most one player name, got extra argument: {value}"
                    ));
                }
                player_name = Some(value.to_string());
            }
        }
        index += 1;
    }

    Ok(ClientRuntimeOptions {
        player_name: player_name.unwrap_or_else(|| "worker-ant".to_string()),
        dev_mode,
        port: port.unwrap_or(14461),
    })
}

fn sync_replay_pheromone_map(app: &mut App, game: &GameState) {
    let Some(channel) = app.pheromone_overlay else {
        app.pheromone_map = None;
        return;
    };
    let Some(hive_id) = app.preferred_hive_id() else {
        app.pheromone_map = None;
        return;
    };
    app.pheromone_map = Some(game.pheromone_map(hive_id, channel));
}

async fn run_app(mut terminal: DefaultTerminal, options: ClientRuntimeOptions) -> Result<()> {
    let client_config = if options.dev_mode {
        ephemeral_client_config()
    } else {
        load_or_create_client_config(&options.player_name)?
    };
    let client_token = client_config.token.clone();
    let mut app = App::new(
        options.player_name.clone(),
        0,
        offline_snapshot(),
        client_config.show_help_at_startup,
        client_config.max_history,
    );
    app.persist_client_files = !options.dev_mode;
    if app.persist_client_files {
        app.command_history = load_command_history(&options.player_name, app.max_history)?;
    }
    app.begin_server_selection();
    if let Some(localhost) = probe_localhost_server(options.port).await {
        app.upsert_discovered_server(localhost);
    }
    let mut events = EventStream::new();
    let mut redraw = time::interval(Duration::from_millis(33));
    let mut reconnect = time::interval(Duration::from_millis(1000));
    let mut pheromone_refresh = time::interval(Duration::from_millis(500));
    let localhost_port = app
        .discovered_servers
        .iter()
        .find(|server| matches!(server.source, crate::discovery::DiscoverySource::Localhost))
        .map(|server| server.port);
    let mut discovery_rx = spawn_mdns_discovery(localhost_port);
    reconnect.tick().await;
    pheromone_refresh.tick().await;
    let mut connection: Option<Connection> = None;

    loop {
        terminal.draw(|frame| draw(frame, &app))?;

        tokio::select! {
            _ = redraw.tick() => app.tick_animation(),
            _ = reconnect.tick(), if connection.is_none() && app.selected_server_addr.is_some() && !app.is_selecting_server() => {
                let server_addr = app
                    .selected_server_addr
                    .clone()
                    .expect("guard ensures selected server addr");
                match timeout(
                    RECONNECT_ATTEMPT_TIMEOUT,
                    connect_session(&app.player_name, &client_token, &server_addr),
                ).await {
                    Ok(Ok(new_connection)) => {
                        app.begin_syncing();
                        connection = Some(new_connection);
                    }
                    Ok(Err(_)) | Err(_) => {
                        app.enter_reconnecting("attempting to reconnect".to_string());
                    }
                }
            }
            _ = pheromone_refresh.tick(), if connection.is_some() => {
                if let (Some(channel), Some(player), Some(connection)) = (app.pheromone_overlay, app.player(), connection.as_mut()) {
                    if let Some(hive_id) = player.hive_id {
                        crate::network::send_message(
                            &mut connection.writer,
                            ClientMessage::RequestPheromoneMap { hive_id, channel },
                        ).await?;
                    }
                }
            }
            maybe_message = recv_server_message(&mut connection), if connection.is_some() => {
                match maybe_message {
                    Some(message) => handle_server_message(&mut app, message),
                    None => {
                        connection = None;
                        app.enter_reconnecting("attempting to reconnect".to_string());
                    }
                }
            }
            Some(update) = discovery_rx.recv() => {
                match update {
                    DiscoveryUpdate::Upsert(server) => app.upsert_discovered_server(server),
                    DiscoveryUpdate::Remove { id } => app.remove_discovered_server(&id),
                    DiscoveryUpdate::Error(message) => app.set_error(message),
                }
            }
            maybe_event = tokio_stream_event(&mut events) => {
                if let Some(event) = maybe_event? {
                    let writer = connection.as_mut().map(|connection| &mut connection.writer);
                    if handle_event(event, &mut app, writer).await? {
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
