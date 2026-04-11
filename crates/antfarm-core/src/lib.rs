use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

pub const MAX_PLAYERS: usize = 5;
pub const WORLD_WIDTH: i32 = 160;
pub const WORLD_HEIGHT: i32 = 80;
pub const SURFACE_Y: i32 = 18;
pub const TICK_MILLIS: u64 = 120;
pub const STONE_DIG_STEPS: u8 = 10;
pub const DEFAULT_SOIL_SETTLE_FREQUENCY: f64 = 0.01;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
}

impl Tile {
    pub fn glyph(self) -> char {
        match self {
            Self::Empty => ' ',
            Self::Dirt => '.',
            Self::Stone => '#',
            Self::Resource => '*',
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
    pub inventory: HashMap<String, u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpcAnt {
    pub id: u16,
    pub pos: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    width: i32,
    height: i32,
    tiles: Vec<Tile>,
}

impl World {
    pub fn new(seed: u64, width: i32, height: i32) -> Self {
        let mut tiles = vec![Tile::Empty; (width * height) as usize];
        let mut rng = StdRng::seed_from_u64(seed);

        for y in 0..height {
            for x in 0..width {
                let tile = if y < SURFACE_Y {
                    Tile::Empty
                } else if y == SURFACE_Y {
                    Tile::Dirt
                } else {
                    let roll = rng.random_range(0..100);
                    if roll < 7 {
                        Tile::Stone
                    } else if roll < 11 {
                        Tile::Resource
                    } else {
                        Tile::Dirt
                    }
                };
                tiles[(y * width + x) as usize] = tile;
            }
        }

        Self {
            width,
            height,
            tiles,
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn in_bounds(&self, pos: Position) -> bool {
        pos.x >= 0 && pos.x < self.width && pos.y >= 0 && pos.y < self.height
    }

    pub fn tile(&self, pos: Position) -> Option<Tile> {
        self.in_bounds(pos)
            .then(|| self.tiles[(pos.y * self.width + pos.x) as usize])
    }

    pub fn set_tile(&mut self, pos: Position, tile: Tile) -> bool {
        if !self.in_bounds(pos) {
            return false;
        }
        self.tiles[(pos.y * self.width + pos.x) as usize] = tile;
        true
    }

    pub fn is_walkable(&self, pos: Position) -> bool {
        matches!(self.tile(pos), Some(Tile::Empty))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub world: World,
    pub players: Vec<Player>,
    pub npcs: Vec<NpcAnt>,
    pub event_log: Vec<String>,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DigProgress {
    pub target: Position,
    pub tile: Tile,
    pub steps: u8,
    pub last_tick: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinAck {
    pub player_id: u8,
    pub snapshot: Snapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    Join { name: String },
    Action(Action),
    ConfigSet { path: String, value: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    Joined(JoinAck),
    Snapshot(Snapshot),
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    Move(MoveDir),
    Dig(MoveDir),
    Place {
        dir: MoveDir,
        material: PlaceMaterial,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlaceMaterial {
    Dirt,
    Stone,
}

#[derive(Debug, Clone)]
pub struct GameState {
    pub tick: u64,
    pub world: World,
    pub players: HashMap<u8, Player>,
    pub npcs: Vec<NpcAnt>,
    pub event_log: Vec<String>,
    pub config: Value,
    dig_progress: HashMap<u8, DigProgress>,
    rng: StdRng,
    next_player_id: u8,
}

impl GameState {
    pub fn new() -> Self {
        let seed = 7;
        let world = World::new(seed, WORLD_WIDTH, WORLD_HEIGHT);
        let npcs = vec![
            NpcAnt {
                id: 1,
                pos: Position {
                    x: 20,
                    y: SURFACE_Y,
                },
            },
            NpcAnt {
                id: 2,
                pos: Position {
                    x: 120,
                    y: SURFACE_Y + 2,
                },
            },
        ];

        Self {
            tick: 0,
            world,
            players: HashMap::new(),
            npcs,
            event_log: vec!["Server booted ant colony".to_string()],
            config: default_config(),
            dig_progress: HashMap::new(),
            rng: StdRng::seed_from_u64(seed + 1),
            next_player_id: 1,
        }
    }

    pub fn add_player(&mut self, name: String) -> Result<(u8, Snapshot), String> {
        if self.players.len() >= MAX_PLAYERS {
            return Err(format!("Room full: max {} players", MAX_PLAYERS));
        }

        let player_id = self.next_player_id;
        self.next_player_id = self.next_player_id.saturating_add(1);

        let spawn_x = 8 + (self.players.len() as i32 * 6);
        let player = Player {
            id: player_id,
            name: name.clone(),
            pos: Position {
                x: spawn_x.min(self.world.width() - 2),
                y: SURFACE_Y - 1,
            },
            facing: Facing::Right,
            inventory: HashMap::from([
                ("dirt".to_string(), 8),
                ("ore".to_string(), 0),
                ("stone".to_string(), 0),
            ]),
        };

        self.players.insert(player_id, player);
        self.push_event(format!("{name} joined as ant {player_id}"));
        let snapshot = self.snapshot();
        Ok((player_id, snapshot))
    }

    pub fn remove_player(&mut self, player_id: u8) {
        self.dig_progress.remove(&player_id);
        if let Some(player) = self.players.remove(&player_id) {
            self.push_event(format!("{} left the colony", player.name));
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut players: Vec<_> = self.players.values().cloned().collect();
        players.sort_by_key(|player| player.id);
        Snapshot {
            tick: self.tick,
            world: self.world.clone(),
            players,
            npcs: self.npcs.clone(),
            event_log: self.event_log.clone(),
            config: self.config.clone(),
        }
    }

    pub fn apply_action(&mut self, player_id: u8, action: Action) {
        match action {
            Action::Move(dir) => self.move_player(player_id, dir),
            Action::Dig(dir) => self.dig(player_id, dir),
            Action::Place { dir, material } => self.place(player_id, dir, material),
        }
    }

    pub fn set_config_value(&mut self, path: &str, value: Value) -> Result<(), String> {
        if path.trim().is_empty() {
            return Err("config path cannot be empty".to_string());
        }

        set_config_path(&mut self.config, path, value)?;
        self.push_event(format!("Config updated: {path}"));
        Ok(())
    }

    pub fn tick(&mut self) {
        self.tick += 1;
        self.tick_soil_settling();

        let player_positions: Vec<_> = self.players.values().map(|player| player.pos).collect();
        let mut events = Vec::new();
        for npc in &mut self.npcs {
            let Some(target) = nearest_target(npc.pos, &player_positions) else {
                continue;
            };

            let dx = (target.x - npc.pos.x).signum();
            let dy = (target.y - npc.pos.y).signum();

            for next in [npc.pos.offset(dx, 0), npc.pos.offset(0, dy)] {
                if !self.world.in_bounds(next) {
                    continue;
                }
                match self.world.tile(next) {
                    Some(Tile::Empty) => {
                        npc.pos = next;
                        break;
                    }
                    Some(Tile::Dirt) | Some(Tile::Resource) => {
                        self.world.set_tile(next, Tile::Empty);
                        events.push(format!(
                            "NPC ant {} tunneled at {},{}",
                            npc.id, next.x, next.y
                        ));
                        break;
                    }
                    Some(Tile::Stone) | None => {}
                }
            }
        }

        let npc_positions: Vec<_> = self.npcs.iter().map(|npc| npc.pos).collect();
        for player in self.players.values_mut() {
            if npc_positions.contains(&player.pos) {
                let _ = remove_inventory(&mut player.inventory, "dirt", 1);
                events.push(format!("{} was disturbed by an NPC ant", player.name));
            }
        }

        for event in events {
            self.push_event(event);
        }
    }

    fn move_player(&mut self, player_id: u8, dir: MoveDir) {
        self.dig_progress.remove(&player_id);
        let Some(current) = self.players.get(&player_id).cloned() else {
            return;
        };

        let (dx, dy) = dir.delta();
        let next = current.pos.offset(dx, dy);
        if !self.world.in_bounds(next) {
            return;
        }

        let target_tile = self.world.tile(next);
        if matches!(target_tile, Some(Tile::Empty)) && !self.occupied_by_actor(next) {
            if let Some(player) = self.players.get_mut(&player_id) {
                player.pos = next;
                if dx < 0 {
                    player.facing = Facing::Left;
                } else if dx > 0 {
                    player.facing = Facing::Right;
                }
            }
        }
    }

    fn dig(&mut self, player_id: u8, dir: MoveDir) {
        let Some(current) = self.players.get(&player_id).cloned() else {
            return;
        };

        let (dx, dy) = dir.delta();
        let target = current.pos.offset(dx, dy);
        let Some(tile) = self.world.tile(target) else {
            self.dig_progress.remove(&player_id);
            return;
        };

        if tile == Tile::Empty {
            self.dig_progress.remove(&player_id);
            return;
        }

        let required_steps = match tile {
            Tile::Stone => STONE_DIG_STEPS,
            Tile::Dirt | Tile::Resource => 1,
            Tile::Empty => 0,
        };

        let steps = {
            let entry = self.dig_progress.entry(player_id).or_insert(DigProgress {
                target,
                tile,
                steps: 0,
                last_tick: self.tick,
            });

            let is_consecutive = entry.target == target
                && entry.tile == tile
                && self.tick.saturating_sub(entry.last_tick) <= 1;

            if !is_consecutive {
                *entry = DigProgress {
                    target,
                    tile,
                    steps: 0,
                    last_tick: self.tick,
                };
            }

            entry.steps = entry.steps.saturating_add(1);
            entry.last_tick = self.tick;
            entry.steps
        };

        if tile == Tile::Stone && steps < required_steps {
            let remaining = required_steps.saturating_sub(steps);
            self.push_event(format!(
                "{} chips stone ({remaining} digs left)",
                current.name
            ));
            return;
        }

        self.world.set_tile(target, Tile::Empty);
        self.dig_progress.remove(&player_id);
        let mut event = None;
        if let Some(player) = self.players.get_mut(&player_id) {
            match tile {
                Tile::Dirt => add_inventory(&mut player.inventory, "dirt", 1),
                Tile::Stone => add_inventory(&mut player.inventory, "stone", 1),
                Tile::Resource => {
                    add_inventory(&mut player.inventory, "dirt", 1);
                    add_inventory(&mut player.inventory, "ore", 1);
                    event = Some(format!("{} found a resource vein", player.name));
                }
                Tile::Empty => {}
            }
        }
        if let Some(event) = event {
            self.push_event(event);
        }
    }

    fn place(&mut self, player_id: u8, dir: MoveDir, material: PlaceMaterial) {
        self.dig_progress.remove(&player_id);
        let Some(current) = self.players.get(&player_id).cloned() else {
            return;
        };

        let (dx, dy) = dir.delta();
        let target = current.pos.offset(dx, dy);
        if !self.world.in_bounds(target) || self.occupied_by_actor(target) {
            return;
        }

        if !matches!(self.world.tile(target), Some(Tile::Empty)) {
            return;
        }

        let Some(player) = self.players.get_mut(&player_id) else {
            return;
        };
        let inventory_key = match material {
            PlaceMaterial::Dirt => "dirt",
            PlaceMaterial::Stone => "stone",
        };
        let tile = match material {
            PlaceMaterial::Dirt => Tile::Dirt,
            PlaceMaterial::Stone => Tile::Stone,
        };

        if inventory_count(&player.inventory, inventory_key) == 0 {
            let name = player.name.clone();
            let _ = player;
            self.push_event(format!("{name} has no {inventory_key} to place"));
            return;
        }

        remove_inventory(&mut player.inventory, inventory_key, 1);
        self.world.set_tile(target, tile);
    }

    fn occupied_by_actor(&self, pos: Position) -> bool {
        self.players.values().any(|player| player.pos == pos)
            || self.npcs.iter().any(|npc| npc.pos == pos)
    }

    fn push_event(&mut self, event: String) {
        self.event_log.push(event);
        if self.event_log.len() > 8 {
            let extra = self.event_log.len() - 8;
            self.event_log.drain(0..extra);
        }
    }

    fn tick_soil_settling(&mut self) {
        let frequency = config_f64(
            &self.config,
            "soil.settle_frequency",
            DEFAULT_SOIL_SETTLE_FREQUENCY,
        )
        .clamp(0.0, 1.0);

        if frequency <= 0.0 {
            return;
        }

        let occupied: Vec<_> = self
            .players
            .values()
            .map(|player| player.pos)
            .chain(self.npcs.iter().map(|npc| npc.pos))
            .collect();

        for y in (0..self.world.height()).rev() {
            for x in 0..self.world.width() {
                let pos = Position { x, y };
                if self.world.tile(pos) != Some(Tile::Dirt) || occupied.contains(&pos) {
                    continue;
                }
                if self.rng.random::<f64>() > frequency {
                    continue;
                }

                let below = pos.offset(0, 1);
                let down_right = pos.offset(1, 1);
                let right = pos.offset(1, 0);
                let down_left = pos.offset(-1, 1);
                let left = pos.offset(-1, 0);

                let target = if self.world.in_bounds(below) && self.world.tile(below) == Some(Tile::Empty)
                {
                    if !occupied.contains(&below) {
                        Some(below)
                    } else {
                        None
                    }
                } else if self.world.in_bounds(right)
                    && self.world.in_bounds(down_right)
                    && self.world.tile(right) == Some(Tile::Empty)
                    && self.world.tile(down_right) == Some(Tile::Empty)
                    && !occupied.contains(&right)
                    && !occupied.contains(&down_right)
                {
                    Some(down_right)
                } else if self.world.in_bounds(left)
                    && self.world.in_bounds(down_left)
                    && self.world.tile(left) == Some(Tile::Empty)
                    && self.world.tile(down_left) == Some(Tile::Empty)
                    && !occupied.contains(&left)
                    && !occupied.contains(&down_left)
                {
                    Some(down_left)
                } else {
                    None
                };

                let Some(target) = target else {
                    continue;
                };

                self.world.set_tile(pos, Tile::Empty);
                self.world.set_tile(target, Tile::Dirt);
            }
        }
    }
}

fn default_config() -> Value {
    json!({
        "soil": {
            "settle_frequency": DEFAULT_SOIL_SETTLE_FREQUENCY
        }
    })
}

fn inventory_count(inventory: &HashMap<String, u16>, key: &str) -> u16 {
    inventory.get(key).copied().unwrap_or(0)
}

fn add_inventory(inventory: &mut HashMap<String, u16>, key: &str, amount: u16) {
    let entry = inventory.entry(key.to_string()).or_insert(0);
    *entry = entry.saturating_add(amount);
}

fn remove_inventory(inventory: &mut HashMap<String, u16>, key: &str, amount: u16) -> bool {
    let Some(entry) = inventory.get_mut(key) else {
        return false;
    };
    if *entry < amount {
        return false;
    }
    *entry -= amount;
    true
}

fn set_config_path(root: &mut Value, path: &str, value: Value) -> Result<(), String> {
    let segments: Vec<_> = path
        .split('.')
        .filter(|segment| !segment.trim().is_empty())
        .collect();
    if segments.is_empty() {
        return Err("config path cannot be empty".to_string());
    }

    if !root.is_object() {
        *root = Value::Object(Map::new());
    }

    let mut current = root;
    for segment in &segments[..segments.len() - 1] {
        let Some(object) = current.as_object_mut() else {
            return Err(format!("path segment {segment} is not an object"));
        };
        current = object
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !current.is_object() {
            *current = Value::Object(Map::new());
        }
    }

    let final_key = segments.last().expect("non-empty segments");
    let Some(object) = current.as_object_mut() else {
        return Err(format!("parent of {final_key} is not an object"));
    };
    object.insert((*final_key).to_string(), value);
    Ok(())
}

fn config_f64(root: &Value, path: &str, default: f64) -> f64 {
    let mut current = root;
    for segment in path.split('.').filter(|segment| !segment.trim().is_empty()) {
        let Some(next) = current.get(segment) else {
            return default;
        };
        current = next;
    }
    current.as_f64().unwrap_or(default)
}

fn nearest_target(origin: Position, positions: &[Position]) -> Option<Position> {
    positions
        .iter()
        .copied()
        .min_by_key(|pos| (pos.x - origin.x).abs() + (pos.y - origin.y).abs())
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
