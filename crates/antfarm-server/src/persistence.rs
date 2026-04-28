use anyhow::{Context, Result, anyhow};
use antfarm_core::{GameState, Player, Snapshot, default_server_config, merge_config};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

use crate::{
    logging::emit_log,
    server_state::PersistMessage,
};

pub(crate) const SNAPSHOT_RETENTION: i64 = 10;

#[derive(Debug, Clone)]
pub(crate) struct NamedGameStateInfo {
    pub(crate) id: i64,
    pub(crate) label: String,
    pub(crate) saved_at: String,
    pub(crate) tick: u64,
}

pub(crate) fn load_startup_game(
    path: &Path,
    start_paused: bool,
    load_gamestate: Option<&str>,
    config_override: &Value,
) -> Result<(GameState, bool)> {
    if let Some(selector) = load_gamestate {
        let snapshot = load_named_gamestate(path, selector)?
            .ok_or_else(|| anyhow!("named gamestate not found: {selector}"))?;
        let mut game = GameState::from_snapshot(snapshot);
        game.apply_config_override(config_override.clone());
        if start_paused {
            game.set_simulation_paused(true);
        }
        emit_log(
            "loading_named_gamestate",
            json!({
                "selector": selector,
                "simulation_paused": game.simulation_paused,
            }),
        );
        return Ok((game, true));
    }

    if let Some(snapshot) = load_latest_snapshot(path)? {
        let mut game = GameState::from_snapshot(snapshot);
        game.apply_config_override(config_override.clone());
        if start_paused {
            game.set_simulation_paused(true);
        }
        Ok((game, true))
    } else {
        let config = merge_config(default_server_config(), config_override.clone());
        emit_log(
            "generating_world",
            json!({
                "source": "startup",
                "world": {
                    "seed": config.pointer("/world/seed").and_then(Value::as_u64),
                    "max_depth": config.pointer("/world/max_depth").and_then(Value::as_i64),
                    "gen_params": config.pointer("/world/gen_params").cloned(),
                }
            }),
        );
        let mut game = GameState::from_config(config);
        if start_paused {
            game.set_simulation_paused(true);
        }
        Ok((game, false))
    }
}

pub(crate) fn spawn_persistence_worker(path: PathBuf) -> Result<mpsc::Sender<PersistMessage>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let (tx, rx) = mpsc::channel::<PersistMessage>();
    thread::spawn(move || {
        if let Err(error) = persistence_worker_loop(path, rx) {
            eprintln!("persistence worker failed: {error}");
        }
    });
    Ok(tx)
}

pub(crate) fn load_player_profile(path: &Path, token: &str) -> Result<Option<Player>> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = open_db(path)?;
    let mut statement =
        connection.prepare("SELECT player_json FROM player_profiles WHERE token = ?1 LIMIT 1")?;
    let mut rows = statement.query(params![token])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let player_json: String = row.get(0)?;
    Ok(Some(serde_json::from_str::<Player>(&player_json)?))
}

pub(crate) fn save_named_gamestate(path: &Path, label: &str, snapshot: &Snapshot) -> Result<i64> {
    let connection = open_db(path)?;
    let state_json = serde_json::to_string(snapshot)?;
    connection.execute(
        "
        INSERT INTO named_gamestates (label, tick, state_json)
        VALUES (?1, ?2, ?3)
        ",
        params![label, snapshot.tick as i64, state_json],
    )?;
    Ok(connection.last_insert_rowid())
}

