use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

pub const MAX_PLAYERS: usize = 5;
pub const WORLD_WIDTH: i32 = 160;
pub const SURFACE_Y: i32 = 18;
pub const TICK_MILLIS: u64 = 120;
pub const STONE_DIG_STEPS: u8 = 10;
pub const DEFAULT_SOIL_SETTLE_FREQUENCY: f64 = 0.01;
pub const DEFAULT_WORLD_SNAPSHOT_INTERVAL_SECONDS: f64 = 5.0;
pub const DEFAULT_WORLD_MAX_DEPTH: i32 = -255;
pub const DEFAULT_WORLD_SEED: u64 = 7;

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
    pub fn empty(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            tiles: vec![Tile::Empty; (width * height) as usize],
        }
    }

    pub fn generate(seed: u64, width: i32, config: &Value) -> Self {
        let max_depth = config_i32(config, "world.max_depth", DEFAULT_WORLD_MAX_DEPTH).min(-1);
        let height = SURFACE_Y + max_depth.abs() + 1;
        let mut world = Self::empty(width, height);

        let terrain_variation =
            config_i32(config, "world.gen_params.soil.surface_variation", 4).max(0);
        let dirt_depth = config_i32(config, "world.gen_params.soil.dirt_depth", 150).max(1);
        let dirt_variation = config_i32(config, "world.gen_params.soil.dirt_variation", 3).max(0);
        let chunk_width = config_i32(config, "world.gen_params.chunk_width", 16).clamp(4, 64);

        let surface_heights: Vec<i32> = (0..width)
            .map(|x| {
                let noise = fbm_1d(seed ^ 0x51_7A_2D, f64::from(x) * 0.045, 3);
                let offset = (noise * f64::from(terrain_variation)).round() as i32;
                (SURFACE_Y + offset).clamp(4, height - 3)
            })
            .collect();

        for x in 0..width {
            let surface_y = surface_heights[x as usize];
            let local_dirt_depth = (dirt_depth
                + (fbm_1d(seed ^ 0x92_11_4F, f64::from(x) * 0.09, 2) * f64::from(dirt_variation))
                    .round() as i32)
                .max(1);

            for y in 0..height {
                let pos = Position { x, y };
                let tile = if y < surface_y {
                    Tile::Empty
                } else if y == height - 1 {
                    Tile::Bedrock
                } else {
                    let depth = y - surface_y;
                    if depth <= local_dirt_depth {
                        Tile::Dirt
                    } else {
                        Tile::Stone
                    }
                };
                world.set_tile(pos, tile);
            }
        }

        apply_cluster_pass(
            &mut world,
            seed ^ 0xA5_0E,
            chunk_width,
            Tile::Resource,
            &DepositConfig::from_config(
                config,
                "world.gen_params.ore",
                2,
                6,
                18,
                20,
                max_depth.abs() - 8,
            ),
            &[Tile::Stone],
            &surface_heights,
        );

        apply_depth_scaled_cluster_pass(
            &mut world,
            seed ^ 0x57_0A_E0,
            chunk_width,
            Tile::Stone,
            &DepthScaledDepositConfig::from_config(
                config,
                "world.gen_params.stone_pockets",
                1.0,
                4,
                12,
                6,
                max_depth.abs() - 20,
                1.8,
            ),
            &[Tile::Dirt],
            &surface_heights,
        );

        apply_cluster_pass(
            &mut world,
            seed ^ 0xF0_0D,
            chunk_width,
            Tile::Food,
            &DepositConfig::from_config(config, "world.gen_params.food", 3, 6, 14, 0, 50),
            &[Tile::Dirt, Tile::Stone],
            &surface_heights,
        );

        world
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

    pub fn row_tiles(&self, row: i32) -> Vec<Tile> {
        if row < 0 || row >= self.height {
            return Vec::new();
        }
        (0..self.width)
            .filter_map(|x| self.tile(Position { x, y: row }))
            .collect()
    }

    pub fn set_row_tiles(&mut self, row: i32, tiles: &[Tile]) {
        if row < 0 || row >= self.height {
            return;
        }
        for (x, tile) in tiles.iter().enumerate() {
            if x as i32 >= self.width {
                break;
            }
            let _ = self.set_tile(
                Position {
                    x: x as i32,
                    y: row,
                },
                *tile,
            );
        }
    }

    pub fn is_walkable(&self, pos: Position) -> bool {
        matches!(self.tile(pos), Some(Tile::Empty))
    }

    pub fn spawn_y_for_column(&self, x: i32) -> i32 {
        for y in 0..self.height {
            if self.tile(Position { x, y }) != Some(Tile::Empty) {
                return y.saturating_sub(1);
            }
        }
        0
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
pub enum ClientMessage {
    Join { name: String },
    Action(Action),
    ConfigSet { path: String, value: Value },
    WorldReset { seed: Option<u64> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    FullSyncStart(FullSyncStart),
    FullSyncChunk(FullSyncChunk),
    FullSyncComplete(FullSyncComplete),
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
    pub event_log: Vec<String>,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchFrame {
    pub tick: u64,
    pub tiles: Vec<TileUpdate>,
    pub players: Option<Vec<Player>>,
    pub npcs: Option<Vec<NpcAnt>>,
    pub event_log: Option<Vec<String>>,
    pub config: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileUpdate {
    pub pos: Position,
    pub tile: Tile,
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
    dirty_tiles: HashMap<Position, Tile>,
    players_dirty: bool,
    npcs_dirty: bool,
    event_log_dirty: bool,
    config_dirty: bool,
    rng: StdRng,
    next_player_id: u8,
}

impl GameState {
    pub fn new() -> Self {
        Self::from_config(default_server_config())
    }

    pub fn from_config(config: Value) -> Self {
        let config = merge_with_default_config(config);
        let seed = config_u64(&config, "world.seed", DEFAULT_WORLD_SEED);
        let world = World::generate(seed, WORLD_WIDTH, &config);

        Self {
            tick: 0,
            npcs: default_npcs(&world),
            world,
            players: HashMap::new(),
            event_log: vec!["Server booted ant colony".to_string()],
            config,
            dig_progress: HashMap::new(),
            dirty_tiles: HashMap::new(),
            players_dirty: true,
            npcs_dirty: true,
            event_log_dirty: true,
            config_dirty: true,
            rng: StdRng::seed_from_u64(seed ^ 0xAB_CD_EF),
            next_player_id: 1,
        }
    }

    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        let config = merge_with_default_config(snapshot.config);
        let seed = config_u64(&config, "world.seed", DEFAULT_WORLD_SEED);
        Self {
            tick: snapshot.tick,
            world: snapshot.world,
            players: HashMap::new(),
            npcs: snapshot.npcs,
            event_log: vec!["Server restored world snapshot".to_string()],
            config,
            dig_progress: HashMap::new(),
            dirty_tiles: HashMap::new(),
            players_dirty: true,
            npcs_dirty: true,
            event_log_dirty: true,
            config_dirty: true,
            rng: StdRng::seed_from_u64(seed ^ 0xAB_CD_EF),
            next_player_id: 1,
        }
    }

    pub fn add_player(&mut self, name: String) -> Result<(u8, Snapshot), String> {
        if self.players.len() >= MAX_PLAYERS {
            return Err(format!("Room full: max {} players", MAX_PLAYERS));
        }

        let player_id = self.next_player_id;
        self.next_player_id = self.next_player_id.saturating_add(1);

        let spawn_x = (8 + self.players.len() as i32 * 6).min(self.world.width() - 2);
        let player = Player {
            id: player_id,
            name: name.clone(),
            pos: Position {
                x: spawn_x,
                y: self.world.spawn_y_for_column(spawn_x),
            },
            facing: Facing::Right,
            inventory: default_inventory(),
        };

        self.players.insert(player_id, player);
        self.players_dirty = true;
        self.push_event(format!("{name} joined as ant {player_id}"));
        Ok((player_id, self.snapshot()))
    }

    pub fn remove_player(&mut self, player_id: u8) {
        self.dig_progress.remove(&player_id);
        if let Some(player) = self.players.remove(&player_id) {
            self.players_dirty = true;
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
        self.config = merge_with_default_config(self.config.clone());
        self.config_dirty = true;
        self.push_event(format!("Config updated: {path}"));
        Ok(())
    }

    pub fn world_reset(&mut self, seed: Option<u64>) {
        let config = self.config.clone();
        let existing_players: Vec<(u8, String)> = self
            .players
            .iter()
            .map(|(id, player)| (*id, player.name.clone()))
            .collect();
        let next_player_id = self.next_player_id;
        let mut config = config;
        if let Some(seed) = seed {
            let _ = set_config_path(&mut config, "world.seed", Value::from(seed));
        }
        *self = Self::from_config(config);
        self.next_player_id = next_player_id;

        for (index, (id, name)) in existing_players.into_iter().enumerate() {
            let spawn_x = (8 + index as i32 * 6).min(self.world.width() - 2);
            self.players.insert(
                id,
                Player {
                    id,
                    name,
                    pos: Position {
                        x: spawn_x,
                        y: self.world.spawn_y_for_column(spawn_x),
                    },
                    facing: Facing::Right,
                    inventory: default_inventory(),
                },
            );
        }

        self.players_dirty = true;
        self.push_event("World reset by server command".to_string());
        if !self.players.is_empty() {
            self.push_event(format!("Respawned {} connected ants", self.players.len()));
        }
    }

    pub fn snapshot_interval_seconds(&self) -> f64 {
        config_f64(
            &self.config,
            "world.snapshot_interval",
            DEFAULT_WORLD_SNAPSHOT_INTERVAL_SECONDS,
        )
        .max(0.5)
    }

    pub fn tick(&mut self) {
        self.tick += 1;
        self.tick_soil_settling();

        let player_positions: Vec<_> = self.players.values().map(|player| player.pos).collect();
        let mut events = Vec::new();
        for index in 0..self.npcs.len() {
            let npc_pos = self.npcs[index].pos;
            let npc_id = self.npcs[index].id;
            let Some(target) = nearest_target(npc_pos, &player_positions) else {
                continue;
            };

            let dx = (target.x - npc_pos.x).signum();
            let dy = (target.y - npc_pos.y).signum();

            for next in [npc_pos.offset(dx, 0), npc_pos.offset(0, dy)] {
                if !self.world.in_bounds(next) {
                    continue;
                }
                match self.world.tile(next) {
                    Some(Tile::Empty) => {
                        self.npcs[index].pos = next;
                        self.npcs_dirty = true;
                        break;
                    }
                    Some(Tile::Dirt) | Some(Tile::Resource) | Some(Tile::Food) => {
                        self.set_world_tile(next, Tile::Empty);
                        events.push(format!(
                            "NPC ant {} tunneled at {},{}",
                            npc_id, next.x, next.y
                        ));
                        break;
                    }
                    Some(Tile::Stone) | Some(Tile::Bedrock) | None => {}
                }
            }
        }

        let npc_positions: Vec<_> = self.npcs.iter().map(|npc| npc.pos).collect();
        for player in self.players.values_mut() {
            if npc_positions.contains(&player.pos) {
                let _ = remove_inventory(&mut player.inventory, "dirt", 1);
                self.players_dirty = true;
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

        if matches!(self.world.tile(next), Some(Tile::Empty)) && !self.occupied_by_actor(next) {
            if let Some(player) = self.players.get_mut(&player_id) {
                player.pos = next;
                if dx < 0 {
                    player.facing = Facing::Left;
                } else if dx > 0 {
                    player.facing = Facing::Right;
                }
                self.players_dirty = true;
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

        match tile {
            Tile::Empty => {
                self.dig_progress.remove(&player_id);
                return;
            }
            Tile::Bedrock => {
                self.dig_progress.remove(&player_id);
                self.push_event(format!("{} hit bedrock", current.name));
                return;
            }
            Tile::Dirt | Tile::Resource | Tile::Food | Tile::Stone => {}
        }

        let required_steps = match tile {
            Tile::Stone => STONE_DIG_STEPS,
            Tile::Dirt | Tile::Resource | Tile::Food => 1,
            Tile::Empty | Tile::Bedrock => 0,
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
            self.push_event(format!(
                "{} chips stone ({} digs left)",
                current.name,
                required_steps.saturating_sub(steps)
            ));
            return;
        }

        self.set_world_tile(target, Tile::Empty);
        self.dig_progress.remove(&player_id);
        let mut event = None;
        if let Some(player) = self.players.get_mut(&player_id) {
            match tile {
                Tile::Dirt => add_inventory(&mut player.inventory, "dirt", 1),
                Tile::Stone => add_inventory(&mut player.inventory, "stone", 1),
                Tile::Resource => {
                    add_inventory(&mut player.inventory, "ore", 1);
                    event = Some(format!("{} found an ore vein", player.name));
                }
                Tile::Food => {
                    add_inventory(&mut player.inventory, "food", 1);
                    event = Some(format!("{} harvested food", player.name));
                }
                Tile::Empty | Tile::Bedrock => {}
            }
            self.players_dirty = true;
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
        self.set_world_tile(target, tile);
        self.players_dirty = true;
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
        self.event_log_dirty = true;
    }

    fn set_world_tile(&mut self, pos: Position, tile: Tile) {
        if self.world.tile(pos) == Some(tile) {
            return;
        }
        if self.world.set_tile(pos, tile) {
            self.dirty_tiles.insert(pos, tile);
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

                self.set_world_tile(pos, Tile::Empty);
                self.set_world_tile(target, Tile::Dirt);
            }
        }
    }

    pub fn take_patch(&mut self) -> Option<PatchFrame> {
        if self.dirty_tiles.is_empty()
            && !self.players_dirty
            && !self.npcs_dirty
            && !self.event_log_dirty
            && !self.config_dirty
        {
            return None;
        }

        let mut tiles: Vec<_> = self
            .dirty_tiles
            .drain()
            .map(|(pos, tile)| TileUpdate { pos, tile })
            .collect();
        tiles.sort_by_key(|update| (update.pos.y, update.pos.x));

        let players = self.players_dirty.then(|| {
            let mut players: Vec<_> = self.players.values().cloned().collect();
            players.sort_by_key(|player| player.id);
            players
        });

        let npcs = self.npcs_dirty.then(|| self.npcs.clone());
        let event_log = self.event_log_dirty.then(|| self.event_log.clone());
        let config = self.config_dirty.then(|| self.config.clone());

        self.players_dirty = false;
        self.npcs_dirty = false;
        self.event_log_dirty = false;
        self.config_dirty = false;

        Some(PatchFrame {
            tick: self.tick,
            tiles,
            players,
            npcs,
            event_log,
            config,
        })
    }
}

pub fn default_server_config() -> Value {
    default_config()
}

#[derive(Debug, Clone)]
struct DepositConfig {
    attempts_per_chunk: i32,
    cluster_min: i32,
    cluster_max: i32,
    min_depth: i32,
    max_depth: i32,
}

impl DepositConfig {
    fn from_config(
        config: &Value,
        path: &str,
        attempts_per_chunk: i32,
        cluster_min: i32,
        cluster_max: i32,
        min_depth: i32,
        max_depth: i32,
    ) -> Self {
        Self {
            attempts_per_chunk: config_i32(
                config,
                &format!("{path}.attempts_per_chunk"),
                attempts_per_chunk,
            )
            .max(0),
            cluster_min: config_i32(config, &format!("{path}.cluster_min"), cluster_min).max(1),
            cluster_max: config_i32(config, &format!("{path}.cluster_max"), cluster_max).max(1),
            min_depth: config_i32(config, &format!("{path}.min_depth"), min_depth).max(0),
            max_depth: config_i32(config, &format!("{path}.max_depth"), max_depth).max(0),
        }
    }
}

#[derive(Debug, Clone)]
struct DepthScaledDepositConfig {
    attempts_per_chunk: f64,
    cluster_min: i32,
    cluster_max: i32,
    min_depth: i32,
    max_depth: i32,
    depth_gain: f64,
}

impl DepthScaledDepositConfig {
    fn from_config(
        config: &Value,
        path: &str,
        attempts_per_chunk: f64,
        cluster_min: i32,
        cluster_max: i32,
        min_depth: i32,
        max_depth: i32,
        depth_gain: f64,
    ) -> Self {
        Self {
            attempts_per_chunk: config_f64(
                config,
                &format!("{path}.attempts_per_chunk"),
                attempts_per_chunk,
            )
            .max(0.0),
            cluster_min: config_i32(config, &format!("{path}.cluster_min"), cluster_min).max(1),
            cluster_max: config_i32(config, &format!("{path}.cluster_max"), cluster_max).max(1),
            min_depth: config_i32(config, &format!("{path}.min_depth"), min_depth).max(0),
            max_depth: config_i32(config, &format!("{path}.max_depth"), max_depth).max(0),
            depth_gain: config_f64(config, &format!("{path}.depth_gain"), depth_gain).max(0.1),
        }
    }
}

fn default_config() -> Value {
    json!({
        "soil": {
            "settle_frequency": DEFAULT_SOIL_SETTLE_FREQUENCY
        },
        "world": {
            "seed": DEFAULT_WORLD_SEED,
            "max_depth": DEFAULT_WORLD_MAX_DEPTH,
            "snapshot_interval": DEFAULT_WORLD_SNAPSHOT_INTERVAL_SECONDS,
            "gen_params": {
                "chunk_width": 16,
                "soil": {
                    "surface_variation": 4,
                    "dirt_depth": 150,
                    "dirt_variation": 3
                },
                "ore": {
                    "attempts_per_chunk": 2,
                    "cluster_min": 6,
                    "cluster_max": 18,
                    "min_depth": 20,
                    "max_depth": 220
                },
                "food": {
                    "attempts_per_chunk": 3,
                    "cluster_min": 6,
                    "cluster_max": 14,
                    "min_depth": 0,
                    "max_depth": 50
                },
                "stone_pockets": {
                    "attempts_per_chunk": 60.0,
                    "cluster_min": 1,
                    "cluster_max": 60,
                    "min_depth": 0,
                    "max_depth": 235,
                    "depth_gain": 0.00002
                }
            }
        }
    })
}

fn merge_with_default_config(config: Value) -> Value {
    let mut merged = default_config();
    merge_config_value(&mut merged, migrate_legacy_config(config));
    merged
}

fn migrate_legacy_config(mut config: Value) -> Value {
    let Some(root) = config.as_object_mut() else {
        return config;
    };

    let terrain = root.remove("terrain");
    let ore = root.remove("ore");
    let food = root.remove("food");
    let chunk_width = root
        .get_mut("world")
        .and_then(Value::as_object_mut)
        .and_then(|world| world.remove("chunk_width"));

    let _ = root;

    if let Some(terrain) = terrain {
        let _ = set_config_path(&mut config, "world.gen_params.soil", terrain);
    }
    if let Some(ore) = ore {
        let _ = set_config_path(&mut config, "world.gen_params.ore", ore);
    }
    if let Some(food) = food {
        let _ = set_config_path(&mut config, "world.gen_params.food", food);
    }
    if let Some(chunk_width) = chunk_width {
        let _ = set_config_path(&mut config, "world.gen_params.chunk_width", chunk_width);
    }

    config
}

fn default_inventory() -> HashMap<String, u16> {
    HashMap::from([
        ("dirt".to_string(), 8),
        ("ore".to_string(), 0),
        ("stone".to_string(), 0),
        ("food".to_string(), 0),
    ])
}

fn default_npcs(world: &World) -> Vec<NpcAnt> {
    let surface_1 = world.spawn_y_for_column(20).saturating_add(1);
    let surface_2 = world.spawn_y_for_column(120).saturating_add(3);
    vec![
        NpcAnt {
            id: 1,
            pos: Position {
                x: 20,
                y: surface_1.min(world.height() - 2),
            },
        },
        NpcAnt {
            id: 2,
            pos: Position {
                x: 120,
                y: surface_2.min(world.height() - 2),
            },
        },
    ]
}

fn apply_cluster_pass(
    world: &mut World,
    seed: u64,
    chunk_width: i32,
    tile: Tile,
    deposit: &DepositConfig,
    replaceable: &[Tile],
    surface_heights: &[i32],
) {
    if deposit.attempts_per_chunk == 0 || deposit.max_depth < deposit.min_depth {
        return;
    }

    let chunks = (world.width() + chunk_width - 1) / chunk_width;
    for chunk_x in 0..chunks {
        let chunk_seed = seed ^ mix_u64(chunk_x as u64);
        let mut rng = StdRng::seed_from_u64(chunk_seed);
        let chunk_start = chunk_x * chunk_width;
        let chunk_end = (chunk_start + chunk_width).min(world.width());

        for _ in 0..deposit.attempts_per_chunk {
            let center_x = rng.random_range(chunk_start..chunk_end);
            let surface_y = surface_heights[center_x as usize];
            let min_y = (surface_y + deposit.min_depth).clamp(0, world.height() - 2);
            let max_y = (surface_y + deposit.max_depth).clamp(0, world.height() - 2);
            if min_y > max_y {
                continue;
            }

            let center = Position {
                x: center_x,
                y: rng.random_range(min_y..=max_y),
            };
            let cluster_max = deposit.cluster_max.max(deposit.cluster_min);
            let target_size = rng.random_range(deposit.cluster_min..=cluster_max);
            grow_cluster(world, &mut rng, center, target_size, tile, replaceable, min_y, max_y);
        }
    }
}

fn apply_depth_scaled_cluster_pass(
    world: &mut World,
    seed: u64,
    chunk_width: i32,
    tile: Tile,
    deposit: &DepthScaledDepositConfig,
    replaceable: &[Tile],
    surface_heights: &[i32],
) {
    if deposit.attempts_per_chunk <= 0.0 || deposit.max_depth < deposit.min_depth {
        return;
    }

    let chunks = (world.width() + chunk_width - 1) / chunk_width;
    for chunk_x in 0..chunks {
        let chunk_seed = seed ^ mix_u64(chunk_x as u64);
        let mut rng = StdRng::seed_from_u64(chunk_seed);
        let chunk_start = chunk_x * chunk_width;
        let chunk_end = (chunk_start + chunk_width).min(world.width());

        let mut attempts = deposit.attempts_per_chunk.floor() as i32;
        let fractional = deposit.attempts_per_chunk.fract();
        if rng.random::<f64>() < fractional {
            attempts += 1;
        }

        for _ in 0..attempts.max(1) {
            let center_x = rng.random_range(chunk_start..chunk_end);
            let surface_y = surface_heights[center_x as usize];
            let min_y = (surface_y + deposit.min_depth).clamp(0, world.height() - 2);
            let max_y = (surface_y + deposit.max_depth).clamp(0, world.height() - 2);
            if min_y > max_y {
                continue;
            }

            let center_y = rng.random_range(min_y..=max_y);
            let depth = center_y - surface_y;
            let depth_span = (deposit.max_depth - deposit.min_depth).max(1);
            let depth_factor = ((depth - deposit.min_depth).max(0) as f64 / f64::from(depth_span))
                .clamp(0.0, 1.0)
                .powf(deposit.depth_gain);

            if rng.random::<f64>() > depth_factor {
                continue;
            }

            let cluster_max = deposit.cluster_max.max(deposit.cluster_min);
            let scaled_max = deposit.cluster_min
                + ((cluster_max - deposit.cluster_min) as f64 * depth_factor).round() as i32;
            let target_size = rng.random_range(deposit.cluster_min..=scaled_max.max(deposit.cluster_min));
            grow_cluster(
                world,
                &mut rng,
                Position {
                    x: center_x,
                    y: center_y,
                },
                target_size,
                tile,
                replaceable,
                min_y,
                max_y,
            );
        }
    }
}

fn grow_cluster(
    world: &mut World,
    rng: &mut StdRng,
    center: Position,
    target_size: i32,
    tile: Tile,
    replaceable: &[Tile],
    min_y: i32,
    max_y: i32,
) {
    let mut frontier = vec![center];
    let mut visited = HashSet::new();
    let mut placed = 0;

    while placed < target_size && !frontier.is_empty() {
        let index = rng.random_range(0..frontier.len());
        let pos = frontier.swap_remove(index);
        if !visited.insert(pos) || !world.in_bounds(pos) || pos.y < min_y || pos.y > max_y {
            continue;
        }

        if let Some(existing) = world.tile(pos) {
            if replaceable.contains(&existing) {
                world.set_tile(pos, tile);
                placed += 1;
            }
        }

        for next in [
            pos.offset(1, 0),
            pos.offset(-1, 0),
            pos.offset(0, 1),
            pos.offset(0, -1),
        ] {
            if rng.random::<f64>() < 0.78 {
                frontier.push(next);
            }
        }
    }
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

fn config_i32(root: &Value, path: &str, default: i32) -> i32 {
    let mut current = root;
    for segment in path.split('.').filter(|segment| !segment.trim().is_empty()) {
        let Some(next) = current.get(segment) else {
            return default;
        };
        current = next;
    }
    current
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(default)
}

fn config_u64(root: &Value, path: &str, default: u64) -> u64 {
    let mut current = root;
    for segment in path.split('.').filter(|segment| !segment.trim().is_empty()) {
        let Some(next) = current.get(segment) else {
            return default;
        };
        current = next;
    }
    current.as_u64().unwrap_or(default)
}

fn merge_config_value(target: &mut Value, incoming: Value) {
    match (target, incoming) {
        (Value::Object(target_map), Value::Object(incoming_map)) => {
            for (key, value) in incoming_map {
                match target_map.get_mut(&key) {
                    Some(existing) => merge_config_value(existing, value),
                    None => {
                        target_map.insert(key, value);
                    }
                }
            }
        }
        (target, incoming) => {
            *target = incoming;
        }
    }
}

fn nearest_target(origin: Position, positions: &[Position]) -> Option<Position> {
    positions
        .iter()
        .copied()
        .min_by_key(|pos| (pos.x - origin.x).abs() + (pos.y - origin.y).abs())
}

fn fbm_1d(seed: u64, x: f64, octaves: u32) -> f64 {
    let mut total = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut norm = 0.0;

    for octave in 0..octaves {
        total += value_noise_1d(seed ^ mix_u64(octave as u64), x * frequency) * amplitude;
        norm += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }

    if norm == 0.0 { 0.0 } else { total / norm }
}

fn value_noise_1d(seed: u64, x: f64) -> f64 {
    let x0 = x.floor() as i64;
    let x1 = x0 + 1;
    let t = x - x0 as f64;
    let v0 = random_unit(seed, x0 as u64);
    let v1 = random_unit(seed, x1 as u64);
    lerp(v0, v1, smoothstep(t))
}

fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn random_unit(seed: u64, value: u64) -> f64 {
    let mixed = mix_u64(seed ^ value);
    (mixed as f64 / u64::MAX as f64) * 2.0 - 1.0
}

fn mix_u64(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
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
