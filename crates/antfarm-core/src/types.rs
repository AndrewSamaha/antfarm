use serde::{Deserialize, Serialize, de::Deserializer};
use std::collections::HashMap;

use crate::{pheromones::AntBehaviorState, world::World};

pub const DEFAULT_WORKER_ROLE_PATH: &str = "food_gatherer";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

impl Position {
    pub fn offset(self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Tile {
    Empty,
    Dirt,
    Stone,
    Resource,
    Food,
    Bedrock,
}

impl Tile {
    pub fn glyph(self) -> char {
        match self {
            Self::Empty => ' ',
            Self::Dirt => '.',
            Self::Stone => '#',
            Self::Resource => '*',
            Self::Food => '%',
            Self::Bedrock => '=',
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Facing {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MoveDir {
    Up,
    Down,
    Left,
    Right,
}

impl MoveDir {
    pub fn delta(self) -> (i32, i32) {
        match self {
            Self::Up => (0, -1),
            Self::Down => (0, 1),
            Self::Left => (-1, 0),
            Self::Right => (1, 0),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum QueenChamberGrowthMode {
    #[default]
    Outward,
    Inward,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueenChamberState {
    pub radius_x: Option<i32>,
    pub radius_y: Option<i32>,
    pub anchor: Option<Position>,
    pub has_left_anchor: bool,
    pub growth_mode: QueenChamberGrowthMode,
}

impl Default for QueenChamberState {
    fn default() -> Self {
        Self {
            radius_x: None,
            radius_y: None,
            anchor: None,
            has_left_anchor: false,
            growth_mode: QueenChamberGrowthMode::Outward,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", content = "state", rename_all = "snake_case")]
pub enum NpcRoleState {
    #[default]
    None,
    QueenChamber(QueenChamberState),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: u8,
    pub name: String,
    pub pos: Position,
    pub facing: Facing,
    #[serde(default)]
    pub hive_id: Option<u16>,
    pub inventory: HashMap<String, u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NpcAnt {
    pub id: u16,
    pub pos: Position,
    #[serde(default)]
    pub inventory: HashMap<String, u16>,
    #[serde(default)]
    pub kind: NpcKind,
    #[serde(default = "default_worker_health")]
    pub health: u16,
    #[serde(default)]
    pub food: u16,
    #[serde(default)]
    pub hive_id: Option<u16>,
    #[serde(default)]
    pub age_ticks: u16,
    #[serde(default)]
    pub behavior: AntBehaviorState,
    #[serde(default)]
    pub carrying_food: bool,
    #[serde(default)]
    pub carrying_food_ticks: u16,
    #[serde(default)]
    pub home_trail_steps: Option<u16>,
    #[serde(default)]
    pub recent_home_dir: Option<MoveDir>,
    #[serde(default)]
    pub recent_food_dir: Option<MoveDir>,
    #[serde(default)]
    pub recent_home_memory_ticks: u8,
    #[serde(default)]
    pub recent_food_memory_ticks: u8,
    #[serde(default)]
    pub recent_positions: Vec<Position>,
    #[serde(default)]
    pub search_destination: Option<Position>,
    #[serde(default)]
    pub search_destination_stuck_ticks: u8,
    #[serde(default)]
    pub has_delivered_food: bool,
    #[serde(default)]
    pub last_dirt_place_tick: Option<u64>,
    #[serde(default)]
    pub last_egg_laid_tick: Option<u64>,
    #[serde(default)]
    pub last_egg_hatched_tick: Option<u64>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub role_state: NpcRoleState,
}

impl NpcAnt {
    pub fn queen_chamber_state(&self) -> QueenChamberState {
        match &self.role_state {
            NpcRoleState::QueenChamber(state) => *state,
            NpcRoleState::None => QueenChamberState::default(),
        }
    }

    pub fn set_queen_chamber_state(&mut self, state: QueenChamberState) {
        self.role_state = NpcRoleState::QueenChamber(state);
    }

    pub fn clear_role_state(&mut self) {
        self.role_state = NpcRoleState::None;
    }
}

#[derive(Debug, Deserialize)]
struct NpcAntWire {
    id: u16,
    pos: Position,
    #[serde(default)]
    inventory: HashMap<String, u16>,
    #[serde(default)]
    kind: NpcKind,
    #[serde(default = "default_worker_health")]
    health: u16,
    #[serde(default)]
    food: u16,
    #[serde(default)]
    hive_id: Option<u16>,
    #[serde(default)]
    age_ticks: u16,
    #[serde(default)]
    behavior: AntBehaviorState,
    #[serde(default)]
    carrying_food: bool,
    #[serde(default)]
    carrying_food_ticks: u16,
    #[serde(default)]
    home_trail_steps: Option<u16>,
    #[serde(default)]
    recent_home_dir: Option<MoveDir>,
    #[serde(default)]
    recent_food_dir: Option<MoveDir>,
    #[serde(default)]
    recent_home_memory_ticks: u8,
    #[serde(default)]
    recent_food_memory_ticks: u8,
    #[serde(default)]
    recent_positions: Vec<Position>,
    #[serde(default)]
    search_destination: Option<Position>,
    #[serde(default)]
    search_destination_stuck_ticks: u8,
    #[serde(default)]
    has_delivered_food: bool,
    #[serde(default)]
    last_dirt_place_tick: Option<u64>,
    #[serde(default)]
    last_egg_laid_tick: Option<u64>,
    #[serde(default)]
    last_egg_hatched_tick: Option<u64>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    role_state: Option<NpcRoleState>,
    #[serde(default)]
    chamber_radius_x: Option<i32>,
    #[serde(default)]
    chamber_radius_y: Option<i32>,
    #[serde(default)]
    chamber_anchor: Option<Position>,
    #[serde(default)]
    chamber_has_left_anchor: bool,
    #[serde(default)]
    chamber_growth_mode: QueenChamberGrowthMode,
}

impl<'de> Deserialize<'de> for NpcAnt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = NpcAntWire::deserialize(deserializer)?;
        let role_state = raw.role_state.unwrap_or_else(|| {
            if raw.chamber_radius_x.is_some()
                || raw.chamber_radius_y.is_some()
                || raw.chamber_anchor.is_some()
                || raw.chamber_has_left_anchor
                || raw.chamber_growth_mode != QueenChamberGrowthMode::Outward
            {
                NpcRoleState::QueenChamber(QueenChamberState {
                    radius_x: raw.chamber_radius_x,
                    radius_y: raw.chamber_radius_y,
                    anchor: raw.chamber_anchor,
                    has_left_anchor: raw.chamber_has_left_anchor,
                    growth_mode: raw.chamber_growth_mode,
                })
            } else {
                NpcRoleState::None
            }
        });
        Ok(Self {
            id: raw.id,
            pos: raw.pos,
            inventory: raw.inventory,
            kind: raw.kind,
            health: raw.health,
            food: raw.food,
            hive_id: raw.hive_id,
            age_ticks: raw.age_ticks,
            behavior: raw.behavior,
            carrying_food: raw.carrying_food,
            carrying_food_ticks: raw.carrying_food_ticks,
            home_trail_steps: raw.home_trail_steps,
            recent_home_dir: raw.recent_home_dir,
            recent_food_dir: raw.recent_food_dir,
            recent_home_memory_ticks: raw.recent_home_memory_ticks,
            recent_food_memory_ticks: raw.recent_food_memory_ticks,
            recent_positions: raw.recent_positions,
            search_destination: raw.search_destination,
            search_destination_stuck_ticks: raw.search_destination_stuck_ticks,
            has_delivered_food: raw.has_delivered_food,
            last_dirt_place_tick: raw.last_dirt_place_tick,
            last_egg_laid_tick: raw.last_egg_laid_tick,
            last_egg_hatched_tick: raw.last_egg_hatched_tick,
            role: raw.role,
            role_state,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum NpcKind {
    #[default]
    Worker,
    Queen,
    Egg,
}

impl NpcKind {
    pub fn max_health(self) -> u16 {
        match self {
            Self::Worker => crate::constants::NPC_WORKER_MAX_HEALTH,
            Self::Queen => crate::constants::NPC_QUEEN_MAX_HEALTH,
            Self::Egg => crate::constants::NPC_EGG_MAX_HEALTH,
        }
    }

    pub fn max_food(self) -> u16 {
        match self {
            Self::Worker => crate::constants::NPC_WORKER_MAX_FOOD,
            Self::Queen => crate::constants::NPC_QUEEN_MAX_FOOD,
            Self::Egg => crate::constants::NPC_EGG_MAX_FOOD,
        }
    }
}

fn default_worker_health() -> u16 {
    NpcKind::Worker.max_health()
}

#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub left: i32,
    pub top: i32,
    pub width: u16,
    pub height: u16,
}

impl Viewport {
    pub fn follow(center: Position, screen_width: u16, screen_height: u16, world: &World) -> Self {
        let width = screen_width.max(1);
        let height = screen_height.max(1);
        let half_w = i32::from(width) / 2;
        let half_h = i32::from(height) / 2;

        let max_left = (world.width() - i32::from(width)).max(0);
        let max_top = (world.height() - i32::from(height)).max(0);

        Self {
            left: (center.x - half_w).clamp(0, max_left),
            top: (center.y - half_h).clamp(0, max_top),
            width,
            height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn npc_ant_deserializes_legacy_queen_chamber_fields_into_role_state() {
        let npc: NpcAnt = serde_json::from_value(json!({
            "id": 7,
            "pos": { "x": 12, "y": 34 },
            "role": "hive_maintenance.queen_chamber",
            "chamber_radius_x": 6,
            "chamber_radius_y": 5,
            "chamber_anchor": { "x": 10, "y": 29 },
            "chamber_has_left_anchor": true,
            "chamber_growth_mode": "Inward"
        }))
        .expect("legacy npc ant should deserialize");

        assert_eq!(
            npc.role_state,
            NpcRoleState::QueenChamber(QueenChamberState {
                radius_x: Some(6),
                radius_y: Some(5),
                anchor: Some(Position { x: 10, y: 29 }),
                has_left_anchor: true,
                growth_mode: QueenChamberGrowthMode::Inward,
            })
        );
    }

    #[test]
    fn npc_ant_serializes_role_state_without_legacy_chamber_fields() {
        let npc = NpcAnt {
            id: 7,
            pos: Position { x: 12, y: 34 },
            inventory: HashMap::new(),
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 0,
            hive_id: None,
            age_ticks: 0,
            behavior: AntBehaviorState::Idle,
            carrying_food: false,
            carrying_food_ticks: 0,
            home_trail_steps: None,
            recent_home_dir: None,
            recent_food_dir: None,
            recent_home_memory_ticks: 0,
            recent_food_memory_ticks: 0,
            recent_positions: Vec::new(),
            search_destination: None,
            search_destination_stuck_ticks: 0,
            has_delivered_food: false,
            last_dirt_place_tick: None,
            last_egg_laid_tick: None,
            last_egg_hatched_tick: None,
            role: Some("hive_maintenance.queen_chamber".to_string()),
            role_state: NpcRoleState::QueenChamber(QueenChamberState {
                radius_x: Some(6),
                radius_y: Some(5),
                anchor: Some(Position { x: 10, y: 29 }),
                has_left_anchor: true,
                growth_mode: QueenChamberGrowthMode::Inward,
            }),
        };

        let value = serde_json::to_value(npc).expect("npc ant should serialize");
        let object = value.as_object().expect("npc ant json object");
        assert!(object.contains_key("role_state"));
        assert!(!object.contains_key("chamber_radius_x"));
        assert!(!object.contains_key("chamber_radius_y"));
        assert!(!object.contains_key("chamber_anchor"));
        assert!(!object.contains_key("chamber_has_left_anchor"));
        assert!(!object.contains_key("chamber_growth_mode"));
    }
}
