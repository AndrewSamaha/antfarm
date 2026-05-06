use rand::Rng;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};

const RECENT_POSITION_MEMORY_SIZE: usize = 5;
const DEFAULT_QUEEN_CHAMBER_RADIUS_X: i32 = 20;
const DEFAULT_QUEEN_CHAMBER_RADIUS_Y: i32 = 15;

use crate::{
    ant_roles::{
        WorkerRoleDefinition, configured_worker_roles, initialize_worker_role, tick_worker,
    },
    config::{config_f64, config_i32, config_u16, config_u64},
    constants::{
        DEFAULT_PLANT_GROWTH_FREQUENCY, DEFAULT_SOIL_SETTLE_FREQUENCY,
        DEFAULT_SOIL_VERTICAL_GROWTH_MULTIPLE, EGG_HATCH_TICKS, NPC_WORKER_LIFESPAN_TICKS,
        PHEROMONE_DECAY_AMOUNT, PHEROMONE_DECAY_INTERVAL_TICKS, PHEROMONE_MEMORY_RADIUS,
        PHEROMONE_MEMORY_TICKS, QUEEN_EGG_FOOD_COST, QUEEN_HOME_EMIT_PEAK, QUEEN_HOME_EMIT_RADIUS,
        SURFACE_Y, WORKER_FOOD_DEPOSIT_DECAY_STEPS, WORKER_FOOD_DEPOSIT_FLOOR,
        WORKER_FOOD_DEPOSIT_PEAK, WORKER_HOME_DEPOSIT, WORKER_HOME_DEPOSIT_DECAY_STEPS,
    },
    inventory::{add_inventory, default_npc_inventory, inventory_count, remove_inventory},
    npc::nearest_open_tile,
    pheromones::{AntBehaviorState, PheromoneChannel},
    types::{
        DEFAULT_WORKER_ROLE_PATH, MoveDir, NpcAnt, NpcKind, Position, QueenChamberGrowthMode, Tile,
    },
};

use super::GameState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchBehaviorProfile {
    Baseline,
    OutwardBiasV1,
    LocalFieldV1,
    LocalFieldV2,
    OutwardBiasWithLocalFieldV1,
}

