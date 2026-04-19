use rand::Rng;
use serde_json::{Value, json};

const QUEEN_DELIVERY_RADIUS: i32 = 5;
const RECENT_POSITION_MEMORY_SIZE: usize = 5;

use crate::{
    constants::{
        DEFAULT_SOIL_SETTLE_FREQUENCY, EGG_HATCH_TICKS, PHEROMONE_DECAY_AMOUNT,
        PHEROMONE_DECAY_INTERVAL_TICKS, PHEROMONE_MEMORY_RADIUS, PHEROMONE_MEMORY_TICKS,
        QUEEN_EGG_FOOD_COST, QUEEN_HOME_EMIT_PEAK, QUEEN_HOME_EMIT_RADIUS, SURFACE_Y,
        WORKER_FOOD_DEPOSIT_DECAY_STEPS, WORKER_FOOD_DEPOSIT_FLOOR, WORKER_FOOD_DEPOSIT_PEAK,
        WORKER_HOME_DEPOSIT,
    },
    inventory::{add_inventory, default_npc_inventory, remove_inventory},
    npc::nearest_open_tile,
    pheromones::{AntBehaviorState, PheromoneChannel},
    types::{MoveDir, NpcAnt, NpcKind, Position, Tile},
};

use super::GameState;

impl GameState {
    pub fn tick(&mut self) {
        self.tick += 1;
        self.tick_soil_settling();
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
        self.tick_worker_memory(index);
        let npc_pos = self.npcs[index].pos;
        let npc_id = self.npcs[index].id;
        let npc_hive = self.npcs[index].hive_id;
        let behavior = self.npcs[index].behavior;
        let queen_pos = npc_hive.and_then(|hive_id| self.find_queen_pos(hive_id));
        let home_axes = npc_hive.map(|hive_id| {
            self.sample_gradient_axes(npc_pos, hive_id, PheromoneChannel::Home)
        });
        let food_axes = npc_hive.map(|hive_id| {
            self.sample_gradient_axes(npc_pos, hive_id, PheromoneChannel::Food)
        });
        let current_food_pheromone = u32::from(
            npc_hive
                .map(|hive_id| self.pheromones.value(npc_pos, hive_id, PheromoneChannel::Food))
                .unwrap_or(0),
        );

        if let Some(hive_id) = npc_hive {
            match behavior {
                AntBehaviorState::Searching => {
                    self.pheromones
                        .deposit(npc_pos, hive_id, PheromoneChannel::Home, WORKER_HOME_DEPOSIT);
                }
                AntBehaviorState::ReturningFood => {
                    let deposit = food_deposit_for_carry_ticks(self.npcs[index].carrying_food_ticks);
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
            if !self.world.in_bounds(next) || self.npc_occupied(next, Some(index)) {
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
                AntBehaviorState::Searching => direction_bias(self.npcs[index].recent_food_dir, npc_pos, next),
                AntBehaviorState::ReturningFood => direction_bias(self.npcs[index].recent_home_dir, npc_pos, next),
                AntBehaviorState::Defending | AntBehaviorState::Idle => 0,
            });
            let recent_position_penalty = recent_position_penalty(&self.npcs[index].recent_positions, next);
            raw_candidates.push((
                next,
                tile,
                food_pheromone,
                home_pheromone,
                random_score,
                tile_bonus,
                queen_bias,
                memory_bias,
                recent_position_penalty,
            ));
        }

        let has_increasing_adjacent_food_signal = matches!(behavior, AntBehaviorState::Searching)
            && raw_candidates
                .iter()
                .any(|(_, _, food_pheromone, _, _, _, _, _, _)| *food_pheromone > current_food_pheromone);

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
            recent_position_penalty,
        ) in raw_candidates
        {
            let pheromone_score = match behavior {
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
            let score = pheromone_score + random_score + tile_bonus + queen_bias + memory_bias;
            let score = score.saturating_sub(terrain_penalty + recent_position_penalty);
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
                "recent_position_penalty": recent_position_penalty,
                "terrain_penalty": terrain_penalty,
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
                    if matches!(behavior, AntBehaviorState::ReturningFood) {
                        self.npcs[index].carrying_food_ticks =
                            self.npcs[index].carrying_food_ticks.saturating_add(1);
                    }
                    remember_recent_position(&mut self.npcs[index].recent_positions, next);
                    self.npcs_dirty = true;
                    outcome = "moved".to_string();
                    chosen_next = Some(next);
                    break;
                }
                Some(Tile::Food) if !matches!(behavior, AntBehaviorState::ReturningFood) => {
                    self.set_world_tile(next, Tile::Empty);
                    self.npcs[index].pos = next;
                    self.npcs[index].carrying_food = true;
                    self.npcs[index].behavior = AntBehaviorState::ReturningFood;
                    self.npcs[index].carrying_food_ticks = 0;
                    self.npcs[index].recent_home_memory_ticks = 0;
                    self.npcs[index].recent_positions.clear();
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
                        }),
                    });
                    outcome = "picked_up_food".to_string();
                    chosen_next = Some(next);
                    break;
                }
                Some(Tile::Dirt) | Some(Tile::Resource) => {
                    match tile {
                        Some(Tile::Dirt) => add_inventory(&mut self.npcs[index].inventory, "dirt", 1),
                        Some(Tile::Resource) => add_inventory(&mut self.npcs[index].inventory, "ore", 1),
                        _ => {}
                    }
                    self.set_world_tile(next, Tile::Empty);
                    events.push(format!(
                        "NPC ant {} tunneled at {},{}",
                        npc_id, next.x, next.y
                    ));
                    outcome = "tunneled".to_string();
                    chosen_next = Some(next);
                    break;
                }
                Some(Tile::Food) | Some(Tile::Stone) | Some(Tile::Bedrock) | None => {}
            }
        }

        self.push_npc_debug_event(crate::NpcDebugEvent {
            tick: self.tick,
            npc_id,
            hive_id: npc_hive,
            event_type: "selected_move".to_string(),
            pos: npc_pos,
            details: json!({
                "behavior": behavior_name(behavior),
                "carrying_food": self.npcs[index].carrying_food,
                "carrying_food_ticks": self.npcs[index].carrying_food_ticks,
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

    fn tick_queen(&mut self, index: usize, spawned_npcs: &mut Vec<NpcAnt>, events: &mut Vec<String>) {
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
        if self.npcs[index].food < QUEEN_EGG_FOOD_COST {
            return;
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

        self.npcs[index].food = self.npcs[index].food.saturating_sub(QUEEN_EGG_FOOD_COST);
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
            recent_home_dir: None,
            recent_food_dir: None,
            recent_home_memory_ticks: 0,
            recent_food_memory_ticks: 0,
            recent_positions: Vec::new(),
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
        let egg = &mut self.npcs[index];
        egg.age_ticks = egg.age_ticks.saturating_add(1);
        if egg.age_ticks < EGG_HATCH_TICKS {
            return;
        }

        egg.kind = NpcKind::Worker;
        egg.health = NpcKind::Worker.max_health();
        egg.food = 0;
        egg.age_ticks = 0;
        egg.behavior = AntBehaviorState::Searching;
        egg.carrying_food = false;
        egg.carrying_food_ticks = 0;
        egg.recent_home_dir = None;
        egg.recent_food_dir = None;
        egg.recent_home_memory_ticks = 0;
        egg.recent_food_memory_ticks = 0;
        egg.recent_positions.clear();
        let hatched_id = egg.id;
        let hatched_hive_id = egg.hive_id;
        let hatched_pos = egg.pos;
        self.npcs_dirty = true;
        events.push(format!(
            "Egg {} hatched into a worker ant",
            hatched_id
        ));
        self.push_npc_debug_event(crate::NpcDebugEvent {
            tick: self.tick,
            npc_id: hatched_id,
            hive_id: hatched_hive_id,
            event_type: "egg_hatched".to_string(),
            pos: hatched_pos,
            details: json!({}),
        });
    }

    fn npc_occupied(&self, pos: Position, ignore_index: Option<usize>) -> bool {
        self.npcs
            .iter()
            .enumerate()
            .any(|(index, npc)| Some(index) != ignore_index && npc.pos == pos)
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
                    <= QUEEN_DELIVERY_RADIUS
        });
        let Some(queen_index) = queen_index else {
            return false;
        };

        self.npcs[queen_index].food = self.npcs[queen_index]
            .food
            .saturating_add(1)
            .min(NpcKind::Queen.max_food());
        self.npcs[worker_index].carrying_food = false;
        self.npcs[worker_index].carrying_food_ticks = 0;
        self.npcs[worker_index].behavior = AntBehaviorState::Searching;
        self.npcs[worker_index].food = 0;
        self.npcs[worker_index].recent_food_memory_ticks = 0;
        self.npcs[worker_index].recent_positions.clear();
        self.push_npc_debug_event(crate::NpcDebugEvent {
            tick: self.tick,
            npc_id: self.npcs[worker_index].id,
            hive_id: Some(hive_id),
            event_type: "delivered_food".to_string(),
            pos: worker_pos,
            details: json!({
                "queen_id": self.npcs[queen_index].id,
                "queen_pos": { "x": self.npcs[queen_index].pos.x, "y": self.npcs[queen_index].pos.y },
                "queen_food": self.npcs[queen_index].food,
            }),
        });
        true
    }

    fn find_queen_pos(&self, hive_id: u16) -> Option<Position> {
        self.npcs
            .iter()
            .find(|npc| npc.kind == NpcKind::Queen && npc.hive_id == Some(hive_id))
            .map(|npc| npc.pos)
    }

    fn tick_worker_memory(&mut self, index: usize) {
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
        let frequency = crate::config::config_f64(
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
}

fn food_deposit_for_carry_ticks(carry_ticks: u16) -> u8 {
    let decay = (carry_ticks / WORKER_FOOD_DEPOSIT_DECAY_STEPS) as u8;
    WORKER_FOOD_DEPOSIT_PEAK
        .saturating_sub(decay)
        .max(WORKER_FOOD_DEPOSIT_FLOOR)
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
