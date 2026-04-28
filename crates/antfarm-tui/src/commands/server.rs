use crate::{app::App, commands::parse_config_value, network::send_message};
use anyhow::Result;
use antfarm_core::ClientMessage;

pub(super) async fn submit_server_command(
    trimmed: &str,
    app: &mut App,
    writer: Option<&mut tokio::net::tcp::OwnedWriteHalf>,
) -> Result<()> {
    let mut parts = trimmed.splitn(4, ' ');
    let head = parts.next().unwrap_or_default();
    let verb = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let raw_value = parts.next().unwrap_or_default();

    let Some(writer) = writer else {
        app.set_error("server unavailable while reconnecting");
        return Ok(());
    };

    if let Some(raw_label) = trimmed.strip_prefix("/sc save_gamestate ") {
        let label = trim_wrapped_quotes(raw_label.trim());
        if label.is_empty() {
            app.set_error("expected: /sc save_gamestate \"label\"");
            return Ok(());
        }
        send_message(
            writer,
            ClientMessage::SaveGameState {
                label: label.to_string(),
            },
        )
        .await?;
        app.clear_status();
        return Ok(());
    }

    if trimmed == "/sc list_gamestates" {
        send_message(writer, ClientMessage::ListGameStates).await?;
        app.clear_status();
        return Ok(());
    }

    if let Some(raw_selector) = trimmed.strip_prefix("/sc delete_gamestate ") {
        let selector = trim_wrapped_quotes(raw_selector.trim());
        if selector.is_empty() {
            app.set_error("expected: /sc delete_gamestate <id|label>");
            return Ok(());
        }
        send_message(
            writer,
            ClientMessage::DeleteGameState {
                selector: selector.to_string(),
            },
        )
        .await?;
        app.clear_status();
        return Ok(());
    }

    if trimmed == "/sc delete_all_gamestates" {
        send_message(writer, ClientMessage::DeleteAllGameStates).await?;
        app.clear_status();
        return Ok(());
    }

    if let Some(raw_selector) = trimmed.strip_prefix("/sc load_gamestate ") {
        let selector = trim_wrapped_quotes(raw_selector.trim());
        if selector.is_empty() {
            app.set_error("expected: /sc load_gamestate <id|label>");
            return Ok(());
        }
        send_message(
            writer,
            ClientMessage::LoadGameState {
                selector: selector.to_string(),
            },
        )
        .await?;
        app.clear_status();
        return Ok(());
    }

    if head == "/sc" && verb == "game" {
        match path {
            "pause" => {
                send_message(writer, ClientMessage::SetSimulationPaused { paused: true }).await?;
                app.clear_status();
                return Ok(());
            }
            "unpause" => {
                send_message(writer, ClientMessage::SetSimulationPaused { paused: false }).await?;
                app.clear_status();
                return Ok(());
            }
            _ => {
                app.set_error("expected: /sc game pause|unpause");
                return Ok(());
            }
        }
    }

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

    if head == "/sc" && verb == "give" {
        let mut args = trimmed.split_whitespace();
        let _ = args.next();
        let _ = args.next();
        let target = args.next().unwrap_or_default();
        let resource = args.next().unwrap_or_default();
        let amount_raw = args.next().unwrap_or_default();
        if target.is_empty() || resource.is_empty() || amount_raw.is_empty() {
            app.set_error("expected: /sc give <player-name|@a|@e> <resource> <amount>");
            return Ok(());
        }
        let amount = amount_raw
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("give amount must be an unsigned integer"))?;
        send_message(
            writer,
            ClientMessage::Give {
                target: target.to_string(),
                resource: resource.to_string(),
                amount,
            },
        )
        .await?;
        app.clear_status();
        return Ok(());
    }

    if head == "/sc" && verb == "feed_queen" {
        let amount_raw = path;
        if amount_raw.is_empty() {
            app.set_error("expected: /sc feed_queen <amount>");
            return Ok(());
        }
        let amount = amount_raw
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("feed_queen amount must be an unsigned integer"))?;
        send_message(writer, ClientMessage::FeedQueens { amount }).await?;
        app.clear_status();
        return Ok(());
    }

    if head == "/sc" && verb == "set" && path == "queen.eggs" {
        let eggs_raw = raw_value;
        let eggs = eggs_raw
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("queen.eggs must be an unsigned integer"))?;
        send_message(writer, ClientMessage::SetQueenEggs { eggs }).await?;
        app.clear_status();
        return Ok(());
    }

    if head == "/sc" && verb == "kill" {
        let selector = trimmed
            .strip_prefix("/sc kill ")
            .unwrap_or_default()
            .trim();
        if selector.is_empty() {
            app.set_error("expected: /sc kill <selector>");
            return Ok(());
        }
        send_message(
            writer,
            ClientMessage::Kill {
                selector: selector.to_string(),
            },
        )
        .await?;
        app.clear_status();
        return Ok(());
    }

    if head == "/sc" && verb == "dig" {
        let mut args = trimmed.split_whitespace();
        let _ = args.next();
        let _ = args.next();
        let width_raw = args.next().unwrap_or_default();
        let height_raw = args.next().unwrap_or_default();
        if width_raw.is_empty() || height_raw.is_empty() {
            app.set_error("expected: /sc dig <width> <height>");
            return Ok(());
        }
        let width = width_raw
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("dig width must be an unsigned integer"))?;
        let height = height_raw
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("dig height must be an unsigned integer"))?;
        send_message(writer, ClientMessage::DigArea { width, height }).await?;
        app.clear_status();
        return Ok(());
    }

    if head == "/sc" && verb == "put" {
        let mut args = trimmed.split_whitespace();
        let _ = args.next();
        let _ = args.next();
        let resource = args.next().unwrap_or_default();
        let width_raw = args.next().unwrap_or_default();
        let height_raw = args.next().unwrap_or_default();
        if resource.is_empty() || width_raw.is_empty() || height_raw.is_empty() {
            app.set_error("expected: /sc put <resource> <width> <height>");
            return Ok(());
        }
        let width = width_raw
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("put width must be an unsigned integer"))?;
        let height = height_raw
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("put height must be an unsigned integer"))?;
        send_message(
            writer,
            ClientMessage::PutArea {
                resource: resource.to_string(),
                width,
                height,
            },
        )
        .await?;
        app.clear_status();
        return Ok(());
    }

    if head == "/sc" && verb == "debug.npc" {
        match path {
            "start" => {
                send_message(writer, ClientMessage::DebugNpcStart).await?;
                app.clear_status();
                return Ok(());
            }
            "stop" => {
                send_message(writer, ClientMessage::DebugNpcStop).await?;
                app.clear_status();
                return Ok(());
            }
            "status" => {
                send_message(writer, ClientMessage::DebugNpcStatus).await?;
                app.clear_status();
                return Ok(());
            }
            _ => {
                app.set_error("expected: /sc debug.npc start|stop|status");
                return Ok(());
            }
        }
    }

    if head != "/sc" || verb != "set" || path.is_empty() || raw_value.is_empty() {
        app.set_error("expected: /help, /cc set show_help_at_startup true|false, /cc set max_history <n>, /sc show_params, /sc world_reset [seed], /sc save_gamestate \"label\", /sc list_gamestates, /sc delete_gamestate <id|label>, /sc delete_all_gamestates, /sc load_gamestate <id|label>, /sc game pause|unpause, /sc give <player-name|@a|@e> <resource> <amount>, /sc feed_queen <amount>, /sc set queen.eggs <n>, /sc kill <selector>, /sc dig <width> <height>, /sc put <resource> <width> <height>, /sc debug.npc start|stop|status, or /sc set <path> <value>");
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

fn trim_wrapped_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}