impl GameState {
    pub fn tick(&mut self) {
        self.tick += 1;
        self.tick_soil_settling();
        self.tick_plant_growth();
        if self.tick % PHEROMONE_DECAY_INTERVAL_TICKS == 0 {
            self.pheromones.decay_all(PHEROMONE_DECAY_AMOUNT);
        }

        let mut events = Vec::new();
        let mut spawned_npcs = Vec::new();
        for index in 0..self.npcs.len() {
            match self.npcs[index].kind {
                NpcKind::Worker => self.tick_worker(index, &mut events),
                NpcKind::Queen => self.tick_queen(index, &mut spawned_npcs, &mut events),
                NpcKind::Egg => self.tick_egg(index, &mut events),
            }
        }
        let before_retain = self.npcs.len();
        self.npcs.retain(|npc| npc.health > 0);
        if self.npcs.len() != before_retain {
            self.npcs_dirty = true;
        }
        if !spawned_npcs.is_empty() {
            self.npcs.extend(spawned_npcs);
            self.npcs_dirty = true;
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

    fn tick_worker(&mut self, index: usize, events: &mut Vec<String>) {
        if self.npcs[index].hive_id.is_some() {
            self.npcs[index].age_ticks = self.npcs[index].age_ticks.saturating_add(1);
            if self.npcs[index].age_ticks >= self.worker_lifespan_ticks_for(index) {
                let npc_id = self.npcs[index].id;
                let npc_hive = self.npcs[index].hive_id;
                let npc_pos = self.npcs[index].pos;
                self.npcs[index].health = 0;
                self.push_npc_debug_event(crate::NpcDebugEvent {
                    tick: self.tick,
                    npc_id,
                    hive_id: npc_hive,
                    event_type: "died_of_old_age".to_string(),
                    pos: npc_pos,
                    details: json!({
                        "age_ticks": self.npcs[index].age_ticks,
                    }),
                });
                events.push(format!("NPC ant {} died of old age", npc_id));
                return;
            }
        }
        let npc_hive = self.npcs[index].hive_id;
        let queen_pos = npc_hive.and_then(|hive_id| self.find_queen_pos(hive_id));
        tick_worker(self, index, queen_pos, events);
    }

    pub(crate) fn tick_food_gatherer_worker(
        &mut self,
        index: usize,
        queen_pos: Option<Position>,
        events: &mut Vec<String>,
    ) {
        self.tick_worker_memory(index);
        let npc_hive = self.npcs[index].hive_id;
        let npc_pos = self.npcs[index].pos;
        let npc_id = self.npcs[index].id;
        let behavior = self.npcs[index].behavior;
        let home_axes = npc_hive
            .map(|hive_id| self.sample_gradient_axes(npc_pos, hive_id, PheromoneChannel::Home));
        let food_axes = npc_hive
            .map(|hive_id| self.sample_gradient_axes(npc_pos, hive_id, PheromoneChannel::Food));
        let current_food_pheromone = u32::from(
            npc_hive
                .map(|hive_id| {
                    self.pheromones
                        .value(npc_pos, hive_id, PheromoneChannel::Food)
                })
                .unwrap_or(0),
        );
        let search_profile = self.search_behavior_profile();
        let effective_search_profile =
            self.effective_search_behavior_profile(index, search_profile);
        let food_carry_max = self.food_carry_max();
        let carry_visible_food_targets = if behavior == AntBehaviorState::Searching
            && self.npcs[index].carrying_food
            && self.npcs[index].food < food_carry_max
        {
            self.adjacent_food_positions(npc_pos)
        } else {
            Vec::new()
        };
        let search_destination = match (behavior, effective_search_profile, npc_hive) {
            (AntBehaviorState::Searching, SearchBehaviorProfile::LocalFieldV2, Some(hive_id)) => {
                self.refresh_local_field_destination(index, hive_id, queen_pos)
            }
            _ => {
                self.npcs[index].search_destination = None;
                self.npcs[index].search_destination_stuck_ticks = 0;
                None
            }
        };

        if let Some(hive_id) = npc_hive {
            match behavior {
                AntBehaviorState::Searching => {}
                AntBehaviorState::ReturningFood => {
                    let deposit =
                        food_deposit_for_carry_ticks(self.npcs[index].carrying_food_ticks);
                    self.pheromones
                        .deposit(npc_pos, hive_id, PheromoneChannel::Food, deposit);
                    if self.try_deliver_food_to_queen(index) {
                        self.npcs_dirty = true;
                        return;
                    }
                }
                AntBehaviorState::Defending | AntBehaviorState::Idle => {}
            }
        }

        if behavior == AntBehaviorState::Searching && self.try_place_dirt(index, queen_pos, events)
        {
            self.npcs_dirty = true;
            return;
        }

        let directions = [
            npc_pos.offset(-1, 0),
            npc_pos.offset(1, 0),
            npc_pos.offset(0, 1),
            npc_pos.offset(0, -1),
        ];
        let mut raw_candidates = Vec::new();
        for next in directions {
            if next.y <= SURFACE_Y {
                continue;
            }
            if !self.world.in_bounds(next) || self.npc_blocks_movement(next, index) {
                continue;
            }
            let tile = self.world.tile(next);
            let food_pheromone = u32::from(
                npc_hive
                    .map(|hive_id| self.pheromones.value(next, hive_id, PheromoneChannel::Food))
                    .unwrap_or(0),
            );
            let home_pheromone = u32::from(
                npc_hive
                    .map(|hive_id| self.pheromones.value(next, hive_id, PheromoneChannel::Home))
                    .unwrap_or(0),
            );
            let random_score = u32::from(self.rng.random_range(0..=24_u8));
            let tile_bonus = match tile {
                Some(Tile::Food) if !matches!(behavior, AntBehaviorState::ReturningFood) => 80_u32,
                Some(Tile::Food) => 0_u32,
                Some(Tile::Empty) => 12_u32,
                Some(Tile::Dirt) | Some(Tile::Resource) => 2_u32,
                Some(Tile::Stone) | Some(Tile::Bedrock) | None => 0,
            };
            let queen_bias = match (behavior, queen_pos) {
                (AntBehaviorState::ReturningFood, Some(queen_pos)) => {
                    let current = (queen_pos.x - npc_pos.x).abs() + (queen_pos.y - npc_pos.y).abs();
                    let next_dist = (queen_pos.x - next.x).abs() + (queen_pos.y - next.y).abs();
                    if next_dist < current {
                        120_u32
                    } else if next_dist == current {
                        12_u32
                    } else {
                        0_u32
                    }
                }
                _ => 0_u32,
            };
            let memory_bias = u32::from(match behavior {
                AntBehaviorState::Searching => {
                    direction_bias(self.npcs[index].recent_food_dir, npc_pos, next)
                }
                AntBehaviorState::ReturningFood => {
                    direction_bias(self.npcs[index].recent_home_dir, npc_pos, next)
                }
                AntBehaviorState::Defending | AntBehaviorState::Idle => 0,
            });
            let search_profile_bias = match (behavior, effective_search_profile, queen_pos) {
                (
                    AntBehaviorState::Searching,
                    SearchBehaviorProfile::OutwardBiasV1,
                    Some(queen_pos),
                ) => {
                    let current = (queen_pos.x - npc_pos.x).abs() + (queen_pos.y - npc_pos.y).abs();
                    let next_dist = (queen_pos.x - next.x).abs() + (queen_pos.y - next.y).abs();
                    if next_dist > current {
                        80_u32
                    } else if next_dist == current {
                        8_u32
                    } else {
                        0_u32
                    }
                }
                _ => 0_u32,
            };
            let local_field_search = match (behavior, effective_search_profile, npc_hive) {
                (
                    AntBehaviorState::Searching,
                    SearchBehaviorProfile::LocalFieldV1,
                    Some(hive_id),
                )
                | (
                    AntBehaviorState::Searching,
                    SearchBehaviorProfile::LocalFieldV2,
                    Some(hive_id),
                ) => Some(self.local_field_search_bias(next, hive_id)),
                _ => None,
            };
            let destination_bias = match (behavior, effective_search_profile, search_destination) {
                (
                    AntBehaviorState::Searching,
                    SearchBehaviorProfile::LocalFieldV2,
                    Some(destination),
                ) => local_field_destination_bias(npc_pos, next, destination),
                _ => 0_u32,
            };
            let carry_harvest_bias = if !carry_visible_food_targets.is_empty() {
                let current_best = carry_visible_food_targets
                    .iter()
                    .map(|target| manhattan_distance(npc_pos, *target))
                    .min()
                    .unwrap_or(0);
                let next_best = carry_visible_food_targets
                    .iter()
                    .map(|target| manhattan_distance(next, *target))
                    .min()
                    .unwrap_or(current_best);
                if next_best < current_best {
                    160_u32
                } else if next_best == current_best {
                    16_u32
                } else {
                    0_u32
                }
            } else {
                0_u32
            };
            let recent_position_penalty =
                recent_position_penalty(&self.npcs[index].recent_positions, next);
            raw_candidates.push((
                next,
                tile,
                food_pheromone,
                home_pheromone,
                random_score,
                tile_bonus,
                queen_bias,
                memory_bias,
                search_profile_bias,
                local_field_search,
                destination_bias,
                carry_harvest_bias,
                recent_position_penalty,
            ));
        }

        let has_increasing_adjacent_food_signal = matches!(behavior, AntBehaviorState::Searching)
            && raw_candidates
                .iter()
                .any(|(_, _, food_pheromone, _, _, _, _, _, _, _, _, _, _)| {
                    *food_pheromone > current_food_pheromone
                });

        let mut candidates = Vec::new();
        let mut candidate_logs = Vec::new();
        for (
            next,
            tile,
            food_pheromone,
            home_pheromone,
            random_score,
            tile_bonus,
            queen_bias,
            memory_bias,
            search_profile_bias,
            local_field_search,
            destination_bias,
            carry_harvest_bias,
            recent_position_penalty,
        ) in raw_candidates
        {
            let pheromone_score = match behavior {
                AntBehaviorState::Searching
                    if effective_search_profile == SearchBehaviorProfile::LocalFieldV1 =>
                {
                    local_field_search
                        .map(|score| score.visible_food_bonus + score.food_field_score)
                        .unwrap_or(0)
                }
                AntBehaviorState::Searching
                    if effective_search_profile == SearchBehaviorProfile::LocalFieldV2 =>
                {
                    destination_bias
                }
                AntBehaviorState::Searching if has_increasing_adjacent_food_signal => {
                    food_pheromone.saturating_sub(current_food_pheromone)
                }
                AntBehaviorState::Searching => 255_u32.saturating_sub(home_pheromone),
                AntBehaviorState::ReturningFood => home_pheromone,
                AntBehaviorState::Defending | AntBehaviorState::Idle => 0,
            };
            let terrain_penalty = match (behavior, tile) {
                (AntBehaviorState::ReturningFood, Some(Tile::Stone)) => 220_u32,
                (AntBehaviorState::ReturningFood, Some(Tile::Bedrock)) => 260_u32,
                _ => 0_u32,
            };
            let local_field_penalty = local_field_search
                .map(|score| score.home_field_penalty + score.stone_penalty)
                .unwrap_or(0);
            let score = pheromone_score
                + random_score
                + tile_bonus
                + queen_bias
                + memory_bias
                + search_profile_bias
                + carry_harvest_bias;
            let score = score
                .saturating_sub(terrain_penalty + recent_position_penalty + local_field_penalty);
            candidates.push((score, next, tile));
            candidate_logs.push(json!({
                "next": { "x": next.x, "y": next.y },
                "tile": tile.map(tile_name),
                "food_pheromone": food_pheromone,
                "home_pheromone": home_pheromone,
                "pheromone_score": pheromone_score,
                "current_food_pheromone": current_food_pheromone,
                "has_increasing_adjacent_food_signal": has_increasing_adjacent_food_signal,
                "random_score": random_score,
                "tile_bonus": tile_bonus,
                "queen_bias": queen_bias,
                "memory_bias": memory_bias,
                "search_profile": search_behavior_profile_name(effective_search_profile),
                "search_profile_bias": search_profile_bias,
                "search_destination": search_destination.map(|pos| json!({ "x": pos.x, "y": pos.y })),
                "destination_bias": destination_bias,
                "carry_harvest_bias": carry_harvest_bias,
                "local_field_search": local_field_search.map(|score| json!({
                    "visible_food_bonus": score.visible_food_bonus,
                    "visible_food_distance": score.visible_food_distance,
                    "food_field_score": score.food_field_score,
                    "home_field_penalty": score.home_field_penalty,
                    "stone_penalty": score.stone_penalty,
                })),
                "recent_position_penalty": recent_position_penalty,
                "terrain_penalty": terrain_penalty,
                "local_field_penalty": local_field_penalty,
                "score": score,
            }));
        }

        candidates.sort_by_key(|(score, _, _)| std::cmp::Reverse(*score));

        let mut outcome = "blocked".to_string();
        let mut chosen_next = None;
        for (_, next, tile) in candidates {
            match tile {
                Some(Tile::Empty) => {
                    self.npcs[index].pos = next;
                    if matches!(behavior, AntBehaviorState::Searching)
                        && let (Some(hive_id), Some(home_trail_steps)) =
                            (npc_hive, self.npcs[index].home_trail_steps)
                    {
                        let deposit = home_deposit_for_trail_steps(home_trail_steps);
                        if deposit > 0 {
                            self.pheromones
                                .deposit(next, hive_id, PheromoneChannel::Home, deposit);
                            self.npcs[index].home_trail_steps =
                                Some(home_trail_steps.saturating_add(1));
                        } else {
                            self.npcs[index].home_trail_steps = None;
                        }
                    }
                    if matches!(behavior, AntBehaviorState::ReturningFood) {
                        self.npcs[index].carrying_food_ticks =
                            self.npcs[index].carrying_food_ticks.saturating_add(1);
                    }
                    remember_recent_position(&mut self.npcs[index].recent_positions, next);
                    self.update_search_destination_progress(index, npc_pos, Some(next));
                    self.npcs_dirty = true;
                    outcome = "moved".to_string();
                    chosen_next = Some(next);
                    break;
                }
                Some(Tile::Food) if !matches!(behavior, AntBehaviorState::ReturningFood) => {
                    self.set_world_tile(next, Tile::Empty);
                    self.npcs[index].pos = next;
                    let lifespan_bonus = worker_lifespan_bonus(
                        self.npcs[index].age_ticks,
                        self.worker_lifespan_ticks_for(index),
                    );
                    self.npcs[index].age_ticks =
                        self.npcs[index].age_ticks.saturating_sub(lifespan_bonus);
                    let previous_carried = self.npcs[index].food;
                    let carried_after_pickup =
                        previous_carried.saturating_add(1).min(food_carry_max);
                    self.npcs[index].carrying_food = true;
                    self.npcs[index].food = carried_after_pickup;
                    let keep_collecting =
                        carried_after_pickup < food_carry_max && self.adjacent_food_visible(next);
                    self.npcs[index].behavior = if keep_collecting {
                        AntBehaviorState::Searching
                    } else {
                        AntBehaviorState::ReturningFood
                    };
                    self.npcs[index].carrying_food_ticks = 0;
                    self.npcs[index].home_trail_steps = None;
                    self.npcs[index].recent_home_memory_ticks = 0;
                    self.npcs[index].recent_positions.clear();
                    self.npcs[index].search_destination = None;
                    self.npcs[index].search_destination_stuck_ticks = 0;
                    self.found_food_count = self.found_food_count.saturating_add(1);
                    self.npcs_dirty = true;
                    events.push(format!("NPC ant {} found food", npc_id));
                    self.push_npc_debug_event(crate::NpcDebugEvent {
                        tick: self.tick,
                        npc_id,
                        hive_id: npc_hive,
                        event_type: "found_food".to_string(),
                        pos: next,
                        details: json!({
                            "behavior_before": behavior_name(behavior),
                            "behavior_after": behavior_name(self.npcs[index].behavior),
                            "carried_food_before": previous_carried,
                            "carried_food_after": carried_after_pickup,
                            "food_carry_max": food_carry_max,
                            "keep_collecting": keep_collecting,
                            "lifespan_bonus": lifespan_bonus,
                            "age_ticks": self.npcs[index].age_ticks,
                        }),
                    });
                    outcome = "picked_up_food".to_string();
                    chosen_next = Some(next);
                    break;
                }
                Some(Tile::Dirt) | Some(Tile::Resource) => {
                    match tile {
                        Some(Tile::Dirt) => {
                            add_inventory(&mut self.npcs[index].inventory, "dirt", 1)
                        }
                        Some(Tile::Resource) => {
                            add_inventory(&mut self.npcs[index].inventory, "ore", 1)
                        }
                        _ => {}
                    }
                    self.set_world_tile(next, Tile::Empty);
                    events.push(format!(
                        "NPC ant {} tunneled at {},{}",
                        npc_id, next.x, next.y
                    ));
                    self.update_search_destination_progress(index, npc_pos, None);
                    outcome = "tunneled".to_string();
                    chosen_next = Some(next);
                    break;
                }
                Some(Tile::Food) | Some(Tile::Stone) | Some(Tile::Bedrock) | None => {}
            }
        }
        if chosen_next.is_none() {
            self.update_search_destination_progress(index, npc_pos, None);
        }

        self.push_npc_debug_event(crate::NpcDebugEvent {
            tick: self.tick,
            npc_id,
            hive_id: npc_hive,
            event_type: "selected_move".to_string(),
            pos: npc_pos,
            details: json!({
                "behavior": behavior_name(behavior),
                "search_behavior_profile": search_behavior_profile_name(search_profile),
                "effective_search_behavior_profile": search_behavior_profile_name(effective_search_profile),
                "carrying_food": self.npcs[index].carrying_food,
                "carried_food_count": self.npcs[index].food,
                "food_carry_max": food_carry_max,
                "carrying_food_ticks": self.npcs[index].carrying_food_ticks,
                "home_trail_steps": self.npcs[index].home_trail_steps,
                "search_destination": self.npcs[index]
                    .search_destination
                    .map(|pos| json!({ "x": pos.x, "y": pos.y })),
                "search_destination_stuck_ticks": self.npcs[index].search_destination_stuck_ticks,
                "has_delivered_food": self.npcs[index].has_delivered_food,
                "memory": {
                    "home_dir": self.npcs[index].recent_home_dir.map(dir_name),
                    "home_ttl": self.npcs[index].recent_home_memory_ticks,
                    "food_dir": self.npcs[index].recent_food_dir.map(dir_name),
                    "food_ttl": self.npcs[index].recent_food_memory_ticks,
                    "recent_positions": self.npcs[index]
                        .recent_positions
                        .iter()
                        .map(|pos| json!({ "x": pos.x, "y": pos.y }))
                        .collect::<Vec<_>>(),
                },
                "radius_sample": {
                    "home": home_axes.map(axes_json),
                    "food": food_axes.map(axes_json),
                },
                "queen_pos": queen_pos.map(|pos| json!({ "x": pos.x, "y": pos.y })),
                "neighborhood": npc_hive.map(|hive_id| self.local_neighborhood_snapshot(npc_pos, hive_id)),
                "candidates": candidate_logs,
                "chosen_next": chosen_next.map(|pos| json!({ "x": pos.x, "y": pos.y })),
                "outcome": outcome,
            }),
        });
    }

    fn tick_queen(
        &mut self,
        index: usize,
        spawned_npcs: &mut Vec<NpcAnt>,
        events: &mut Vec<String>,
    ) {
        let queen_pos = self.npcs[index].pos;
        let queen_id = self.npcs[index].id;
        let queen_hive_id = self.npcs[index].hive_id;
        if let Some(hive_id) = queen_hive_id {
            self.pheromones.emit_radius(
                queen_pos,
                hive_id,
                PheromoneChannel::Home,
                QUEEN_HOME_EMIT_RADIUS,
                QUEEN_HOME_EMIT_PEAK,
            );
        }
        self.npcs[index].food = self.npcs[index].food.min(NpcKind::Queen.max_food());
        if let Some(last_tick) = self.npcs[index].last_egg_laid_tick
            && self.tick.saturating_sub(last_tick) < self.egg_laying_cooldown_ticks()
        {
            return;
        }
        let egg_food_cost = self.queen_egg_food_cost();
        if self.npcs[index].food < egg_food_cost {
            return;
        }
        let max_workers_per_hive = self.max_workers_per_hive();
        if let (Some(limit), Some(hive_id)) = (max_workers_per_hive, queen_hive_id) {
            let hive_workers = self
                .npcs
                .iter()
                .filter(|npc| {
                    npc.hive_id == Some(hive_id)
                        && matches!(npc.kind, NpcKind::Worker | NpcKind::Egg)
                })
                .count();
            if hive_workers >= limit {
                return;
            }
        }
        let occupied: Vec<_> = self
            .players
            .values()
            .map(|player| player.pos)
            .chain(self.npcs.iter().map(|npc| npc.pos))
            .collect();
        let Some(egg_pos) = nearest_open_tile(&self.world, &occupied, queen_pos) else {
            return;
        };

        self.npcs[index].food = self.npcs[index].food.saturating_sub(egg_food_cost);
        self.npcs[index].last_egg_laid_tick = Some(self.tick);
        self.egg_laid_count = self.egg_laid_count.saturating_add(1);
        spawned_npcs.push(NpcAnt {
            id: self.next_npc_id,
            pos: egg_pos,
            inventory: default_npc_inventory(),
            kind: NpcKind::Egg,
            health: NpcKind::Egg.max_health(),
            food: 0,
            hive_id: queen_hive_id,
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
        self.next_npc_id = self.next_npc_id.saturating_add(1);
        self.npcs_dirty = true;
        events.push(format!(
            "Queen {} laid an egg at {},{}",
            queen_id, egg_pos.x, egg_pos.y
        ));
        self.push_npc_debug_event(crate::NpcDebugEvent {
            tick: self.tick,
            npc_id: queen_id,
            hive_id: queen_hive_id,
            event_type: "egg_laid".to_string(),
            pos: queen_pos,
            details: json!({
                "egg_pos": { "x": egg_pos.x, "y": egg_pos.y },
            }),
        });
    }

    fn tick_egg(&mut self, index: usize, events: &mut Vec<String>) {
        let minimum_delay_to_hatch = self.minimum_delay_to_hatch();
        self.npcs[index].age_ticks = self.npcs[index].age_ticks.saturating_add(1);
        if self.npcs[index].age_ticks < minimum_delay_to_hatch {
            return;
        }
        let egg_hive_id = self.npcs[index].hive_id;
        let queen_index = egg_hive_id.and_then(|hive_id| {
            self.npcs
                .iter()
                .position(|npc| npc.kind == NpcKind::Queen && npc.hive_id == Some(hive_id))
        });
        if let Some(queen_index) = queen_index
            && let Some(last_tick) = self.npcs[queen_index].last_egg_hatched_tick
            && self.tick.saturating_sub(last_tick) < self.egg_hatch_cooldown_ticks()
        {
            return;
        }
        let assigned_role = self.choose_worker_role_for_hatch(egg_hive_id);

        {
            let egg = &mut self.npcs[index];
            egg.kind = NpcKind::Worker;
            egg.health = NpcKind::Worker.max_health();
            egg.food = 0;
            egg.age_ticks = 0;
            egg.behavior = AntBehaviorState::Idle;
            egg.carrying_food = false;
            egg.carrying_food_ticks = 0;
            egg.home_trail_steps = None;
            egg.recent_home_dir = None;
            egg.recent_food_dir = None;
            egg.recent_home_memory_ticks = 0;
            egg.recent_food_memory_ticks = 0;
            egg.recent_positions.clear();
            egg.search_destination = None;
            egg.search_destination_stuck_ticks = 0;
            egg.has_delivered_food = false;
            egg.role = assigned_role.clone();
            egg.chamber_radius_x = None;
            egg.chamber_radius_y = None;
            egg.chamber_anchor = None;
            egg.chamber_has_left_anchor = false;
            egg.chamber_growth_mode = QueenChamberGrowthMode::Outward;
        }
        initialize_worker_role(self, index);
        self.egg_hatched_count = self.egg_hatched_count.saturating_add(1);
        let hatched_id = self.npcs[index].id;
        let hatched_hive_id = self.npcs[index].hive_id;
        let hatched_pos = self.npcs[index].pos;
        let hatched_role = self.npcs[index]
            .role
            .clone()
            .unwrap_or_else(|| DEFAULT_WORKER_ROLE_PATH.to_string());
        if let Some(queen_index) = queen_index {
            self.npcs[queen_index].last_egg_hatched_tick = Some(self.tick);
        }
        self.npcs_dirty = true;
        events.push(format!(
            "Egg {} hatched into a {} worker ant",
            hatched_id, hatched_role
        ));
        self.push_npc_debug_event(crate::NpcDebugEvent {
            tick: self.tick,
            npc_id: hatched_id,
            hive_id: hatched_hive_id,
            event_type: "egg_hatched".to_string(),
            pos: hatched_pos,
            details: json!({
                "role": hatched_role,
                "behavior_after": behavior_name(self.npcs[index].behavior),
            }),
        });
    }

    pub(crate) fn set_worker_idle(&mut self, index: usize) {
        if self.npcs[index].behavior != AntBehaviorState::Idle {
            self.npcs[index].behavior = AntBehaviorState::Idle;
            self.npcs_dirty = true;
        }
    }

    pub(crate) fn mark_npcs_dirty(&mut self) {
        self.npcs_dirty = true;
    }

    pub(crate) fn next_queen_chamber_growth_mode(&mut self) -> QueenChamberGrowthMode {
        crate::ant_roles::queen_chamber::random_queen_chamber_growth_mode(&mut self.rng)
    }

    pub(crate) fn worker_role_path(&self, index: usize) -> &str {
        self.npcs[index]
            .role
            .as_deref()
            .unwrap_or(DEFAULT_WORKER_ROLE_PATH)
    }

    fn choose_worker_role_for_hatch(&self, hive_id: Option<u16>) -> Option<String> {
        let roles = configured_worker_roles(&self.config);
        if roles.is_empty() {
            return Some(DEFAULT_WORKER_ROLE_PATH.to_string());
        }

        let mut role_counts: HashMap<&str, u16> = HashMap::new();
        let mut hive_workers = 0u16;
        for npc in &self.npcs {
            if npc.kind != NpcKind::Worker || npc.health == 0 {
                continue;
            }
            let in_same_hive = match hive_id {
                Some(hive_id) => npc.hive_id == Some(hive_id),
                None => npc.hive_id.is_none(),
            };
            if !in_same_hive {
                continue;
            }
            hive_workers = hive_workers.saturating_add(1);
            let role_path = npc.role.as_deref().unwrap_or(DEFAULT_WORKER_ROLE_PATH);
            let entry = role_counts.entry(role_path).or_insert(0);
            *entry = entry.saturating_add(1);
        }

        let total_after_hatch = hive_workers.saturating_add(1);
        let total_weight: u32 = roles.iter().map(|role| u32::from(role.weight)).sum();
        if total_weight == 0 {
            return Some(DEFAULT_WORKER_ROLE_PATH.to_string());
        }

        let mut best_role: Option<&WorkerRoleDefinition> = None;
        let mut best_deficit = f64::NEG_INFINITY;
        for role in &roles {
            let current_count = f64::from(*role_counts.get(role.path.as_str()).unwrap_or(&0));
            let desired_count =
                f64::from(total_after_hatch) * f64::from(role.weight) / f64::from(total_weight);
            let deficit = desired_count - current_count;
            let should_replace = match best_role {
                None => true,
                Some(_) if deficit > best_deficit => true,
                Some(current_best)
                    if (deficit - best_deficit).abs() < f64::EPSILON
                        && role.weight > current_best.weight =>
                {
                    true
                }
                Some(current_best)
                    if (deficit - best_deficit).abs() < f64::EPSILON
                        && role.weight == current_best.weight
                        && role.path < current_best.path =>
                {
                    true
                }
                _ => false,
            };
            if should_replace {
                best_role = Some(role);
                best_deficit = deficit;
            }
        }

        best_role.map(|role| role.path.clone())
    }

    pub(crate) fn npc_blocks_movement(&self, pos: Position, mover_index: usize) -> bool {
        let mover_hive_id = self.npcs[mover_index].hive_id;
        self.npcs.iter().enumerate().any(|(index, npc)| {
            index != mover_index && npc.pos == pos && !same_hive(mover_hive_id, npc.hive_id)
        })
    }

    fn npc_occupied(&self, pos: Position, ignore_index: Option<usize>) -> bool {
        self.npcs
            .iter()
            .enumerate()
            .any(|(index, npc)| Some(index) != ignore_index && npc.pos == pos)
    }

    fn try_place_dirt(
        &mut self,
        index: usize,
        queen_pos: Option<Position>,
        events: &mut Vec<String>,
    ) -> bool {
        if self.npcs[index].carrying_food
            || inventory_count(&self.npcs[index].inventory, "dirt") == 0
        {
            return false;
        }
        if let Some(last_tick) = self.npcs[index].last_dirt_place_tick
            && self.tick.saturating_sub(last_tick) < self.dirt_place_cooldown_ticks()
        {
            return false;
        }

        let Some(hive_id) = self.npcs[index].hive_id else {
            return false;
        };
        let Some(queen_pos) = queen_pos else {
            return false;
        };
        let npc_pos = self.npcs[index].pos;
        let queen_distance = (queen_pos.x - npc_pos.x).abs() + (queen_pos.y - npc_pos.y).abs();
        if queen_distance <= self.queen_no_fill_radius() {
            return false;
        }

        let candidates = [
            npc_pos.offset(-1, 0),
            npc_pos.offset(1, 0),
            npc_pos.offset(0, 1),
            npc_pos.offset(0, -1),
        ];
        let mut best_target = None;
        let mut best_score = i32::MIN;

        for target in candidates {
            if !self.can_place_dirt_at(index, hive_id, target) {
                continue;
            }
            let solid_neighbors = self.solid_neighbor_count(target);
            if solid_neighbors > best_score {
                best_score = solid_neighbors;
                best_target = Some(target);
            }
        }

        let Some(target) = best_target else {
            return false;
        };

        if !remove_inventory(&mut self.npcs[index].inventory, "dirt", 1) {
            return false;
        }

        self.set_world_tile(target, Tile::Dirt);
        self.npcs[index].last_dirt_place_tick = Some(self.tick);
        let npc_id = self.npcs[index].id;
        events.push(format!(
            "NPC ant {} placed dirt at {},{}",
            npc_id, target.x, target.y
        ));
        self.push_npc_debug_event(crate::NpcDebugEvent {
            tick: self.tick,
            npc_id,
            hive_id: Some(hive_id),
            event_type: "placed_dirt".to_string(),
            pos: self.npcs[index].pos,
            details: json!({
                "target": { "x": target.x, "y": target.y },
                "remaining_dirt": inventory_count(&self.npcs[index].inventory, "dirt"),
                "last_dirt_place_tick": self.npcs[index].last_dirt_place_tick,
            }),
        });
        true
    }

    fn can_place_dirt_at(&self, index: usize, hive_id: u16, target: Position) -> bool {
        if !self.world.in_bounds(target) {
            return false;
        }
        if self.world.tile(target) != Some(Tile::Empty) {
            return false;
        }
        if self.npc_occupied(target, Some(index))
            || self.players.values().any(|player| player.pos == target)
            || self.art_occupies_cell(target)
        {
            return false;
        }
        if !self.pheromone_clear_for_fill(target, hive_id) {
            return false;
        }
        if self.solid_neighbor_count(target) < 3 {
            return false;
        }
        if self.open_cardinal_neighbor_count(target, Some(index)) < 2 {
            return false;
        }
        true
    }

    fn pheromone_clear_for_fill(&self, target: Position, hive_id: u16) -> bool {
        let positions = [
            target,
            target.offset(-1, 0),
            target.offset(1, 0),
            target.offset(0, 1),
            target.offset(0, -1),
        ];
        positions.into_iter().all(|pos| {
            !self.world.in_bounds(pos)
                || (self.pheromones.value(pos, hive_id, PheromoneChannel::Home) == 0
                    && self.pheromones.value(pos, hive_id, PheromoneChannel::Food) == 0)
        })
    }

    fn solid_neighbor_count(&self, target: Position) -> i32 {
        let mut solids = 0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let pos = target.offset(dx, dy);
                if !self.world.in_bounds(pos) {
                    continue;
                }
                if matches!(
                    self.world.tile(pos),
                    Some(Tile::Dirt | Tile::Stone | Tile::Bedrock | Tile::Resource)
                ) {
                    solids += 1;
                }
            }
        }
        solids
    }

    fn open_cardinal_neighbor_count(&self, target: Position, ignore_index: Option<usize>) -> i32 {
        [
            target.offset(-1, 0),
            target.offset(1, 0),
            target.offset(0, 1),
            target.offset(0, -1),
        ]
        .into_iter()
        .filter(|pos| {
            self.world.in_bounds(*pos)
                && self.world.tile(*pos) == Some(Tile::Empty)
                && !self.players.values().any(|player| player.pos == *pos)
                && !self.npc_occupied(*pos, ignore_index)
                && !self.art_occupies_cell(*pos)
        })
        .count() as i32
    }

    fn try_deliver_food_to_queen(&mut self, worker_index: usize) -> bool {
        let Some(hive_id) = self.npcs[worker_index].hive_id else {
            return false;
        };
        let worker_pos = self.npcs[worker_index].pos;
        let queen_index = self.npcs.iter().position(|npc| {
            npc.kind == NpcKind::Queen
                && npc.hive_id == Some(hive_id)
                && (npc.pos.x - worker_pos.x).abs() + (npc.pos.y - worker_pos.y).abs()
                    <= self.queen_delivery_radius()
        });
        let Some(queen_index) = queen_index else {
            return false;
        };
        let delivered_amount = self.npcs[worker_index].food.max(1);

        self.npcs[queen_index].food = self.npcs[queen_index]
            .food
            .saturating_add(delivered_amount)
            .min(NpcKind::Queen.max_food());
        self.npcs[worker_index].carrying_food = false;
        self.npcs[worker_index].carrying_food_ticks = 0;
        self.npcs[worker_index].behavior = AntBehaviorState::Searching;
        self.npcs[worker_index].food = 0;
        self.npcs[worker_index].home_trail_steps = Some(0);
        self.npcs[worker_index].recent_food_memory_ticks = 0;
        self.npcs[worker_index].recent_positions.clear();
        self.npcs[worker_index].search_destination = None;
        self.npcs[worker_index].search_destination_stuck_ticks = 0;
        self.npcs[worker_index].has_delivered_food = true;
        self.delivered_food_count = self
            .delivered_food_count
            .saturating_add(u64::from(delivered_amount));
        self.push_npc_debug_event(crate::NpcDebugEvent {
            tick: self.tick,
            npc_id: self.npcs[worker_index].id,
            hive_id: Some(hive_id),
            event_type: "delivered_food".to_string(),
            pos: worker_pos,
            details: json!({
                "queen_id": self.npcs[queen_index].id,
                "queen_pos": { "x": self.npcs[queen_index].pos.x, "y": self.npcs[queen_index].pos.y },
                "delivered_amount": delivered_amount,
                "queen_food": self.npcs[queen_index].food,
            }),
        });
        true
    }

    pub(crate) fn find_queen_pos(&self, hive_id: u16) -> Option<Position> {
        self.npcs
            .iter()
            .find(|npc| npc.kind == NpcKind::Queen && npc.hive_id == Some(hive_id))
            .map(|npc| npc.pos)
    }

    pub(crate) fn tick_worker_memory(&mut self, index: usize) {
        let Some(hive_id) = self.npcs[index].hive_id else {
            return;
        };
        let pos = self.npcs[index].pos;

        if self.npcs[index].recent_home_memory_ticks > 0 {
            self.npcs[index].recent_home_memory_ticks -= 1;
        }
        if self.npcs[index].recent_food_memory_ticks > 0 {
            self.npcs[index].recent_food_memory_ticks -= 1;
        }

        let should_refresh_home = self.npcs[index].recent_home_memory_ticks == 0;
        let should_refresh_food = self.npcs[index].recent_food_memory_ticks == 0;

        if should_refresh_home {
            let axes = self.sample_gradient_axes(pos, hive_id, PheromoneChannel::Home);
            self.npcs[index].recent_home_dir = best_direction_for_axes(axes);
            self.npcs[index].recent_home_memory_ticks = PHEROMONE_MEMORY_TICKS;
            self.push_npc_debug_event(crate::NpcDebugEvent {
                tick: self.tick,
                npc_id: self.npcs[index].id,
                hive_id: Some(hive_id),
                event_type: "memory_refresh_home".to_string(),
                pos,
                details: json!({
                    "dir": self.npcs[index].recent_home_dir.map(dir_name),
                    "ttl": self.npcs[index].recent_home_memory_ticks,
                    "axes": axes_json(axes),
                }),
            });
        }
        if should_refresh_food {
            let axes = self.sample_gradient_axes(pos, hive_id, PheromoneChannel::Food);
            self.npcs[index].recent_food_dir = best_direction_for_axes(axes);
            self.npcs[index].recent_food_memory_ticks = PHEROMONE_MEMORY_TICKS;
            self.push_npc_debug_event(crate::NpcDebugEvent {
                tick: self.tick,
                npc_id: self.npcs[index].id,
                hive_id: Some(hive_id),
                event_type: "memory_refresh_food".to_string(),
                pos,
                details: json!({
                    "dir": self.npcs[index].recent_food_dir.map(dir_name),
                    "ttl": self.npcs[index].recent_food_memory_ticks,
                    "axes": axes_json(axes),
                }),
            });
        }
    }

    fn sample_gradient_axes(
        &self,
        origin: Position,
        hive_id: u16,
        channel: PheromoneChannel,
    ) -> (u32, u32, u32, u32) {
        let mut up = 0u32;
        let mut down = 0u32;
        let mut left = 0u32;
        let mut right = 0u32;

        for dy in -PHEROMONE_MEMORY_RADIUS..=PHEROMONE_MEMORY_RADIUS {
            for dx in -PHEROMONE_MEMORY_RADIUS..=PHEROMONE_MEMORY_RADIUS {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let pos = origin.offset(dx, dy);
                if !self.world.in_bounds(pos) {
                    continue;
                }
                let value = u32::from(self.pheromones.value(pos, hive_id, channel));
                if value == 0 {
                    continue;
                }
                if dx < 0 {
                    left += value;
                } else if dx > 0 {
                    right += value;
                }
                if dy < 0 {
                    up += value;
                } else if dy > 0 {
                    down += value;
                }
            }
        }

        (left, right, up, down)
    }

    fn local_field_search_bias(&self, origin: Position, hive_id: u16) -> LocalFieldSearchScore {
        let mut best_food_distance = None;
        let mut food_field_score = 0u32;
        let mut home_field_penalty = 0u32;
        let mut stone_penalty = 0u32;

        for dy in -PHEROMONE_MEMORY_RADIUS..=PHEROMONE_MEMORY_RADIUS {
            for dx in -PHEROMONE_MEMORY_RADIUS..=PHEROMONE_MEMORY_RADIUS {
                let pos = origin.offset(dx, dy);
                if !self.world.in_bounds(pos) {
                    continue;
                }
                let distance = (dx.abs() + dy.abs()) as u32;
                let distance = distance.max(1);

                if self.world.tile(pos) == Some(Tile::Food) {
                    best_food_distance = Some(
                        best_food_distance
                            .map(|current: u32| current.min(distance))
                            .unwrap_or(distance),
                    );
                }

                let food_value =
                    u32::from(self.pheromones.value(pos, hive_id, PheromoneChannel::Food));
                if food_value > 0 {
                    food_field_score += (food_value * 8) / distance;
                }

                let home_value =
                    u32::from(self.pheromones.value(pos, hive_id, PheromoneChannel::Home));
                if home_value > 0 {
                    home_field_penalty += (home_value * 3) / distance;
                }

                if matches!(self.world.tile(pos), Some(Tile::Stone | Tile::Bedrock)) {
                    stone_penalty += 18 / distance;
                }
            }
        }

        let visible_food_bonus = best_food_distance
            .map(|distance| 360_u32.saturating_sub(distance.saturating_sub(1) * 80))
            .unwrap_or(0);

        LocalFieldSearchScore {
            visible_food_bonus,
            visible_food_distance: best_food_distance,
            food_field_score,
            home_field_penalty,
            stone_penalty,
        }
    }

    pub(crate) fn refresh_local_field_destination(
        &mut self,
        index: usize,
        hive_id: u16,
        queen_pos: Option<Position>,
    ) -> Option<Position> {
        let origin = self.npcs[index].pos;
        let current = self.npcs[index].search_destination;
        let invalid = current.is_none_or(|destination| {
            destination == origin
                || !self.local_destination_is_valid(index, destination)
                || manhattan_distance(origin, destination) > PHEROMONE_MEMORY_RADIUS
        });
        let stuck = self.npcs[index].search_destination_stuck_ticks >= 6;
        if invalid || stuck {
            self.npcs[index].search_destination =
                self.choose_local_field_destination(index, hive_id, queen_pos);
            self.npcs[index].search_destination_stuck_ticks = 0;
            if let Some(destination) = self.npcs[index].search_destination {
                self.push_npc_debug_event(crate::NpcDebugEvent {
                    tick: self.tick,
                    npc_id: self.npcs[index].id,
                    hive_id: self.npcs[index].hive_id,
                    event_type: "search_destination_selected".to_string(),
                    pos: origin,
                    details: json!({
                        "destination": { "x": destination.x, "y": destination.y },
                        "search_behavior_profile": "local_field_v2",
                    }),
                });
            }
        }
        self.npcs[index].search_destination
    }

    fn choose_local_field_destination(
        &self,
        index: usize,
        hive_id: u16,
        queen_pos: Option<Position>,
    ) -> Option<Position> {
        let origin = self.npcs[index].pos;
        let reachable = self.local_reachable_search_tiles(index, origin);
        let mut best = None;
        let mut best_score = 0u32;

        for (candidate, distance) in reachable {
            if candidate == origin {
                continue;
            }
            let Some(tile) = self.world.tile(candidate) else {
                continue;
            };
            let local = self.local_field_search_bias(candidate, hive_id);
            let home_here = u32::from(self.pheromones.value(
                candidate,
                hive_id,
                PheromoneChannel::Home,
            ));
            let food_here = u32::from(self.pheromones.value(
                candidate,
                hive_id,
                PheromoneChannel::Food,
            ));
            let tile_bonus = match tile {
                Tile::Food => 1_200_u32,
                Tile::Empty => 60_u32,
                Tile::Dirt | Tile::Resource => 20_u32,
                Tile::Stone | Tile::Bedrock => 0_u32,
            };
            let outward_bonus = queen_pos
                .map(|queen| {
                    let current = manhattan_distance(origin, queen);
                    let next = manhattan_distance(candidate, queen);
                    if next > current {
                        (u32::try_from(next - current).unwrap_or(0)) * 25
                    } else {
                        0
                    }
                })
                .unwrap_or(0);
            let openness_bonus =
                u32::try_from(self.open_cardinal_neighbor_count(candidate, Some(index)))
                    .unwrap_or(0)
                    * 18;
            let distance_bonus = u32::try_from(distance).unwrap_or(0) * 20;
            let exploration_bonus = 255_u32.saturating_sub(home_here).min(80);
            let score = tile_bonus
                + local.visible_food_bonus
                + (local.food_field_score * 3)
                + (food_here * 16)
                + outward_bonus
                + openness_bonus
                + distance_bonus
                + exploration_bonus;
            let penalty = (local.home_field_penalty / 4)
                + (local.stone_penalty / 3)
                + recent_position_penalty(&self.npcs[index].recent_positions, candidate);
            let score = score.saturating_sub(penalty);
            if score > best_score {
                best_score = score;
                best = Some(candidate);
            }
        }

        best
    }

    fn local_reachable_search_tiles(
        &self,
        index: usize,
        origin: Position,
    ) -> Vec<(Position, usize)> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut reachable = Vec::new();
        visited.insert(origin);
        queue.push_back((origin, 0usize));

        while let Some((pos, distance)) = queue.pop_front() {
            reachable.push((pos, distance));
            for next in [
                pos.offset(-1, 0),
                pos.offset(1, 0),
                pos.offset(0, -1),
                pos.offset(0, 1),
            ] {
                if visited.contains(&next)
                    || !self.world.in_bounds(next)
                    || manhattan_distance(origin, next) > PHEROMONE_MEMORY_RADIUS
                    || !self.search_tile_traversable(index, next)
                {
                    continue;
                }
                visited.insert(next);
                queue.push_back((next, distance + 1));
            }
        }

        reachable
    }

    fn local_destination_is_valid(&self, index: usize, destination: Position) -> bool {
        self.world.in_bounds(destination) && self.search_tile_traversable(index, destination)
    }

    fn search_tile_traversable(&self, index: usize, pos: Position) -> bool {
        if self.players.values().any(|player| player.pos == pos)
            || self.npc_blocks_movement(pos, index)
            || self.art_occupies_cell(pos)
        {
            return false;
        }
        matches!(
            self.world.tile(pos),
            Some(Tile::Empty | Tile::Food | Tile::Dirt | Tile::Resource)
        )
    }

    pub(crate) fn update_search_destination_progress(
        &mut self,
        index: usize,
        current_pos: Position,
        chosen_next: Option<Position>,
    ) {
        let Some(destination) = self.npcs[index].search_destination else {
            return;
        };
        let Some(chosen_next) = chosen_next else {
            self.npcs[index].search_destination_stuck_ticks = self.npcs[index]
                .search_destination_stuck_ticks
                .saturating_add(1);
            return;
        };
        if chosen_next == destination {
            self.npcs[index].search_destination = None;
            self.npcs[index].search_destination_stuck_ticks = 0;
            return;
        }
        let current_distance = manhattan_distance(current_pos, destination);
        let next_distance = manhattan_distance(chosen_next, destination);
        if next_distance < current_distance {
            self.npcs[index].search_destination_stuck_ticks = 0;
        } else {
            self.npcs[index].search_destination_stuck_ticks = self.npcs[index]
                .search_destination_stuck_ticks
                .saturating_add(1);
        }
    }

    fn local_neighborhood_snapshot(&self, origin: Position, hive_id: u16) -> Value {
        let mut cells = Vec::new();
        for dy in -PHEROMONE_MEMORY_RADIUS..=PHEROMONE_MEMORY_RADIUS {
            for dx in -PHEROMONE_MEMORY_RADIUS..=PHEROMONE_MEMORY_RADIUS {
                let pos = origin.offset(dx, dy);
                if !self.world.in_bounds(pos) {
                    continue;
                }
                cells.push(json!({
                    "dx": dx,
                    "dy": dy,
                    "x": pos.x,
                    "y": pos.y,
                    "tile": self.world.tile(pos).map(tile_name),
                    "home": self.pheromones.value(pos, hive_id, PheromoneChannel::Home),
                    "food": self.pheromones.value(pos, hive_id, PheromoneChannel::Food),
                    "player": self.players.values().any(|player| player.pos == pos),
                    "npc": self.npcs.iter().any(|npc| npc.pos == pos),
                }));
            }
        }
        Value::Array(cells)
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

                let target =
                    if self.world.in_bounds(below) && self.world.tile(below) == Some(Tile::Empty) {
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

    fn tick_plant_growth(&mut self) {
        let frequency = config_f64(
            &self.config,
            "soil.plant_growth_frequency",
            DEFAULT_PLANT_GROWTH_FREQUENCY,
        )
        .clamp(0.0, 1.0);
        if frequency <= 0.0 {
            return;
        }
        let vertical_growth_multiple = config_f64(
            &self.config,
            "soil.vertical_growth_multiple",
            DEFAULT_SOIL_VERTICAL_GROWTH_MULTIPLE,
        )
        .max(0.0);
        let total_direction_weight = (vertical_growth_multiple * 2.0) + 2.0;
        if total_direction_weight <= 0.0 {
            return;
        }

        let occupied: HashSet<_> = self
            .players
            .values()
            .map(|player| player.pos)
            .chain(self.npcs.iter().map(|npc| npc.pos))
            .collect();
        let mut new_growth = HashSet::new();

        for y in 0..self.world.height() {
            for x in 0..self.world.width() {
                let pos = Position { x, y };
                if self.world.tile(pos) != Some(Tile::Food) {
                    continue;
                }

                let roll = self.rng.random::<f64>() * total_direction_weight;
                let target = if roll < vertical_growth_multiple {
                    pos.offset(0, -1)
                } else if roll < vertical_growth_multiple * 2.0 {
                    pos.offset(0, 1)
                } else if roll < (vertical_growth_multiple * 2.0) + 1.0 {
                    pos.offset(-1, 0)
                } else {
                    pos.offset(1, 0)
                };
                if !self.world.in_bounds(target) {
                    continue;
                }
                let Some(target_tile) = self.world.tile(target) else {
                    continue;
                };
                if !matches!(target_tile, Tile::Empty | Tile::Dirt | Tile::Stone)
                    || occupied.contains(&target)
                    || new_growth.contains(&target)
                {
                    continue;
                }

                let neighboring_food = (-1..=1)
                    .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                    .filter(|(dx, dy)| !(*dx == 0 && *dy == 0))
                    .filter_map(|(dx, dy)| {
                        let neighbor = target.offset(dx, dy);
                        self.world.in_bounds(neighbor).then_some(neighbor)
                    })
                    .filter(|neighbor| self.world.tile(*neighbor) == Some(Tile::Food))
                    .count();
                if !(1..=3).contains(&neighboring_food) {
                    continue;
                }
                let cardinal_neighboring_food = [
                    target.offset(0, -1),
                    target.offset(0, 1),
                    target.offset(-1, 0),
                    target.offset(1, 0),
                ]
                .into_iter()
                .filter(|neighbor| self.world.in_bounds(*neighbor))
                .filter(|neighbor| self.world.tile(*neighbor) == Some(Tile::Food))
                .count();
                let effective_frequency = if cardinal_neighboring_food >= 2 {
                    (frequency * 4.0).clamp(0.0, 1.0)
                } else {
                    frequency
                };
                if self.rng.random::<f64>() > effective_frequency {
                    continue;
                }

                new_growth.insert(target);
            }
        }

        for pos in new_growth {
            self.set_world_tile(pos, Tile::Food);
        }
    }
}

impl GameState {
    fn worker_lifespan_ticks_for(&self, index: usize) -> u16 {
        let role_path = self.worker_role_path(index);
        configured_worker_roles(&self.config)
            .into_iter()
            .find(|role| role.path == role_path)
            .map(|role| role.lifespan_ticks)
            .unwrap_or_else(|| {
                config_u16(
                    &self.config,
                    "colony.worker_lifespan_ticks",
                    NPC_WORKER_LIFESPAN_TICKS,
                )
            })
    }

    fn queen_egg_food_cost(&self) -> u16 {
        config_u16(
            &self.config,
            "colony.queen_egg_food_cost",
            QUEEN_EGG_FOOD_COST,
        )
    }

    fn minimum_delay_to_hatch(&self) -> u16 {
        self.config
            .pointer("/colony/minimum_delay_to_hatch")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .or_else(|| {
                self.config
                    .pointer("/colony/egg_hatch_ticks")
                    .and_then(Value::as_u64)
                    .and_then(|value| u16::try_from(value).ok())
            })
            .unwrap_or(EGG_HATCH_TICKS)
    }

    fn egg_laying_cooldown_ticks(&self) -> u64 {
        config_u64(&self.config, "queen.egg_laying_cooldown_ticks", 1)
    }

    fn egg_hatch_cooldown_ticks(&self) -> u64 {
        config_u64(&self.config, "queen.egg_hatch_cooldown_ticks", 0)
    }

    fn queen_delivery_radius(&self) -> i32 {
        config_i32(&self.config, "colony.queen_delivery_radius", 5).max(1)
    }

    fn queen_no_fill_radius(&self) -> i32 {
        config_i32(&self.config, "colony.queen_no_fill_radius", 8).max(0)
    }

    pub(crate) fn queen_chamber_max_radii(&self) -> (i32, i32) {
        let radius_x = config_i32(
            &self.config,
            "colony.roles.hive_maintenance.queen_chamber.radius_x",
            DEFAULT_QUEEN_CHAMBER_RADIUS_X,
        )
        .max(1);
        let radius_y = config_i32(
            &self.config,
            "colony.roles.hive_maintenance.queen_chamber.radius_y",
            DEFAULT_QUEEN_CHAMBER_RADIUS_Y,
        )
        .max(1);
        (radius_x, radius_y)
    }

    fn dirt_place_cooldown_ticks(&self) -> u64 {
        config_u64(&self.config, "colony.dirt_place_cooldown_ticks", 11)
    }

    fn max_workers_per_hive(&self) -> Option<usize> {
        match config_u64(&self.config, "colony.max_workers_per_hive", 0) {
            0 => None,
            value => usize::try_from(value).ok(),
        }
    }

    fn food_carry_max(&self) -> u16 {
        config_u16(&self.config, "colony.food_carry_max", 1).max(1)
    }

    fn adjacent_food_visible(&self, pos: Position) -> bool {
        !self.adjacent_food_positions(pos).is_empty()
    }

    fn adjacent_food_positions(&self, pos: Position) -> Vec<Position> {
        (-1..=1)
            .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
            .filter(|(dx, dy)| !(*dx == 0 && *dy == 0))
            .map(|(dx, dy)| pos.offset(dx, dy))
            .filter(|neighbor| self.world.in_bounds(*neighbor))
            .filter(|neighbor| self.world.tile(*neighbor) == Some(Tile::Food))
            .collect()
    }

    fn search_behavior_profile(&self) -> SearchBehaviorProfile {
        match self
            .config
            .pointer("/colony/search_behavior_profile")
            .and_then(Value::as_str)
            .unwrap_or("baseline")
        {
            "outward_bias_with_local_field_v1" => {
                SearchBehaviorProfile::OutwardBiasWithLocalFieldV1
            }
            "local_field_v1" => SearchBehaviorProfile::LocalFieldV1,
            "local_field_v2" => SearchBehaviorProfile::LocalFieldV2,
            "outward_bias_v1" => SearchBehaviorProfile::OutwardBiasV1,
            _ => SearchBehaviorProfile::Baseline,
        }
    }

    fn effective_search_behavior_profile(
        &self,
        index: usize,
        configured_profile: SearchBehaviorProfile,
    ) -> SearchBehaviorProfile {
        match configured_profile {
            SearchBehaviorProfile::OutwardBiasWithLocalFieldV1 => {
                if self.npcs[index].has_delivered_food {
                    SearchBehaviorProfile::OutwardBiasV1
                } else {
                    SearchBehaviorProfile::LocalFieldV2
                }
            }
            _ => configured_profile,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LocalFieldSearchScore {
    visible_food_bonus: u32,
    visible_food_distance: Option<u32>,
    food_field_score: u32,
    home_field_penalty: u32,
    stone_penalty: u32,
}

fn food_deposit_for_carry_ticks(carry_ticks: u16) -> u8 {
    let decay = (carry_ticks / WORKER_FOOD_DEPOSIT_DECAY_STEPS) as u8;
    WORKER_FOOD_DEPOSIT_PEAK
        .saturating_sub(decay)
        .max(WORKER_FOOD_DEPOSIT_FLOOR)
}

fn home_deposit_for_trail_steps(trail_steps: u16) -> u8 {
    let decay = (trail_steps / WORKER_HOME_DEPOSIT_DECAY_STEPS) as u8;
    WORKER_HOME_DEPOSIT.saturating_sub(decay)
}

fn worker_lifespan_bonus(age_ticks: u16, default_max_life_span: u16) -> u16 {
    if default_max_life_span == 0 {
        return 0;
    }
    let remaining = default_max_life_span.saturating_sub(age_ticks) as u32;
    ((remaining * 200) / u32::from(default_max_life_span)) as u16
}

fn search_behavior_profile_name(profile: SearchBehaviorProfile) -> &'static str {
    match profile {
        SearchBehaviorProfile::Baseline => "baseline",
        SearchBehaviorProfile::OutwardBiasV1 => "outward_bias_v1",
        SearchBehaviorProfile::LocalFieldV1 => "local_field_v1",
        SearchBehaviorProfile::LocalFieldV2 => "local_field_v2",
        SearchBehaviorProfile::OutwardBiasWithLocalFieldV1 => "outward_bias_with_local_field_v1",
    }
}

fn same_hive(left: Option<u16>, right: Option<u16>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        (None, None) => true,
        _ => false,
    }
}

fn local_field_destination_bias(current: Position, next: Position, destination: Position) -> u32 {
    let current_distance = manhattan_distance(current, destination);
    let next_distance = manhattan_distance(next, destination);
    if next == destination {
        280
    } else if next_distance < current_distance {
        u32::try_from(current_distance - next_distance).unwrap_or(0) * 140
    } else if next_distance == current_distance {
        12
    } else {
        0
    }
}

fn manhattan_distance(a: Position, b: Position) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

fn remember_recent_position(recent_positions: &mut Vec<Position>, pos: Position) {
    recent_positions.push(pos);
    if recent_positions.len() > RECENT_POSITION_MEMORY_SIZE {
        let extra = recent_positions.len() - RECENT_POSITION_MEMORY_SIZE;
        recent_positions.drain(0..extra);
    }
}

fn recent_position_penalty(recent_positions: &[Position], next: Position) -> u32 {
    recent_positions
        .iter()
        .rev()
        .enumerate()
        .find_map(|(index, pos)| {
            if *pos != next {
                return None;
            }
            Some(match index {
                0 => 160,
                1 => 120,
                2 => 80,
                3 => 48,
                _ => 24,
            })
        })
        .unwrap_or(0)
}

fn best_direction_for_axes((left, right, up, down): (u32, u32, u32, u32)) -> Option<MoveDir> {
    let candidates = [
        (left, MoveDir::Left),
        (right, MoveDir::Right),
        (up, MoveDir::Up),
        (down, MoveDir::Down),
    ];
    let best = candidates.into_iter().max_by_key(|(score, _)| *score)?;
    (best.0 > 0).then_some(best.1)
}

fn axes_json((left, right, up, down): (u32, u32, u32, u32)) -> Value {
    json!({
        "left": left,
        "right": right,
        "up": up,
        "down": down,
    })
}

fn direction_bias(preferred: Option<MoveDir>, current: Position, next: Position) -> u8 {
    let Some(preferred) = preferred else {
        return 0;
    };
    let dir = match (next.x - current.x, next.y - current.y) {
        (-1, 0) => MoveDir::Left,
        (1, 0) => MoveDir::Right,
        (0, -1) => MoveDir::Up,
        (0, 1) => MoveDir::Down,
        _ => return 0,
    };
    if dir == preferred { 32 } else { 0 }
}

fn dir_name(dir: MoveDir) -> &'static str {
    match dir {
        MoveDir::Up => "up",
        MoveDir::Down => "down",
        MoveDir::Left => "left",
        MoveDir::Right => "right",
    }
}

fn tile_name(tile: Tile) -> &'static str {
    match tile {
        Tile::Empty => "empty",
        Tile::Dirt => "dirt",
        Tile::Stone => "stone",
        Tile::Resource => "resource",
        Tile::Food => "food",
        Tile::Bedrock => "bedrock",
    }
}

fn behavior_name(behavior: AntBehaviorState) -> &'static str {
    match behavior {
        AntBehaviorState::Searching => "searching",
        AntBehaviorState::ReturningFood => "returning_food",
        AntBehaviorState::Defending => "defending",
        AntBehaviorState::Idle => "idle",
    }
}
