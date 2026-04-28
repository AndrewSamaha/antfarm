mod player_actions;
mod simulation;

use rand::{SeedableRng, rngs::StdRng};
use serde_json::Value;
use std::collections::HashMap;

use crate::{
    art::find_ascii_art_asset,
    config::{
        config_f64, config_u16, config_u64, default_server_config, merge_config,
        merge_with_default_config,
        set_config_path,
    },
    constants::{
        DEFAULT_WORLD_SEED, DEFAULT_WORLD_SNAPSHOT_INTERVAL_SECONDS, WORLD_WIDTH,
    },
    inventory::default_inventory,
    npc::default_npcs_with_count,
    npc_debug::NpcDebugEvent,
    pheromones::{PheromoneChannel, PheromoneGrid, PheromoneMap},
    protocol::{DigProgress, PatchFrame, PlacedArt, Snapshot, TileUpdate},
    types::{Facing, NpcAnt, Player, Position, Tile},
    world::World,
};

#[derive(Debug, Clone)]
pub struct GameState {
    pub tick: u64,
    pub world: World,
    pub pheromones: PheromoneGrid,
    pub players: HashMap<u8, Player>,
    pub npcs: Vec<NpcAnt>,
    pub placed_art: Vec<PlacedArt>,
    pub event_log: Vec<String>,
    pub config: Value,
    pub simulation_paused: bool,
    pub found_food_count: u64,
    pub delivered_food_count: u64,
    pub egg_laid_count: u64,
    pub egg_hatched_count: u64,
    npc_debug_enabled: bool,
    npc_debug_events: Vec<NpcDebugEvent>,
    dig_progress: HashMap<u8, DigProgress>,
    dirty_tiles: HashMap<Position, Tile>,
    players_dirty: bool,
    npcs_dirty: bool,
    placed_art_dirty: bool,
    event_log_dirty: bool,
    config_dirty: bool,
    simulation_paused_dirty: bool,
    rng: StdRng,
    next_player_id: u8,
    next_hive_id: u16,
    next_npc_id: u16,
}

impl GameState {
    pub fn new() -> Self {
        Self::from_config(default_server_config())
    }

    pub fn from_config(config: Value) -> Self {
        let config = merge_with_default_config(config);
        let seed = config_u64(&config, "world.seed", DEFAULT_WORLD_SEED);
        let world = World::generate(seed, WORLD_WIDTH, &config);
        let pheromones = PheromoneGrid::empty(world.width(), world.height());
        let ambient_worker_count = config_u16(&config, "colony.ambient_worker_count", 2);
        let npcs = default_npcs_with_count(&world, ambient_worker_count);
        let next_npc_id = ambient_worker_count.saturating_add(1);

        Self {
            tick: 0,
            npcs,
            world,
            pheromones,
            players: HashMap::new(),
            event_log: vec!["Server booted ant colony".to_string()],
            placed_art: Vec::new(),
            config,
            simulation_paused: false,
            found_food_count: 0,
            delivered_food_count: 0,
            egg_laid_count: 0,
            egg_hatched_count: 0,
            npc_debug_enabled: false,
            npc_debug_events: Vec::new(),
            dig_progress: HashMap::new(),
            dirty_tiles: HashMap::new(),
            players_dirty: true,
            npcs_dirty: true,
            placed_art_dirty: true,
            event_log_dirty: true,
            config_dirty: true,
            simulation_paused_dirty: true,
            rng: StdRng::seed_from_u64(seed ^ 0xAB_CD_EF),
            next_player_id: 1,
            next_hive_id: 1,
            next_npc_id,
        }
    }

    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        let config = merge_with_default_config(snapshot.config);
        let seed = config_u64(&config, "world.seed", DEFAULT_WORLD_SEED);
        let next_hive_id = snapshot
            .placed_art
            .iter()
            .filter_map(|placed| placed.hive_id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let next_npc_id = snapshot
            .npcs
            .iter()
            .map(|npc| npc.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let pheromones = PheromoneGrid::empty(snapshot.world.width(), snapshot.world.height());
        Self {
            tick: snapshot.tick,
            world: snapshot.world,
            pheromones,
            players: HashMap::new(),
            npcs: snapshot.npcs,
            placed_art: snapshot.placed_art,
            event_log: vec!["Server restored world snapshot".to_string()],
            config,
            simulation_paused: snapshot.simulation_paused,
            found_food_count: 0,
            delivered_food_count: 0,
            egg_laid_count: 0,
            egg_hatched_count: 0,
            npc_debug_enabled: false,
            npc_debug_events: Vec::new(),
            dig_progress: HashMap::new(),
            dirty_tiles: HashMap::new(),
            players_dirty: true,
            npcs_dirty: true,
            placed_art_dirty: true,
            event_log_dirty: true,
            config_dirty: true,
            simulation_paused_dirty: true,
            rng: StdRng::seed_from_u64(seed ^ 0xAB_CD_EF),
            next_player_id: 1,
            next_hive_id,
            next_npc_id,
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
            placed_art: self.placed_art.clone(),
            event_log: self.event_log.clone(),
            config: self.config.clone(),
            simulation_paused: self.simulation_paused,
        }
    }

    pub fn pheromone_map(&self, hive_id: u16, channel: PheromoneChannel) -> PheromoneMap {
        self.pheromones.export_map(hive_id, channel)
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

    pub fn apply_config_override(&mut self, override_config: Value) {
        self.config = merge_with_default_config(merge_config(self.config.clone(), override_config));
        let seed = config_u64(&self.config, "world.seed", DEFAULT_WORLD_SEED);
        self.rng = StdRng::seed_from_u64(seed ^ 0xAB_CD_EF);
        self.config_dirty = true;
    }

    pub fn world_reset(&mut self, seed: Option<u64>) {
        let paused = self.simulation_paused;
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
        self.simulation_paused = paused;
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
                    hive_id: None,
                    inventory: default_inventory(),
                },
            );
        }

