use rand::Rng;

use crate::{
    constants::DEFAULT_SOIL_SETTLE_FREQUENCY,
    inventory::remove_inventory,
    npc::nearest_target,
    types::{Position, Tile},
};

use super::GameState;

impl GameState {
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
