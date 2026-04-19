use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{pheromones::AntBehaviorState, world::World};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub recent_home_dir: Option<MoveDir>,
    #[serde(default)]
    pub recent_food_dir: Option<MoveDir>,
    #[serde(default)]
    pub recent_home_memory_ticks: u8,
    #[serde(default)]
    pub recent_food_memory_ticks: u8,
    #[serde(default)]
    pub recent_positions: Vec<Position>,
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