        self.players_dirty = true;
        self.simulation_paused_dirty = true;
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

    pub fn set_simulation_paused(&mut self, paused: bool) {
        if self.simulation_paused == paused {
            return;
        }
        self.simulation_paused = paused;
        self.simulation_paused_dirty = true;
        if paused {
            self.push_event("Simulation paused".to_string());
        } else {
            self.push_event("Simulation resumed".to_string());
        }
    }

    pub fn set_npc_debug_enabled(&mut self, enabled: bool) {
        self.npc_debug_enabled = enabled;
    }

    pub fn npc_debug_enabled(&self) -> bool {
        self.npc_debug_enabled
    }

    pub fn take_npc_debug_events(&mut self) -> Vec<NpcDebugEvent> {
        std::mem::take(&mut self.npc_debug_events)
    }

    pub fn push_server_event(&mut self, event: impl Into<String>) {
        self.push_event(event.into());
    }

    pub fn take_patch(&mut self) -> Option<PatchFrame> {
        if self.dirty_tiles.is_empty()
            && !self.players_dirty
            && !self.npcs_dirty
            && !self.placed_art_dirty
            && !self.event_log_dirty
            && !self.config_dirty
            && !self.simulation_paused_dirty
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
        let placed_art = self.placed_art_dirty.then(|| self.placed_art.clone());
        let event_log = self.event_log_dirty.then(|| self.event_log.clone());
        let config = self.config_dirty.then(|| self.config.clone());
        let simulation_paused = self.simulation_paused_dirty.then_some(self.simulation_paused);

        self.players_dirty = false;
        self.npcs_dirty = false;
        self.placed_art_dirty = false;
        self.event_log_dirty = false;
        self.config_dirty = false;
        self.simulation_paused_dirty = false;

        Some(PatchFrame {
            tick: self.tick,
            tiles,
            players,
            npcs,
            placed_art,
            event_log,
            config,
            simulation_paused,
        })
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

    fn push_npc_debug_event(&mut self, event: NpcDebugEvent) {
        if self.npc_debug_enabled {
            self.npc_debug_events.push(event);
        }
    }

    fn set_world_tile(&mut self, pos: Position, tile: Tile) {
        if self.world.tile(pos) == Some(tile) {
            return;
        }
        if self.world.set_tile(pos, tile) {
            self.dirty_tiles.insert(pos, tile);
        }
    }

    fn art_occupies_cell(&self, pos: Position) -> bool {
        self.placed_art.iter().any(|placed| {
            let Some(asset) = find_ascii_art_asset(&placed.asset_id) else {
                return false;
            };
            let local_x = pos.x - placed.pos.x;
            let local_y = pos.y - placed.pos.y;
            asset.glyph_pair_at_world(local_x, local_y).is_some()
        })
    }
}
