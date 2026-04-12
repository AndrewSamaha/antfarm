use crate::{
    constants::{MAX_PLAYERS, STONE_DIG_STEPS},
    inventory::{add_inventory, default_inventory, inventory_count, remove_inventory},
    protocol::{Action, DigProgress, PlaceMaterial, Snapshot},
    types::{Facing, MoveDir, Player, Position, Tile},
};

use super::GameState;

impl GameState {
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

    pub fn apply_action(&mut self, player_id: u8, action: Action) {
        match action {
            Action::Move(dir) => self.move_player(player_id, dir),
            Action::Dig(dir) => self.dig(player_id, dir),
            Action::Place { dir, material } => self.place(player_id, dir, material),
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
}
