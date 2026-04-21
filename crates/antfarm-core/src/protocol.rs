use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    pheromones::{PheromoneChannel, PheromoneMap},
    types::{MoveDir, NpcAnt, Player, Position, Tile},
    world::World,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub world: World,
    pub players: Vec<Player>,
    pub npcs: Vec<NpcAnt>,
    #[serde(default)]
    pub placed_art: Vec<PlacedArt>,
    pub event_log: Vec<String>,
    pub config: Value,
    #[serde(default)]
    pub simulation_paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DigProgress {
    pub target: Position,
    pub tile: Tile,
    pub steps: u8,
    pub last_tick: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    Join { name: String, token: String },
    Action(Action),
    RequestPheromoneMap {
        hive_id: u16,
        channel: PheromoneChannel,
    },
    ConfigSet { path: String, value: Value },
    Give {
        target: String,
        resource: String,
        amount: u16,
    },
    DigArea {
        width: u16,
        height: u16,
    },
    PutArea {
        resource: String,
        width: u16,
        height: u16,
    },
    DebugNpcStart,
    DebugNpcStop,
    DebugNpcStatus,
    SaveGameState {
        label: String,
    },
    ListGameStates,
    DeleteGameState {
        selector: String,
    },
    DeleteAllGameStates,
    LoadGameState {
        selector: String,
    },
    SetSimulationPaused {
        paused: bool,
    },
    WorldReset { seed: Option<u64> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    FullSyncStart(FullSyncStart),
    FullSyncChunk(FullSyncChunk),
    FullSyncComplete(FullSyncComplete),
    PheromoneMap(PheromoneMap),
    Patch(PatchFrame),
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullSyncStart {
    pub player_id: u8,
    pub tick: u64,
    pub world_width: i32,
    pub world_height: i32,
    pub total_rows: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullSyncChunk {
    pub start_row: i32,
    pub rows: Vec<Vec<Tile>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullSyncComplete {
    pub players: Vec<Player>,
    pub npcs: Vec<NpcAnt>,
    #[serde(default)]
    pub placed_art: Vec<PlacedArt>,
    pub event_log: Vec<String>,
    pub config: Value,
    #[serde(default)]
    pub simulation_paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchFrame {
    pub tick: u64,
    pub tiles: Vec<TileUpdate>,
    pub players: Option<Vec<Player>>,
    pub npcs: Option<Vec<NpcAnt>>,
    #[serde(default)]
    pub placed_art: Option<Vec<PlacedArt>>,
    pub event_log: Option<Vec<String>>,
    pub config: Option<Value>,
    #[serde(default)]
    pub simulation_paused: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileUpdate {
    pub pos: Position,
    pub tile: Tile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacedArt {
    pub asset_id: String,
    pub pos: Position,
    #[serde(default)]
    pub hive_id: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    Move(MoveDir),
    Dig(MoveDir),
    Place {
        dir: MoveDir,
        material: PlaceMaterial,
    },
    PlaceQueen,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlaceMaterial {
    Dirt,
    Stone,
    Food,
    Queen,
}
