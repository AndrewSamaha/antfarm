mod player_actions;
mod simulation;

use rand::{SeedableRng, rngs::StdRng};
use serde_json::Value;
use std::collections::HashMap;

use crate::{
    art::find_ascii_art_asset,
    config::{
        config_f64, config_u16, config_u64, default_server_config, merge_config,
        merge_with_default_config, set_config_path,
    },
    constants::{DEFAULT_WORLD_SEED, DEFAULT_WORLD_SNAPSHOT_INTERVAL_SECONDS, WORLD_WIDTH},
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

    pub fn from_replay_snapshot(snapshot: Snapshot) -> Self {
        let config = merge_with_default_config(snapshot.config.clone());
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
        let mut players = HashMap::new();
        let next_player_id = snapshot
            .players
            .iter()
            .map(|player| player.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        for player in snapshot.players {
            players.insert(player.id, player);
        }
        let pheromones = PheromoneGrid::empty(snapshot.world.width(), snapshot.world.height());
        Self {
            tick: snapshot.tick,
            world: snapshot.world,
            pheromones,
            players,
            npcs: snapshot.npcs,
            placed_art: snapshot.placed_art,
            event_log: snapshot.event_log,
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
            next_player_id,
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

    pub fn final_snapshot_hash_hex(&self) -> Result<String, serde_json::Error> {
        self.snapshot().deterministic_hash_hex()
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
        let simulation_paused = self
            .simulation_paused_dirty
            .then_some(self.simulation_paused);

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

#[cfg(test)]
mod tests {
    use super::{GameState, simulation::*};
    use crate::{
        art::find_ascii_art_asset,
        config::set_config_path,
        inventory::default_npc_inventory,
        pheromones::AntBehaviorState,
        protocol::PlacedArt,
        replay::ReplayArtifact,
        types::{
            DEFAULT_WORKER_ROLE_PATH, NpcAnt, NpcKind, Position, QueenChamberGrowthMode, Tile,
        },
    };
    use rand::{SeedableRng, rngs::StdRng};
    use serde_json::json;
    use std::{fs, path::PathBuf};

    #[test]
    fn headless_simulation_is_deterministic_for_same_seed() {
        let config = json!({
            "world": { "seed": 12345 },
            "soil": {
                "settle_frequency": 0.05,
                "plant_growth_frequency": 0.01,
                "vertical_growth_multiple": 2.0
            },
            "colony": {
                "ambient_worker_count": 0
            }
        });

        let mut first = GameState::from_config(config.clone());
        let mut second = GameState::from_config(config);
        seed_test_colony(&mut first);
        seed_test_colony(&mut second);
        let initial_snapshot = first.snapshot();
        let initial_first_hash = initial_snapshot
            .deterministic_hash_hex()
            .expect("hash initial first snapshot");
        let initial_second_hash = second
            .final_snapshot_hash_hex()
            .expect("hash initial second snapshot");
        let mut first_hashes = Vec::new();
        let mut second_hashes = Vec::new();

        for _ in 0..400 {
            first.tick();
            second.tick();
            first_hashes.push(
                first
                    .final_snapshot_hash_hex()
                    .expect("hash first snapshot"),
            );
            second_hashes.push(
                second
                    .final_snapshot_hash_hex()
                    .expect("hash second snapshot"),
            );
        }

        assert_eq!(initial_first_hash, initial_second_hash);
        assert_eq!(first_hashes, second_hashes);
        assert_ne!(
            initial_first_hash,
            first_hashes
                .last()
                .expect("simulation should produce at least one snapshot hash")
                .as_str()
        );

        let replay_artifact = ReplayArtifact::new(
            initial_snapshot.clone(),
            400,
            first_hashes
                .last()
                .expect("simulation should produce at least one snapshot hash")
                .clone(),
            json!({
                "source": "headless_determinism_test",
                "ticks": 400,
            }),
        )
        .expect("build replay artifact");
        let replay_verification = replay_artifact.replay().expect("replay artifact");
        assert!(replay_verification.matches_expected);

        let replay_dir = replay_artifact_dir();
        fs::create_dir_all(&replay_dir).expect("create replay artifact dir");
        fs::write(
            replay_dir.join("replay.json"),
            serde_json::to_vec_pretty(&replay_artifact).expect("serialize replay artifact"),
        )
        .expect("write replay artifact");
    }

    #[test]
    fn hatched_workers_fill_weighted_roles_and_non_foragers_stay_idle() {
        let mut config = json!({
            "world": { "seed": 7 },
            "colony": {
                "ambient_worker_count": 0,
                "minimum_delay_to_hatch": 1
            }
        });
        set_config_path(&mut config, "queen.egg_laying_cooldown_ticks", json!(10))
            .expect("set queen laying cooldown");
        let mut game = GameState::from_config(config);
        seed_test_colony(&mut game);
        game.set_queen_eggs(4).expect("seed four eggs");

        while game
            .npcs
            .iter()
            .filter(|npc| npc.kind == NpcKind::Worker && npc.hive_id.is_some())
            .count()
            < 4
        {
            game.tick();
        }

        let food_gatherers = game
            .npcs
            .iter()
            .filter(|npc| npc.kind == NpcKind::Worker)
            .filter(|npc| {
                npc.role.as_deref().unwrap_or(DEFAULT_WORKER_ROLE_PATH) == DEFAULT_WORKER_ROLE_PATH
            })
            .count();
        let queen_chamber_workers: Vec<_> = game
            .npcs
            .iter()
            .filter(|npc| npc.kind == NpcKind::Worker)
            .filter(|npc| npc.role.as_deref() == Some("hive_maintenance.queen_chamber"))
            .cloned()
            .collect();

        assert_eq!(food_gatherers, 3);
        assert_eq!(queen_chamber_workers.len(), 1);

        let maintenance_worker = queen_chamber_workers
            .first()
            .expect("maintenance worker should exist");
        let maintenance_id = maintenance_worker.id;

        for _ in 0..5 {
            game.tick();
        }

        let maintenance_worker = game
            .npcs
            .iter()
            .find(|npc| npc.id == maintenance_id)
            .expect("maintenance worker should still exist");
        assert_eq!(maintenance_worker.behavior, AntBehaviorState::Idle);
        assert_eq!(
            maintenance_worker.role.as_deref(),
            Some("hive_maintenance.queen_chamber")
        );
    }

    #[test]
    fn queen_chamber_workers_move_out_to_the_ring() {
        let mut config = json!({
            "world": { "seed": 11 },
            "colony": {
                "ambient_worker_count": 0,
                "roles": {
                    "hive_maintenance": {
                        "queen_chamber": {
                            "radius_x": 20,
                            "radius_y": 15
                        }
                    }
                }
            }
        });
        set_config_path(&mut config, "soil.settle_frequency", json!(0.0))
            .expect("disable settling");
        let mut game = GameState::from_config(config);
        seed_test_colony(&mut game);
        let queen = game
            .npcs
            .iter()
            .find(|npc| npc.kind == NpcKind::Queen)
            .cloned()
            .expect("queen should exist");
        game.dig_area_at(queen.pos, 90, 90, None)
            .expect("open queen chamber ring area");
        if let Some(queen_mut) = game
            .npcs
            .iter_mut()
            .find(|npc| npc.kind == NpcKind::Queen && npc.id == queen.id)
        {
            queen_mut.food = 0;
        }
        let worker_id = game.next_npc_id;
        game.next_npc_id = game.next_npc_id.saturating_add(1);
        game.npcs.push(NpcAnt {
            id: worker_id,
            pos: Position {
                x: queen.pos.x + 4,
                y: queen.pos.y,
            },
            inventory: default_npc_inventory(),
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 0,
            hive_id: queen.hive_id,
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
            chamber_radius_x: Some(20),
            chamber_radius_y: Some(15),
            chamber_anchor: None,
            chamber_has_left_anchor: false,
            chamber_growth_mode: QueenChamberGrowthMode::Outward,
        });

        for _ in 0..40 {
            game.tick();
        }

        let worker = game
            .npcs
            .iter()
            .find(|npc| npc.id == worker_id)
            .expect("worker should still exist");
        assert!(is_on_queen_chamber_oval_perimeter(
            &game, worker.pos, queen.pos, 20, 15
        ));
    }

    #[test]
    fn queen_chamber_growth_mode_randomizer_and_initial_radii_support_both_directions() {
        let mut saw_outward = false;
        let mut saw_inward = false;

        for seed in 0..32_u64 {
            let mut rng = StdRng::seed_from_u64(seed ^ 0xAB_CD_EF);
            match random_queen_chamber_growth_mode(&mut rng) {
                QueenChamberGrowthMode::Outward => {
                    saw_outward = true;
                    assert_eq!(
                        queen_chamber_initial_radii_for_mode(
                            QueenChamberGrowthMode::Outward,
                            20,
                            15,
                        ),
                        (2, 2)
                    );
                }
                QueenChamberGrowthMode::Inward => {
                    saw_inward = true;
                    assert_eq!(
                        queen_chamber_initial_radii_for_mode(
                            QueenChamberGrowthMode::Inward,
                            20,
                            15,
                        ),
                        (20, 15)
                    );
                }
            }
        }

        assert!(saw_outward, "expected at least one outward chamber hatch");
        assert!(saw_inward, "expected at least one inward chamber hatch");
    }

    #[test]
    fn queen_chamber_inward_growth_shrinks_after_a_revolution() {
        let mut config = json!({
            "world": { "seed": 37 },
            "colony": {
                "ambient_worker_count": 0,
                "roles": {
                    "hive_maintenance": {
                        "queen_chamber": {
                            "radius_x": 6,
                            "radius_y": 5
                        }
                    }
                }
            }
        });
        set_config_path(&mut config, "soil.settle_frequency", json!(0.0))
            .expect("disable settling");
        let mut game = GameState::from_config(config);
        seed_test_colony(&mut game);
        let queen = game
            .npcs
            .iter()
            .find(|npc| npc.kind == NpcKind::Queen)
            .cloned()
            .expect("queen should exist");
        game.dig_area_at(queen.pos, 40, 40, None)
            .expect("open queen chamber ring area");
        let start = Position {
            x: queen.pos.x,
            y: queen.pos.y - 5,
        };
        let worker_id = game.next_npc_id;
        game.next_npc_id = game.next_npc_id.saturating_add(1);
        game.npcs.push(NpcAnt {
            id: worker_id,
            pos: start,
            inventory: default_npc_inventory(),
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 0,
            hive_id: queen.hive_id,
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
            chamber_radius_x: Some(6),
            chamber_radius_y: Some(5),
            chamber_anchor: Some(start),
            chamber_has_left_anchor: true,
            chamber_growth_mode: QueenChamberGrowthMode::Inward,
        });

        game.tick();

        let worker = game
            .npcs
            .iter()
            .find(|npc| npc.id == worker_id)
            .expect("worker should still exist");
        assert_eq!(worker.chamber_radius_x, Some(5));
        assert_eq!(worker.chamber_radius_y, Some(4));
    }

    #[test]
    fn queen_chamber_workers_route_clockwise_around_stone() {
        let mut config = json!({
            "world": { "seed": 19 },
            "colony": {
                "ambient_worker_count": 0,
                "roles": {
                    "hive_maintenance": {
                        "queen_chamber": {
                            "radius_x": 20,
                            "radius_y": 15
                        }
                    }
                }
            }
        });
        set_config_path(&mut config, "soil.settle_frequency", json!(0.0))
            .expect("disable settling");
        let mut game = GameState::from_config(config);
        seed_test_colony(&mut game);
        let queen = game
            .npcs
            .iter()
            .find(|npc| npc.kind == NpcKind::Queen)
            .cloned()
            .expect("queen should exist");
        game.dig_area_at(queen.pos, 90, 90, None)
            .expect("open queen chamber ring area");
        if let Some(queen_mut) = game
            .npcs
            .iter_mut()
            .find(|npc| npc.kind == NpcKind::Queen && npc.id == queen.id)
        {
            queen_mut.food = 0;
        }
        let start = Position {
            x: queen.pos.x,
            y: queen.pos.y - 15,
        };
        let worker_id = game.next_npc_id;
        game.next_npc_id = game.next_npc_id.saturating_add(1);
        game.npcs.push(NpcAnt {
            id: worker_id,
            pos: start,
            inventory: default_npc_inventory(),
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 0,
            hive_id: queen.hive_id,
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
            chamber_radius_x: Some(20),
            chamber_radius_y: Some(15),
            chamber_anchor: None,
            chamber_has_left_anchor: false,
            chamber_growth_mode: QueenChamberGrowthMode::Outward,
        });
        game.set_world_tile(start.offset(1, 0), Tile::Stone);

        game.tick();

        let worker = game
            .npcs
            .iter()
            .find(|npc| npc.id == worker_id)
            .expect("worker should still exist");
        assert_ne!(worker.pos, start.offset(1, 0));
        assert_ne!(worker.pos, start);
        assert!(
            worker.pos == start.offset(-1, 0)
                || worker.pos == start.offset(0, -1)
                || worker.pos == start.offset(0, 1)
        );
    }

    #[test]
    fn queen_chamber_workers_move_through_dirt_in_one_tick() {
        let mut config = json!({
            "world": { "seed": 29 },
            "colony": {
                "ambient_worker_count": 0,
                "roles": {
                    "hive_maintenance": {
                        "queen_chamber": {
                            "radius_x": 20,
                            "radius_y": 15
                        }
                    }
                }
            }
        });
        set_config_path(&mut config, "soil.settle_frequency", json!(0.0))
            .expect("disable settling");
        let mut game = GameState::from_config(config);
        seed_test_colony(&mut game);
        let queen = game
            .npcs
            .iter()
            .find(|npc| npc.kind == NpcKind::Queen)
            .cloned()
            .expect("queen should exist");
        game.dig_area_at(queen.pos, 90, 90, None)
            .expect("open queen chamber ring area");
        let start = Position {
            x: queen.pos.x,
            y: queen.pos.y - 14,
        };
        let target = start.offset(0, -1);
        game.set_world_tile(target, Tile::Dirt);
        game.set_world_tile(start.offset(-1, 0), Tile::Stone);
        game.set_world_tile(start.offset(1, 0), Tile::Stone);
        game.set_world_tile(start.offset(0, 1), Tile::Stone);
        let worker_id = game.next_npc_id;
        game.next_npc_id = game.next_npc_id.saturating_add(1);
        game.npcs.push(NpcAnt {
            id: worker_id,
            pos: start,
            inventory: default_npc_inventory(),
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 0,
            hive_id: queen.hive_id,
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
            chamber_radius_x: Some(20),
            chamber_radius_y: Some(15),
            chamber_anchor: None,
            chamber_has_left_anchor: false,
            chamber_growth_mode: QueenChamberGrowthMode::Outward,
        });

        game.tick();

        let worker = game
            .npcs
            .iter()
            .find(|npc| npc.id == worker_id)
            .expect("worker should still exist");
        assert_eq!(worker.pos, target);
        assert_eq!(game.world.tile(target), Some(Tile::Empty));
    }

    #[test]
    fn queen_chamber_workers_move_through_food_in_one_tick() {
        let mut config = json!({
            "world": { "seed": 33 },
            "colony": {
                "ambient_worker_count": 0,
                "roles": {
                    "hive_maintenance": {
                        "queen_chamber": {
                            "radius_x": 20,
                            "radius_y": 15
                        }
                    }
                }
            }
        });
        set_config_path(&mut config, "soil.settle_frequency", json!(0.0))
            .expect("disable settling");
        let mut game = GameState::from_config(config);
        seed_test_colony(&mut game);
        let queen = game
            .npcs
            .iter()
            .find(|npc| npc.kind == NpcKind::Queen)
            .cloned()
            .expect("queen should exist");
        game.dig_area_at(queen.pos, 90, 90, None)
            .expect("open queen chamber ring area");
        let start = Position {
            x: queen.pos.x,
            y: queen.pos.y - 14,
        };
        let target = start.offset(0, -1);
        game.set_world_tile(target, Tile::Food);
        game.set_world_tile(start.offset(-1, 0), Tile::Stone);
        game.set_world_tile(start.offset(1, 0), Tile::Stone);
        game.set_world_tile(start.offset(0, 1), Tile::Stone);
        let worker_id = game.next_npc_id;
        game.next_npc_id = game.next_npc_id.saturating_add(1);
        game.npcs.push(NpcAnt {
            id: worker_id,
            pos: start,
            inventory: default_npc_inventory(),
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 0,
            hive_id: queen.hive_id,
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
            chamber_radius_x: Some(20),
            chamber_radius_y: Some(15),
            chamber_anchor: None,
            chamber_has_left_anchor: false,
            chamber_growth_mode: QueenChamberGrowthMode::Outward,
        });

        game.tick();

        let worker = game
            .npcs
            .iter()
            .find(|npc| npc.id == worker_id)
            .expect("worker should still exist");
        assert_eq!(worker.pos, target);
        assert_eq!(game.world.tile(target), Some(Tile::Empty));
    }

    #[test]
    fn same_hive_workers_can_move_through_each_other() {
        let mut config = json!({
            "world": { "seed": 31 },
            "colony": { "ambient_worker_count": 0 }
        });
        set_config_path(&mut config, "soil.settle_frequency", json!(0.0))
            .expect("disable settling");
        let mut game = GameState::from_config(config);
        seed_test_colony(&mut game);
        let queen = game
            .npcs
            .iter()
            .find(|npc| npc.kind == NpcKind::Queen)
            .cloned()
            .expect("queen should exist");
        game.dig_area_at(queen.pos, 90, 90, None)
            .expect("open queen chamber ring area");

        let blocker_pos = Position {
            x: queen.pos.x + 9,
            y: queen.pos.y,
        };
        let mover_start = Position {
            x: queen.pos.x + 10,
            y: queen.pos.y,
        };
        game.set_world_tile(mover_start.offset(1, 0), Tile::Stone);
        game.set_world_tile(mover_start.offset(0, -1), Tile::Stone);
        game.set_world_tile(mover_start.offset(0, 1), Tile::Stone);

        let blocker_id = game.next_npc_id;
        game.next_npc_id = game.next_npc_id.saturating_add(1);
        game.npcs.push(NpcAnt {
            id: blocker_id,
            pos: blocker_pos,
            inventory: default_npc_inventory(),
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 0,
            hive_id: queen.hive_id,
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
            role: Some("hive_maintenance".to_string()),
            chamber_radius_x: None,
            chamber_radius_y: None,
            chamber_anchor: None,
            chamber_has_left_anchor: false,
            chamber_growth_mode: QueenChamberGrowthMode::Outward,
        });

        let mover_id = game.next_npc_id;
        game.next_npc_id = game.next_npc_id.saturating_add(1);
        game.npcs.push(NpcAnt {
            id: mover_id,
            pos: mover_start,
            inventory: default_npc_inventory(),
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 1,
            hive_id: queen.hive_id,
            age_ticks: 0,
            behavior: AntBehaviorState::ReturningFood,
            carrying_food: true,
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
            role: Some("food_gatherer".to_string()),
            chamber_radius_x: None,
            chamber_radius_y: None,
            chamber_anchor: None,
            chamber_has_left_anchor: false,
            chamber_growth_mode: QueenChamberGrowthMode::Outward,
        });

        game.tick();

        let mover = game
            .npcs
            .iter()
            .find(|npc| npc.id == mover_id)
            .expect("mover should still exist");
        assert_eq!(mover.pos, blocker_pos);
    }

    fn is_on_queen_chamber_oval_perimeter(
        game: &GameState,
        pos: Position,
        queen_pos: Position,
        radius_x: i32,
        radius_y: i32,
    ) -> bool {
        let dx = f64::from(pos.x - queen_pos.x);
        let dy = f64::from(pos.y - queen_pos.y);
        let ellipse_value = ((dx * dx) / f64::from(radius_x * radius_x))
            + ((dy * dy) / f64::from(radius_y * radius_y));
        if ellipse_value > 1.0 {
            return false;
        }
        [
            pos.offset(-1, 0),
            pos.offset(1, 0),
            pos.offset(0, -1),
            pos.offset(0, 1),
        ]
        .into_iter()
        .any(|neighbor| {
            !game.world.in_bounds(neighbor) || {
                let dx = f64::from(neighbor.x - queen_pos.x);
                let dy = f64::from(neighbor.y - queen_pos.y);
                ((dx * dx) / f64::from(radius_x * radius_x))
                    + ((dy * dy) / f64::from(radius_y * radius_y))
                    > 1.0
            }
        })
    }

    #[test]
    fn put_queen_at_clears_queen_footprint_tiles() {
        let mut game = GameState::from_config(json!({
            "world": { "seed": 23 },
            "colony": { "ambient_worker_count": 0 }
        }));
        let center_x = game.world.width() / 2;
        let queen_center = Position {
            x: center_x,
            y: game.world.spawn_y_for_column(center_x) + 30,
        };
        let asset = find_ascii_art_asset("queen_ant").expect("queen art asset");
        let origin = Position {
            x: queen_center.x - asset.world_anchor_x(),
            y: queen_center.y - asset.anchor_y,
        };

        for row_index in 0..asset.height {
            for col_index in 0..asset.world_width() as usize {
                if asset
                    .glyph_pair_at_world(col_index as i32, row_index as i32)
                    .is_none()
                {
                    continue;
                }
                let pos = Position {
                    x: origin.x + col_index as i32,
                    y: origin.y + row_index as i32,
                };
                game.set_world_tile(pos, Tile::Stone);
            }
        }

        game.put_queen_at(queen_center, None)
            .expect("server queen placement should succeed");

        assert!(
            game.npcs
                .iter()
                .any(|npc| npc.kind == NpcKind::Queen && npc.pos == queen_center)
        );

        for row_index in 0..asset.height {
            for col_index in 0..asset.world_width() as usize {
                if asset
                    .glyph_pair_at_world(col_index as i32, row_index as i32)
                    .is_none()
                {
                    continue;
                }
                let pos = Position {
                    x: origin.x + col_index as i32,
                    y: origin.y + row_index as i32,
                };
                assert_eq!(game.world.tile(pos), Some(Tile::Empty));
            }
        }
    }

    fn replay_artifact_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("repo root")
            .join(".artifacts/tests/replays/headless-determinism/latest")
    }

    fn seed_test_colony(game: &mut GameState) {
        let center_x = game.world.width() / 2;
        let center_y = game.world.spawn_y_for_column(center_x) + 80;
        let chamber_center = Position {
            x: center_x,
            y: center_y,
        };

        game.dig_area_at(chamber_center, 35, 25, None)
            .expect("dig deterministic test chamber");
        let asset = find_ascii_art_asset("queen_ant").expect("queen art asset");
        let hive_id = game.next_hive_id;
        game.next_hive_id = game.next_hive_id.saturating_add(1);
        game.placed_art.push(PlacedArt {
            asset_id: "queen_ant".to_string(),
            pos: Position {
                x: chamber_center.x - asset.world_anchor_x(),
                y: chamber_center.y - asset.anchor_y,
            },
            hive_id: Some(hive_id),
        });
        game.npcs.push(NpcAnt {
            id: game.next_npc_id,
            pos: chamber_center,
            inventory: default_npc_inventory(),
            kind: NpcKind::Queen,
            health: NpcKind::Queen.max_health(),
            food: 0,
            hive_id: Some(hive_id),
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
            role: None,
            chamber_radius_x: None,
            chamber_radius_y: None,
            chamber_anchor: None,
            chamber_has_left_anchor: false,
            chamber_growth_mode: QueenChamberGrowthMode::Outward,
        });
        game.next_npc_id = game.next_npc_id.saturating_add(1);
        game.npcs_dirty = true;
        game.placed_art_dirty = true;
        game.set_queen_eggs(10).expect("seed queen eggs");
        seed_test_food(game, chamber_center);
    }

    fn seed_test_food(game: &mut GameState, queen_pos: Position) {
        for food_center in [
            Position {
                x: queen_pos.x - 23,
                y: queen_pos.y,
            },
            Position {
                x: queen_pos.x + 23,
                y: queen_pos.y,
            },
            Position {
                x: queen_pos.x,
                y: queen_pos.y - 23,
            },
        ] {
            game.put_area_at(food_center, "food", 5, 5, None)
                .expect("seed deterministic test food");
        }
    }
}