pub(crate) fn list_named_gamestates(path: &Path) -> Result<Vec<NamedGameStateInfo>> {
    let connection = open_db(path)?;
    let mut statement = connection.prepare(
        "
        SELECT id, label, saved_at, tick
        FROM named_gamestates
        ORDER BY id DESC
        ",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(NamedGameStateInfo {
            id: row.get(0)?,
            label: row.get(1)?,
            saved_at: row.get(2)?,
            tick: row.get::<_, i64>(3)? as u64,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub(crate) fn load_named_gamestate(path: &Path, selector: &str) -> Result<Option<Snapshot>> {
    let connection = open_db(path)?;
    let state_json: Option<String> = if let Ok(id) = selector.parse::<i64>() {
        connection
            .query_row(
                "SELECT state_json FROM named_gamestates WHERE id = ?1 LIMIT 1",
                params![id],
                |row| row.get(0),
            )
            .optional()?
    } else {
        connection
            .query_row(
                "
                SELECT state_json
                FROM named_gamestates
                WHERE label = ?1
                ORDER BY id DESC
                LIMIT 1
                ",
                params![selector],
                |row| row.get(0),
            )
            .optional()?
    };
    state_json
        .map(|state_json| serde_json::from_str::<Snapshot>(&state_json))
        .transpose()
        .map_err(Into::into)
}

pub(crate) fn reset_world_state_preserve_gamestates(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = open_db(path)?;
    connection.execute("DELETE FROM world_snapshots", [])?;
    connection.execute("DELETE FROM player_profiles", [])?;
    Ok(())
}

pub(crate) fn delete_named_gamestate(path: &Path, selector: &str) -> Result<usize> {
    let connection = open_db(path)?;
    let deleted = if let Ok(id) = selector.parse::<i64>() {
        connection.execute(
            "DELETE FROM named_gamestates WHERE id = ?1",
            params![id],
        )?
    } else {
        connection.execute(
            "
            DELETE FROM named_gamestates
            WHERE id = (
                SELECT id
                FROM named_gamestates
                WHERE label = ?1
                ORDER BY id DESC
                LIMIT 1
            )
            ",
            params![selector],
        )?
    };
    Ok(deleted)
}

pub(crate) fn delete_all_named_gamestates(path: &Path) -> Result<usize> {
    let connection = open_db(path)?;
    Ok(connection.execute("DELETE FROM named_gamestates", [])?)
}

fn persistence_worker_loop(path: PathBuf, rx: mpsc::Receiver<PersistMessage>) -> Result<()> {
    let connection = open_db(&path)?;
    for message in rx {
        match message {
            PersistMessage::Save(snapshot) => {
                save_snapshot(&connection, &snapshot)?;
                prune_snapshots(&connection)?;
                emit_log(
                    "snapshot_saved",
                    json!({ "tick": snapshot.tick, "retention": SNAPSHOT_RETENTION }),
                );
            }
            PersistMessage::UpsertPlayerProfile { token, player } => {
                save_player_profile(&connection, &token, &player)?;
            }
            PersistMessage::ClearPlayerProfiles => {
                clear_player_profiles(&connection)?;
            }
        }
    }
    Ok(())
}

fn open_db(path: &Path) -> Result<Connection> {
    let connection = Connection::open(path)
        .with_context(|| format!("open sqlite database at {}", path.display()))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "busy_timeout", 5_000)?;
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS world_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tick INTEGER NOT NULL,
            saved_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            state_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS player_profiles (
            token TEXT PRIMARY KEY,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            player_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS named_gamestates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT NOT NULL,
            tick INTEGER NOT NULL,
            saved_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            state_json TEXT NOT NULL
        );
        ",
    )?;
    Ok(connection)
}

fn load_latest_snapshot(path: &Path) -> Result<Option<Snapshot>> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = open_db(path)?;
    let mut statement =
        connection.prepare("SELECT state_json FROM world_snapshots ORDER BY id DESC LIMIT 1")?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let state_json: String = row.get(0)?;
    Ok(Some(serde_json::from_str::<Snapshot>(&state_json)?))
}

fn save_snapshot(connection: &Connection, snapshot: &Snapshot) -> Result<()> {
    let state_json = serde_json::to_string(snapshot)?;
    connection.execute(
        "INSERT INTO world_snapshots (tick, state_json) VALUES (?1, ?2)",
        params![snapshot.tick as i64, state_json],
    )?;
    Ok(())
}

fn save_player_profile(connection: &Connection, token: &str, player: &Player) -> Result<()> {
    let player_json = serde_json::to_string(player)?;
    connection.execute(
        "
        INSERT INTO player_profiles (token, player_json, updated_at)
        VALUES (?1, ?2, CURRENT_TIMESTAMP)
        ON CONFLICT(token) DO UPDATE SET
            player_json = excluded.player_json,
            updated_at = CURRENT_TIMESTAMP
        ",
        params![token, player_json],
    )?;
    Ok(())
}

fn clear_player_profiles(connection: &Connection) -> Result<()> {
    connection.execute("DELETE FROM player_profiles", [])?;
    Ok(())
}

fn prune_snapshots(connection: &Connection) -> Result<()> {
    connection.execute(
        "
        DELETE FROM world_snapshots
        WHERE id NOT IN (
            SELECT id FROM world_snapshots ORDER BY id DESC LIMIT ?1
        )
        ",
        params![SNAPSHOT_RETENTION],
    )?;
    Ok(())
}
