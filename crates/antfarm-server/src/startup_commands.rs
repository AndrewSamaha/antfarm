use anyhow::{Result, anyhow};
use serde_json::Value;
use std::path::Path;

use crate::{
    debug_npc::{start_npc_debug_session, stop_npc_debug_session},
    logging::{emit_log, world_log_fields},
    persistence::{
        delete_all_named_gamestates, delete_named_gamestate,
        load_named_gamestate, save_named_gamestate,
    },
    server_state::ServerState,
};
use serde_json::json;

pub(crate) async fn run_startup_sc_commands(
    state: &ServerState,
    snapshot_db_path: &Path,
    npc_debug_dir: &Path,
    commands: &[String],
) -> Result<()> {
    for command in commands {
        run_startup_sc_command(state, snapshot_db_path, npc_debug_dir, command).await?;
    }
    Ok(())
}

async fn run_startup_sc_command(
    state: &ServerState,
    snapshot_db_path: &Path,
    npc_debug_dir: &Path,
    command: &str,
) -> Result<()> {
    let trimmed = command.trim();
    if let Some(raw_label) = trimmed.strip_prefix("/sc save_gamestate ") {
        let label = trim_wrapped_quotes(raw_label.trim());
        if label.is_empty() {
            return Err(anyhow!("save_gamestate label cannot be empty"));
        }
        let snapshot = {
            let game = state.game.lock().await;
            game.snapshot()
        };
        let save_id = save_named_gamestate(snapshot_db_path, label, &snapshot)?;
        let mut game = state.game.lock().await;
        game.push_server_event(format!(
            "Saved game state #{save_id}: {label} (tick {})",
            snapshot.tick
        ));
        let _ = game.take_patch();
        return Ok(());
    }

    if let Some(raw_selector) = trimmed.strip_prefix("/sc load_gamestate ") {
        let selector = trim_wrapped_quotes(raw_selector.trim());
        if selector.is_empty() {
            return Err(anyhow!("load_gamestate requires an id or exact label"));
        }
        let Some(snapshot) = load_named_gamestate(snapshot_db_path, selector)? else {
            return Err(anyhow!("no saved game state matched: {selector}"));
        };
        let mut game = state.game.lock().await;
        *game = antfarm_core::GameState::from_snapshot(snapshot);
        game.push_server_event(format!("Loaded game state: {selector}"));
        let _ = game.take_patch();
        return Ok(());
    }

    if let Some(raw_selector) = trimmed.strip_prefix("/sc delete_gamestate ") {
        let selector = trim_wrapped_quotes(raw_selector.trim());
        if selector.is_empty() {
            return Err(anyhow!("delete_gamestate requires an id or exact label"));
        }
        let deleted = delete_named_gamestate(snapshot_db_path, selector)?;
        if deleted == 0 {
            return Err(anyhow!("no saved game state matched: {selector}"));
        }
        let mut game = state.game.lock().await;
        game.push_server_event(format!(
            "Deleted {deleted} saved game state(s): {selector}"
        ));
        let _ = game.take_patch();
        return Ok(());
    }

    if trimmed == "/sc delete_all_gamestates" {
        let deleted = delete_all_named_gamestates(snapshot_db_path)?;
        let mut game = state.game.lock().await;
        game.push_server_event(format!("Deleted {deleted} saved game state(s)"));
        let _ = game.take_patch();
        return Ok(());
    }

    if let Some(raw) = trimmed.strip_prefix("/sc set ") {
        let mut parts = raw.splitn(2, ' ');
        let path = parts.next().unwrap_or_default().trim();
        let raw_value = parts.next().unwrap_or_default().trim();
        if path.is_empty() || raw_value.is_empty() {
            return Err(anyhow!("expected: /sc set <path> <value>"));
        }
        let value = parse_config_value(raw_value)?;
        let snapshot = {
            let mut game = state.game.lock().await;
            game.set_config_value(path, value.clone()).map_err(anyhow::Error::msg)?;
            let snapshot = game.snapshot();
            let _ = game.take_patch();
            snapshot
        };
        let _ = state.persistence_tx.send(crate::server_state::PersistMessage::Save(snapshot));
        emit_log("startup_sc_set", json!({ "path": path, "value": value }));
        return Ok(());
    }

    if trimmed == "/sc game pause" || trimmed == "/sc game unpause" {
        let paused = trimmed.ends_with("pause") && !trimmed.ends_with("unpause");
        let snapshot = {
            let mut game = state.game.lock().await;
            game.set_simulation_paused(paused);
            let snapshot = game.snapshot();
            let _ = game.take_patch();
            snapshot
        };
        let _ = state.persistence_tx.send(crate::server_state::PersistMessage::Save(snapshot));
        return Ok(());
    }

    if let Some(raw) = trimmed.strip_prefix("/sc world_reset") {
        let raw = raw.trim();
        let seed = if raw.is_empty() {
            None
        } else {
            Some(raw.parse::<u64>().map_err(|_| anyhow!("world_reset seed must be an unsigned integer"))?)
        };
        let snapshot = {
            let mut game = state.game.lock().await;
            game.world_reset(seed);
            let snapshot = game.snapshot();
            let _ = game.take_patch();
            emit_log(
                "world_reset",
                json!({
                    "seed_override": seed,
                    "world": world_log_fields(&game),
                }),
            );
            snapshot
        };
        let _ = state.persistence_tx.send(crate::server_state::PersistMessage::Save(snapshot));
        let _ = state
            .persistence_tx
            .send(crate::server_state::PersistMessage::ClearPlayerProfiles);
        return Ok(());
    }

    if let Some(raw) = trimmed.strip_prefix("/sc give ") {
        let mut args = raw.split_whitespace();
        let target = args.next().unwrap_or_default();
        let resource = args.next().unwrap_or_default();
        let amount_raw = args.next().unwrap_or_default();
        if target.is_empty() || resource.is_empty() || amount_raw.is_empty() {
            return Err(anyhow!("expected: /sc give <player-name|@a|@e> <resource> <amount>"));
        }
        let amount = amount_raw
            .parse::<u16>()
            .map_err(|_| anyhow!("give amount must be an unsigned integer"))?;
        let snapshot = {
            let mut game = state.game.lock().await;
            game.give_resource(target, resource, amount)
                .map_err(anyhow::Error::msg)?;
            let snapshot = game.snapshot();
            let _ = game.take_patch();
            snapshot
        };
        let _ = state.persistence_tx.send(crate::server_state::PersistMessage::Save(snapshot));
        return Ok(());
    }

    if let Some(raw) = trimmed.strip_prefix("/sc feed_queen ") {
        let amount = raw
            .trim()
            .parse::<u16>()
            .map_err(|_| anyhow!("feed_queen amount must be an unsigned integer"))?;
        let snapshot = {
            let mut game = state.game.lock().await;
            game.feed_queens(amount).map_err(anyhow::Error::msg)?;
            let snapshot = game.snapshot();
            let _ = game.take_patch();
            snapshot
        };
        let _ = state
            .persistence_tx
            .send(crate::server_state::PersistMessage::Save(snapshot));
        return Ok(());
    }

    if let Some(raw) = trimmed.strip_prefix("/sc kill ") {
        let selector = raw.trim();
        if selector.is_empty() {
            return Err(anyhow!("expected: /sc kill <selector>"));
        }
        let snapshot = {
            let mut game = state.game.lock().await;
            game.kill_by_selector(selector).map_err(anyhow::Error::msg)?;
            let snapshot = game.snapshot();
            let _ = game.take_patch();
            snapshot
        };
        let _ = state
            .persistence_tx
            .send(crate::server_state::PersistMessage::Save(snapshot));
        return Ok(());
    }

    if trimmed == "/sc debug.npc start" {
        if state.npc_debug.lock().await.is_some() {
            return Err(anyhow!("NPC debug is already active"));
        }
        let session = start_npc_debug_session(npc_debug_dir)?;
        {
            let mut game = state.game.lock().await;
            game.set_npc_debug_enabled(true);
            game.push_server_event(format!("NPC debug started: {}", session.path.display()));
            let _ = game.take_patch();
        }
        *state.npc_debug.lock().await = Some(session);
        return Ok(());
    }

    if trimmed == "/sc debug.npc stop" {
        let session = state.npc_debug.lock().await.take();
        let Some(session) = session else {
            return Err(anyhow!("NPC debug is not active"));
        };
        {
            let mut game = state.game.lock().await;
            game.set_npc_debug_enabled(false);
            game.take_npc_debug_events();
            game.push_server_event(format!("NPC debug stopped: {}", session.path.display()));
            let _ = game.take_patch();
        }
        stop_npc_debug_session(session);
        return Ok(());
    }

    if trimmed == "/sc debug.npc status" {
        let active = state.npc_debug.lock().await.as_ref().map(|s| s.path.clone());
        let mut game = state.game.lock().await;
        match active {
            Some(path) => game.push_server_event(format!("NPC debug active: {}", path.display())),
            None => game.push_server_event("NPC debug inactive".to_string()),
        }
        let _ = game.take_patch();
        return Ok(());
    }

    if trimmed == "/sc list_gamestates" {
        let states = crate::persistence::list_named_gamestates(snapshot_db_path)?;
        let summary = if states.is_empty() {
            "Saved game states: none".to_string()
        } else {
            let entries = states
                .iter()
                .take(5)
                .map(|state| {
                    format!(
                        "#{} '{}' @ tick {} ({})",
                        state.id, state.label, state.tick, state.saved_at
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if states.len() > 5 { ", ..." } else { "" };
            format!("Saved game states: {entries}{suffix}")
        };
        let mut game = state.game.lock().await;
        game.push_server_event(summary);
        let _ = game.take_patch();
        return Ok(());
    }

    if trimmed.starts_with("/sc dig ") || trimmed.starts_with("/sc put ") {
        return Err(anyhow!("startup commands do not support player-relative dig/put without a live player context"));
    }

    Err(anyhow!("unsupported startup command: {trimmed}"))
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

fn parse_config_value(raw: &str) -> Result<Value> {
    if raw.eq_ignore_ascii_case("true") {
        return Ok(Value::Bool(true));
    }
    if raw.eq_ignore_ascii_case("false") {
        return Ok(Value::Bool(false));
    }
    if raw.eq_ignore_ascii_case("null") {
        return Ok(Value::Null);
    }
    if let Ok(value) = raw.parse::<i64>() {
        return Ok(Value::from(value));
    }
    if let Ok(value) = raw.parse::<f64>() {
        return Ok(Value::from(value));
    }
    if ((raw.starts_with('{') && raw.ends_with('}')) || (raw.starts_with('[') && raw.ends_with(']')))
        && let Ok(value) = serde_json::from_str::<Value>(raw)
    {
        return Ok(value);
    }
    Ok(Value::String(trim_wrapped_quotes(raw).to_string()))
}
