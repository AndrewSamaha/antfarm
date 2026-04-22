use anyhow::{Context, Result};
use antfarm_core::NpcDebugEvent;
use rusqlite::{Connection, params};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::logging::emit_log;

#[derive(Clone)]
pub(crate) struct NpcDebugSession {
    pub(crate) path: PathBuf,
    tx: mpsc::Sender<NpcDebugMessage>,
}

pub(crate) enum NpcDebugMessage {
    Event(NpcDebugEvent),
    Shutdown,
}

pub(crate) fn start_npc_debug_session(dir: &Path) -> Result<NpcDebugSession> {
    fs::create_dir_all(dir)?;
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before unix epoch")?
        .as_millis();
    let path = dir.join(format!("debug.npc.{ts_ms}.sqlite"));
    start_npc_debug_session_at_path(&path)
}

pub(crate) fn start_npc_debug_session_at_path(path: &Path) -> Result<NpcDebugSession> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let (tx, rx) = mpsc::channel::<NpcDebugMessage>();
    let path = path.to_path_buf();
    let thread_path = path.clone();
    thread::spawn(move || {
        if let Err(error) = npc_debug_worker_loop(thread_path, rx) {
            eprintln!("npc debug worker failed: {error}");
        }
    });
    emit_log(
        "npc_debug_started",
        json!({ "path": path.display().to_string() }),
    );
    Ok(NpcDebugSession { path, tx })
}

pub(crate) fn send_npc_debug_events(session: &NpcDebugSession, events: Vec<NpcDebugEvent>) {
    for event in events {
        let _ = session.tx.send(NpcDebugMessage::Event(event));
    }
}

pub(crate) fn stop_npc_debug_session(session: NpcDebugSession) {
    let _ = session.tx.send(NpcDebugMessage::Shutdown);
    emit_log(
        "npc_debug_stopped",
        json!({ "path": session.path.display().to_string() }),
    );
}

fn npc_debug_worker_loop(path: PathBuf, rx: mpsc::Receiver<NpcDebugMessage>) -> Result<()> {
    let connection = Connection::open(&path)
        .with_context(|| format!("open npc debug sqlite database at {}", path.display()))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "busy_timeout", 5_000)?;
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS npc_debug_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts_ms INTEGER NOT NULL,
            tick INTEGER NOT NULL,
            npc_id INTEGER NOT NULL,
            hive_id INTEGER,
            event_type TEXT NOT NULL,
            x INTEGER NOT NULL,
            y INTEGER NOT NULL,
            details_json TEXT NOT NULL
        );
        ",
    )?;

    for message in rx {
        match message {
            NpcDebugMessage::Event(event) => {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .context("system clock before unix epoch")?
                    .as_millis() as i64;
                let details_json = serde_json::to_string(&event.details)?;
                connection.execute(
                    "
                    INSERT INTO npc_debug_events
                        (ts_ms, tick, npc_id, hive_id, event_type, x, y, details_json)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    ",
                    params![
                        now_ms,
                        event.tick as i64,
                        i64::from(event.npc_id),
                        event.hive_id.map(i64::from),
                        event.event_type,
                        i64::from(event.pos.x),
                        i64::from(event.pos.y),
                        details_json,
                    ],
                )?;
            }
            NpcDebugMessage::Shutdown => break,
        }
    }

    Ok(())
}
