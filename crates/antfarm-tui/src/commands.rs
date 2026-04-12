use crate::{
    app::App,
    client_files::{
        load_or_create_client_config, save_client_config, save_command_history,
    },
    network::send_message,
};
use anyhow::Result;
use antfarm_core::ClientMessage;
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

pub(crate) async fn submit_command(
    command: String,
    app: &mut App,
    writer: Option<&mut tokio::net::tcp::OwnedWriteHalf>,
) -> Result<()> {
    let trimmed = command.trim();
    if !trimmed.is_empty() && push_command_history(app, trimmed) {
        save_command_history(&app.player_name, &app.command_history, app.max_history)?;
    }
    let mut parts = trimmed.splitn(4, ' ');
    let head = parts.next().unwrap_or_default();
    let verb = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let raw_value = parts.next().unwrap_or_default();

    if trimmed == "/help" {
        app.show_help = true;
        app.clear_status();
        return Ok(());
    }

    if head == "/cc" && verb == "set" && !path.is_empty() && !raw_value.is_empty() {
        let mut client_config = load_or_create_client_config(&app.player_name)?;
        match path {
            "show_help_at_startup" => {
                let show_help_at_startup = match raw_value {
                    "true" => true,
                    "false" => false,
                    _ => {
                        app.set_error("expected: /cc set show_help_at_startup true|false");
                        return Ok(());
                    }
                };
                client_config.show_help_at_startup = show_help_at_startup;
                save_client_config(&app.player_name, &client_config)?;
                app.set_info(format!(
                    "client config updated: show_help_at_startup={show_help_at_startup}"
                ));
                return Ok(());
            }
            "max_history" => {
                let max_history = raw_value
                    .parse::<usize>()
                    .map_err(|_| anyhow::anyhow!("max_history must be a positive integer"))?;
                if max_history == 0 {
                    app.set_error("max_history must be at least 1");
                    return Ok(());
                }
                client_config.max_history = max_history;
                save_client_config(&app.player_name, &client_config)?;
                app.max_history = max_history;
                if app.command_history.len() > app.max_history {
                    let extra = app.command_history.len() - app.max_history;
                    app.command_history.drain(0..extra);
                }
                save_command_history(&app.player_name, &app.command_history, app.max_history)?;
                app.set_info(format!("client config updated: max_history={max_history}"));
                return Ok(());
            }
            _ => {
                app.set_error(
                    "expected: /cc set show_help_at_startup true|false or /cc set max_history <n>",
                );
                return Ok(());
            }
        }
    }

    if trimmed == "/sc show_params" {
        app.open_params();
        app.clear_status();
        return Ok(());
    }

    let Some(writer) = writer else {
        app.set_error("server unavailable while reconnecting");
        return Ok(());
    };

    if head == "/sc" && verb == "world_reset" {
        let seed = if path.is_empty() {
            None
        } else {
            Some(
                path.parse::<u64>()
                    .map_err(|_| anyhow::anyhow!("world_reset seed must be an unsigned integer"))?,
            )
        };
        send_message(writer, ClientMessage::WorldReset { seed }).await?;
        app.clear_status();
        return Ok(());
    }

    if head != "/sc" || verb != "set" || path.is_empty() || raw_value.is_empty() {
        app.set_error("expected: /help, /cc set show_help_at_startup true|false, /cc set max_history <n>, /sc show_params, /sc world_reset [seed], or /sc set <path> <value>");
        return Ok(());
    }

    let value = parse_config_value(raw_value)?;
    send_message(
        writer,
        ClientMessage::ConfigSet {
            path: path.to_string(),
            value,
        },
    )
    .await?;
    app.clear_status();
    Ok(())
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

fn autocomplete_command(input: &mut String) {
    let trimmed = input.trim_start();
    let suggestions = [
        "/help",
        "/cc set show_help_at_startup false",
        "/cc set show_help_at_startup true",
        "/cc set max_history 100",
        "/sc show_params",
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
