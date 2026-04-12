use rand::{Rng, SeedableRng, rngs::StdRng};
use serde_json::Value;
use std::collections::HashMap;

use crate::{
    config::{config_f64, config_u64, default_server_config, merge_with_default_config, set_config_path},
    constants::{
        DEFAULT_SOIL_SETTLE_FREQUENCY, DEFAULT_WORLD_SEED,
        DEFAULT_WORLD_SNAPSHOT_INTERVAL_SECONDS, MAX_PLAYERS, STONE_DIG_STEPS, WORLD_WIDTH,
    },
    protocol::{Action, DigProgress, PatchFrame, PlaceMaterial, Snapshot, TileUpdate},
    types::{Facing, MoveDir, NpcAnt, Player, Position, Tile},
    world::World,
};

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

    pub fn add_player(
        &mut self,
        name: String,
        restored_player: Option<Player>,
    ) -> Result<(u8, Snapshot), String> {
        if self.players.len() >= MAX_PLAYERS {
            return Err(format!("Room full: max {} players", MAX_PLAYERS));
        }

        let player_id = self.next_player_id;
        self.next_player_id = self.next_player_id.saturating_add(1);

        let spawn_x = (8 + self.players.len() as i32 * 6).min(self.world.width() - 2);
        let was_restored = restored_player.is_some();
        let mut player = restored_player.unwrap_or_else(|| Player {
            id: player_id,
            name: name.clone(),
            pos: Position {
                x: spawn_x,
                y: self.world.spawn_y_for_column(spawn_x),
            },
            facing: Facing::Right,
            inventory: default_inventory(),
        });
        player.id = player_id;
        player.name = name.clone();
        if !self.world.in_bounds(player.pos) || self.occupied_by_actor(player.pos) {
            player.pos = Position {
                x: spawn_x,
                y: self.world.spawn_y_for_column(spawn_x),
            };
        }

        self.players.insert(player_id, player);
        self.players_dirty = true;
        if was_restored {
            self.push_event(format!("{name} rejoined as ant {player_id}"));
        } else {
            self.push_event(format!("{name} joined as ant {player_id}"));
        }
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

fn nearest_target(origin: Position, positions: &[Position]) -> Option<Position> {
    positions
        .iter()
        .copied()
        .min_by_key(|pos| (pos.x - origin.x).abs() + (pos.y - origin.y).abs())
}
