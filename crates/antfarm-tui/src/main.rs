mod app;
mod art;
mod client_files;
mod commands;
mod input;
mod modals;
mod network;
mod render;

use crate::{
    app::{App, SyncState, handle_server_message},
    client_files::{load_command_history, load_or_create_client_config},
    input::handle_event,
    network::{
        Connection, RECONNECT_ATTEMPT_TIMEOUT, connect_session, offline_snapshot,
        recv_server_message, tokio_stream_event,
    },
    render::draw,
};
use anyhow::Result;
use crossterm::{
    event::EventStream,
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::DefaultTerminal;
use std::{env, io, time::Duration};
use tokio::time::{self, timeout};

#[tokio::main]
async fn main() -> Result<()> {
    let name = env::args()
        .nth(1)
        .unwrap_or_else(|| "worker-ant".to_string());
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let terminal = ratatui::init();

    let result = run_app(terminal, name).await;

    ratatui::restore();
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    result
}

async fn run_app(mut terminal: DefaultTerminal, player_name: String) -> Result<()> {
    let client_config = load_or_create_client_config(&player_name)?;
    let client_token = client_config.token.clone();
    let mut app = App::new(
        player_name.clone(),
        0,
        offline_snapshot(),
        client_config.show_help_at_startup,
        client_config.max_history,
    );
    app.command_history = load_command_history(&player_name, app.max_history)?;
    app.sync_state = SyncState::Connecting;
    let mut events = EventStream::new();
    let mut redraw = time::interval(Duration::from_millis(33));
    let mut reconnect = time::interval(Duration::from_millis(1000));
    reconnect.tick().await;
    let mut connection: Option<Connection> = None;

    loop {
        terminal.draw(|frame| draw(frame, &app))?;

        tokio::select! {
            _ = redraw.tick() => app.tick_animation(),
            _ = reconnect.tick(), if connection.is_none() => {
                match timeout(RECONNECT_ATTEMPT_TIMEOUT, connect_session(&app.player_name, &client_token)).await {
                    Ok(Ok(new_connection)) => {
                        app.begin_syncing();
                        connection = Some(new_connection);
                    }
                    Ok(Err(_)) | Err(_) => {
                        app.enter_reconnecting("attempting to reconnect".to_string());
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
