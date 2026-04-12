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
