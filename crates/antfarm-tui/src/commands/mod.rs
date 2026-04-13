mod local;
mod server;

use crate::app::App;
use anyhow::Result;
use crossterm::event::KeyCode;
use serde_json::Value;

pub(crate) async fn handle_command_input(
    code: KeyCode,
    app: &mut App,
    writer: Option<&mut tokio::net::tcp::OwnedWriteHalf>,
) -> Result<bool> {
    let Some(input) = app.command_input.as_mut() else {
        return Ok(false);
    };

    match code {
        KeyCode::Esc => {
            app.command_input = None;
            app.command_feedback = None;
            app.command_history_index = None;
        }
        KeyCode::Backspace => {
            app.command_history_index = None;
            input.pop();
            if input.is_empty() {
                app.command_input = None;
                app.command_feedback = None;
            }
        }
        KeyCode::Enter => {
            let command = input.clone();
            app.command_input = None;
            app.command_feedback = None;
            app.command_history_index = None;
            submit_command(command, app, writer).await?;
        }
        KeyCode::Tab => {
            autocomplete_command(input);
            app.command_feedback = command_suggestion(input);
        }
        KeyCode::Up => history_up(app),
        KeyCode::Down => history_down(app),
        KeyCode::Char(ch) => {
            app.command_history_index = None;
            input.push(ch);
        }
        _ => {}
    }

    if let Some(input) = &app.command_input {
        app.command_feedback = command_suggestion(input);
    }

    Ok(false)
}

pub(crate) fn command_suggestion(input: &str) -> Option<String> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') {
        return None;
    }

    let suggestions = [
        "/help",
        "/cc set show_help_at_startup false",
        "/cc set show_help_at_startup true",
        "/cc set max_history 100",
        "/sc show_params",
        "/sc give all q 1",
        "/sc world_reset",
        "/sc world_reset 42",
        "/sc set soil.settle_frequency 0.01",
        "/sc set world.gen_params.soil.dirt_depth 8",
        "/sc set world.gen_params.ore.cluster_max 18",
        "/sc set world.gen_params.food.max_depth 50",
        "/sc set world.gen_params.stone_pockets.cluster_max 12",
        "/sc set world.snapshot_interval 5.0",
    ];

    let matches: Vec<_> = suggestions
        .into_iter()
        .filter(|candidate| candidate.starts_with(trimmed))
        .collect();

    if matches.is_empty() {
        None
    } else {
        Some(matches.join("   "))
    }
}

pub(crate) fn parse_config_value(raw: &str) -> Result<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return Ok(value);
    }

    if let Ok(number) = raw.parse::<f64>() {
        return Ok(Value::from(number));
    }

    match raw {
        "true" => Ok(Value::from(true)),
        "false" => Ok(Value::from(false)),
        "null" => Ok(Value::Null),
        _ => Ok(Value::from(raw)),
    }
}

async fn submit_command(
    command: String,
    app: &mut App,
    writer: Option<&mut tokio::net::tcp::OwnedWriteHalf>,
) -> Result<()> {
    let trimmed = command.trim();
    if !trimmed.is_empty() && push_command_history(app, trimmed) {
        crate::client_files::save_command_history(&app.player_name, &app.command_history, app.max_history)?;
    }

    if trimmed == "/help" {
        app.show_help = true;
        app.clear_status();
        return Ok(());
    }

    if trimmed.starts_with("/cc ") {
        return local::submit_local_command(trimmed, app).await;
    }

    if trimmed == "/sc show_params" {
        app.open_params();
        app.clear_status();
        return Ok(());
    }

    server::submit_server_command(trimmed, app, writer).await
}

fn autocomplete_command(input: &mut String) {
    let trimmed = input.trim_start();
    let suggestions = [
        "/help",
        "/cc set show_help_at_startup false",
        "/cc set show_help_at_startup true",
        "/cc set max_history 100",
        "/sc show_params",
        "/sc give all q 1",
        "/sc world_reset",
        "/sc world_reset 42",
        "/sc set soil.settle_frequency 0.01",
        "/sc set world.gen_params.soil.dirt_depth 8",
        "/sc set world.gen_params.ore.cluster_max 18",
        "/sc set world.gen_params.food.max_depth 50",
        "/sc set world.gen_params.stone_pockets.cluster_max 12",
        "/sc set world.snapshot_interval 5.0",
    ];

    let matches: Vec<_> = suggestions
        .into_iter()
        .filter(|candidate| candidate.starts_with(trimmed))
        .collect();

    if matches.len() == 1 {
        *input = matches[0].to_string();
    }
}

fn push_command_history(app: &mut App, command: &str) -> bool {
    if app.command_history.last().is_some_and(|last| last == command) {
        return false;
    }
    app.command_history.push(command.to_string());
    if app.command_history.len() > app.max_history {
        let extra = app.command_history.len() - app.max_history;
        app.command_history.drain(0..extra);
    }
    true
}

fn history_up(app: &mut App) {
    if app.command_history.is_empty() {
        return;
    }

    let next_index = match app.command_history_index {
        None => app.command_history.len().saturating_sub(1),
        Some(0) => 0,
        Some(index) => index.saturating_sub(1),
    };
    app.command_history_index = Some(next_index);
    if let Some(command) = app.command_history.get(next_index) {
        app.command_input = Some(command.clone());
    }
}

fn history_down(app: &mut App) {
    let Some(index) = app.command_history_index else {
        return;
    };

    if index + 1 >= app.command_history.len() {
        app.command_history_index = None;
        app.command_input = Some("/".to_string());
        return;
    }

    let next_index = index + 1;
    app.command_history_index = Some(next_index);
    if let Some(command) = app.command_history.get(next_index) {
        app.command_input = Some(command.clone());
    }
}
