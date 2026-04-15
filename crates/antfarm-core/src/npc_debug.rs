use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::Position;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpcDebugEvent {
    pub tick: u64,
    pub npc_id: u16,
    pub hive_id: Option<u16>,
    pub event_type: String,
    pub pos: Position,
    pub details: Value,
}
