use rand::Rng;

use crate::{
    constants::{DEFAULT_SOIL_SETTLE_FREQUENCY, EGG_HATCH_TICKS, QUEEN_EGG_FOOD_COST, SURFACE_Y},
    inventory::remove_inventory,
    npc::nearest_open_tile,
    types::{NpcAnt, NpcKind, Position, Tile},
};

use super::GameState;

impl GameState {
    pub fn tick(&mut self) {
        self.tick += 1;
        self.tick_soil_settling();

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
        let npc_pos = self.npcs[index].pos;
        let npc_id = self.npcs[index].id;
        let directions = [
            npc_pos.offset(-1, 0),
            npc_pos.offset(1, 0),
            npc_pos.offset(0, 1),
            npc_pos.offset(0, -1),
        ];
        let start = self.rng.random_range(0..directions.len());

        for step in 0..directions.len() {
            let next = directions[(start + step) % directions.len()];
            if next.y <= SURFACE_Y {
                continue;
            }
            if !self.world.in_bounds(next) || self.npc_occupied(next, Some(index)) {
                continue;
            }
            let tile = self.world.tile(next);
            match tile {
                Some(Tile::Empty) => {
                    self.npcs[index].pos = next;
                    self.npcs[index].food = self.npcs[index]
                        .food
                        .saturating_sub(1)
                        .min(NpcKind::Worker.max_food());
                    self.npcs_dirty = true;
                    break;
                }
                Some(Tile::Dirt) | Some(Tile::Resource) | Some(Tile::Food) => {
                    self.set_world_tile(next, Tile::Empty);
                    if tile == Some(Tile::Food) {
                        self.npcs[index].food = self.npcs[index]
                            .food
                            .saturating_add(1)
                            .min(NpcKind::Worker.max_food());
                    }
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

    fn tick_queen(&mut self, index: usize, spawned_npcs: &mut Vec<NpcAnt>, events: &mut Vec<String>) {
        let queen_pos = self.npcs[index].pos;
        let queen_id = self.npcs[index].id;
        let queen_hive_id = self.npcs[index].hive_id;
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
            kind: NpcKind::Egg,
            health: NpcKind::Egg.max_health(),
            food: 0,
            hive_id: queen_hive_id,
            age_ticks: 0,
        });
        self.next_npc_id = self.next_npc_id.saturating_add(1);
        self.npcs_dirty = true;
        events.push(format!(
            "Queen {} laid an egg at {},{}",
            queen_id, egg_pos.x, egg_pos.y
        ));
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
        self.npcs_dirty = true;
        events.push(format!(
            "Egg {} hatched into a worker ant",
            egg.id
        ));
    }

    fn npc_occupied(&self, pos: Position, ignore_index: Option<usize>) -> bool {
        self.npcs
            .iter()
            .enumerate()
            .any(|(index, npc)| Some(index) != ignore_index && npc.pos == pos)
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
