use crate::{
    art::find_ascii_art_asset,
    config::config_u16,
    constants::SURFACE_Y,
    constants::{MAX_PLAYERS, QUEEN_EGG_FOOD_COST, STONE_DIG_STEPS},
    inventory::{
        add_inventory, default_inventory, default_npc_inventory, inventory_count, remove_inventory,
    },
    pheromones::AntBehaviorState,
    protocol::{Action, DigProgress, PlaceMaterial, PlacedArt, Snapshot},
    types::{Facing, MoveDir, NpcAnt, NpcKind, NpcRoleState, Player, Position, Tile},
};

use super::GameState;

const QUEEN_ART_ID: &str = "queen_ant";

impl GameState {
    pub fn dig_area(&mut self, player_id: u8, width: u16, height: u16) -> Result<(), String> {
        let Some(player) = self.players.get(&player_id).cloned() else {
            return Err("unknown player".to_string());
        };
        self.dig_area_at(player.pos, width, height, Some(player.name))
    }

    pub fn dig_area_at(
        &mut self,
        center: Position,
        width: u16,
        height: u16,
        actor_label: Option<String>,
    ) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Err("dig dimensions must be greater than zero".to_string());
        }

        let half_w = i32::from(width) / 2;
        let half_h = i32::from(height) / 2;
        let left = center.x - half_w;
        let top = center.y - half_h;

        let mut dirt = 0u16;
        let mut stone = 0u16;
        let mut ore = 0u16;
        let mut food = 0u16;

        for dy in 0..i32::from(height) {
            for dx in 0..i32::from(width) {
                let pos = Position {
                    x: left + dx,
                    y: top + dy,
                };
                let Some(tile) = self.world.tile(pos) else {
                    continue;
                };
                match tile {
                    Tile::Empty | Tile::Bedrock => continue,
                    Tile::Dirt => dirt = dirt.saturating_add(1),
                    Tile::Stone => stone = stone.saturating_add(1),
                    Tile::Resource => ore = ore.saturating_add(1),
                    Tile::Food => food = food.saturating_add(1),
                }
                self.set_world_tile(pos, Tile::Empty);
            }
        }

        if let Some(player_name) = actor_label {
            let Some(player) = self
                .players
                .values_mut()
                .find(|player| player.name == player_name)
            else {
                return Err("unknown player".to_string());
            };
            if dirt > 0 {
                add_inventory(&mut player.inventory, "dirt", dirt);
            }
            if stone > 0 {
                add_inventory(&mut player.inventory, "stone", stone);
            }
            if ore > 0 {
                add_inventory(&mut player.inventory, "ore", ore);
            }
            if food > 0 {
                add_inventory(&mut player.inventory, "food", food);
            }
            self.players_dirty = true;
            self.push_event(format!(
                "{} excavated {}x{} around the colony",
                player_name, width, height
            ));
        } else {
            self.push_event(format!(
                "Server excavated {}x{} at {},{}",
                width, height, center.x, center.y
            ));
        }
        Ok(())
    }

    pub fn put_area(
        &mut self,
        player_id: u8,
        resource: &str,
        width: u16,
        height: u16,
    ) -> Result<(), String> {
        let Some(player) = self.players.get(&player_id).cloned() else {
            return Err("unknown player".to_string());
        };
        self.put_area_to_right_of(player.pos, resource, width, height, Some(player.name))
    }

    pub fn put_area_at(
        &mut self,
        center: Position,
        resource: &str,
        width: u16,
        height: u16,
        actor_label: Option<String>,
    ) -> Result<(), String> {
        if matches!(resource, "q" | "queen") {
            return self.put_queen_at(center, actor_label);
        }
        if width == 0 || height == 0 {
            return Err("put dimensions must be greater than zero".to_string());
        }

        let tile = put_resource_tile(resource)
            .ok_or_else(|| format!("unsupported put resource: {resource}"))?;
        let half_w = i32::from(width) / 2;
        let half_h = i32::from(height) / 2;
        let left = center.x - half_w;
        let top = center.y - half_h;
        let mut placed = 0u16;

        for dy in 0..i32::from(height) {
            for dx in 0..i32::from(width) {
                let pos = Position {
                    x: left + dx,
                    y: top + dy,
                };
                if !self.world.in_bounds(pos) || self.occupied_by_actor(pos) {
                    continue;
                }
                if matches!(self.world.tile(pos), Some(Tile::Bedrock) | None) {
                    continue;
                }
                self.set_world_tile(pos, tile);
                placed = placed.saturating_add(1);
            }
        }

        if let Some(actor_label) = actor_label {
            self.push_event(format!(
                "{} placed {} {} tiles centered at {},{}",
                actor_label, placed, resource, center.x, center.y
            ));
        } else {
            self.push_event(format!(
                "Server placed {} {} tiles centered at {},{}",
                placed, resource, center.x, center.y
            ));
        }
        Ok(())
    }

    pub fn put_queen_at(
        &mut self,
        center: Position,
        actor_label: Option<String>,
    ) -> Result<(), String> {
        let Some(asset) = find_ascii_art_asset(QUEEN_ART_ID) else {
            return Err("queen art asset is missing".to_string());
        };
        let hive_id = self.next_hive_id;
        let origin = Position {
            x: center.x - asset.world_anchor_x(),
            y: center.y - asset.anchor_y,
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
                if !self.world.in_bounds(pos) {
                    return Err(format!(
                        "queen footprint is out of bounds at {},{}",
                        pos.x, pos.y
                    ));
                }
                if self.world.tile(pos) == Some(Tile::Bedrock) {
                    return Err(format!(
                        "cannot place queen through bedrock at {},{}",
                        pos.x, pos.y
                    ));
                }
                if self.occupied_by_actor(pos) || self.art_occupies_cell(pos) {
                    return Err(format!(
                        "queen footprint is occupied at {},{}",
                        pos.x, pos.y
                    ));
                }
            }
        }

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
                self.set_world_tile(pos, Tile::Empty);
            }
        }

        self.placed_art.push(PlacedArt {
            asset_id: QUEEN_ART_ID.to_string(),
            pos: origin,
            hive_id: Some(hive_id),
        });
        self.npcs.push(NpcAnt {
            id: self.next_npc_id,
            pos: center,
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
            role_state: NpcRoleState::None,
        });
        self.next_hive_id = self.next_hive_id.saturating_add(1);
        self.next_npc_id = self.next_npc_id.saturating_add(1);
        self.npcs_dirty = true;
        self.placed_art_dirty = true;
        match actor_label {
            Some(actor_label) => self.push_event(format!(
                "{} placed the queen at {},{}",
                actor_label, center.x, center.y
            )),
            None => self.push_event(format!(
                "Server placed the queen at {},{}",
                center.x, center.y
            )),
        }
        Ok(())
    }

    fn put_area_to_right_of(
        &mut self,
        origin: Position,
        resource: &str,
        width: u16,
        height: u16,
        actor_label: Option<String>,
    ) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Err("put dimensions must be greater than zero".to_string());
        }

        let tile = put_resource_tile(resource)
            .ok_or_else(|| format!("unsupported put resource: {resource}"))?;

        let left = origin.x + 1;
        let top = origin.y;
        let mut placed = 0u16;

        for dy in 0..=i32::from(height) {
            for dx in 0..=i32::from(width) {
                let pos = Position {
                    x: left + dx,
                    y: top + dy,
                };
                if !self.world.in_bounds(pos) || self.occupied_by_actor(pos) {
                    continue;
                }
                if matches!(self.world.tile(pos), Some(Tile::Bedrock) | None) {
                    continue;
                }
                self.set_world_tile(pos, tile);
                placed = placed.saturating_add(1);
            }
        }

        if let Some(actor_label) = actor_label {
            self.push_event(format!(
                "{} placed {} {} tiles to the right",
                actor_label, placed, resource
            ));
        } else {
            self.push_event(format!(
                "Server placed {} {} tiles to the right",
                placed, resource
            ));
        }
        Ok(())
    }

    pub fn give_resource(
        &mut self,
        target: &str,
        resource: &str,
        amount: u16,
    ) -> Result<(), String> {
        if amount == 0 {
            return Err("give amount must be greater than zero".to_string());
        }

        let resource_key = normalize_resource_key(resource)
            .ok_or_else(|| format!("unknown resource: {resource}"))?;

        let mut granted = 0usize;
        match target {
            "@a" => {
                for player in self.players.values_mut() {
                    add_inventory(&mut player.inventory, resource_key, amount);
                    granted += 1;
                }
                if granted == 0 {
                    return Err("no players matched target: @a".to_string());
                }
                self.players_dirty = true;
                self.push_event(format!(
                    "Granted {amount} {resource_key} to @a ({granted} players)"
                ));
                return Ok(());
            }
            "@e" => {
                for npc in &mut self.npcs {
                    add_inventory(&mut npc.inventory, resource_key, amount);
                    granted += 1;
                }
                if granted == 0 {
                    return Err("no NPCs matched target: @e".to_string());
                }
                self.npcs_dirty = true;
                self.push_event(format!(
                    "Granted {amount} {resource_key} to @e ({granted} NPCs)"
                ));
                return Ok(());
            }
            _ => {
                for player in self.players.values_mut() {
                    if player.name == target {
                        add_inventory(&mut player.inventory, resource_key, amount);
                        granted += 1;
                    }
                }
            }
        }

        if granted == 0 {
            return Err(format!("no players matched target: {target}"));
        }

        self.players_dirty = true;
        self.push_event(format!(
            "Granted {amount} {resource_key} to {target} ({granted} players)"
        ));
        Ok(())
    }

    pub fn feed_queens(&mut self, amount: u16) -> Result<(), String> {
        if amount == 0 {
            return Err("feed amount must be greater than zero".to_string());
        }

        let mut fed = 0usize;
        for npc in &mut self.npcs {
            if npc.kind != NpcKind::Queen {
                continue;
            }
            npc.food = npc
                .food
                .saturating_add(amount)
                .min(NpcKind::Queen.max_food());
            fed += 1;
        }

        if fed == 0 {
            return Err("no queens available to feed".to_string());
        }

        self.npcs_dirty = true;
        self.push_event(format!("Fed {fed} queen(s) with {amount} food"));
        Ok(())
    }

    pub fn set_queen_eggs(&mut self, eggs: u16) -> Result<(), String> {
        let egg_food_cost = config_u16(
            &self.config,
            "colony.queen_egg_food_cost",
            QUEEN_EGG_FOOD_COST,
        );

        let target_food = egg_food_cost
            .saturating_mul(eggs)
            .min(NpcKind::Queen.max_food());

        let mut updated = 0usize;
        for npc in &mut self.npcs {
            if npc.kind != NpcKind::Queen {
                continue;
            }
            npc.food = target_food;
            updated += 1;
        }

        if updated == 0 {
            return Err("no queens available to set eggs".to_string());
        }

        self.npcs_dirty = true;
        self.push_event(format!(
            "Set {updated} queen(s) to {eggs} egg(s) worth of food"
        ));
        Ok(())
    }

    pub fn kill_by_selector(&mut self, selector: &str) -> Result<(), String> {
        let parsed = parse_npc_selector(selector)?;
        let before = self.npcs.len();
        self.npcs.retain(|npc| !parsed.matches(npc));
        let killed = before.saturating_sub(self.npcs.len());
        if killed == 0 {
            return Err(format!("no NPCs matched selector: {selector}"));
        }
        self.npcs_dirty = true;
        self.push_event(format!("Killed {killed} NPC(s) matching {selector}"));
        Ok(())
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
            hive_id: None,
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
            Action::PlaceQueen => self.place_queen(player_id),
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
            PlaceMaterial::Food => "food",
            PlaceMaterial::Queen => return,
        };
        let tile = match material {
            PlaceMaterial::Dirt => Tile::Dirt,
            PlaceMaterial::Stone => Tile::Stone,
            PlaceMaterial::Food => Tile::Food,
            PlaceMaterial::Queen => return,
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

    fn place_queen(&mut self, player_id: u8) {
        self.dig_progress.remove(&player_id);

        let Some(player) = self.players.get(&player_id).cloned() else {
            return;
        };

        if player.pos.y <= SURFACE_Y {
            self.push_event(format!(
                "{} must place the queen in an underground cavern",
                player.name
            ));
            return;
        }

        if inventory_count(&player.inventory, "queen") == 0 {
            self.push_event(format!("{} has no queen to place", player.name));
            return;
        }

        let Some(asset) = find_ascii_art_asset(QUEEN_ART_ID) else {
            self.push_event("queen art asset is missing".to_string());
            return;
        };

        let preferred = Position {
            x: player.pos.x - asset.world_anchor_x(),
            y: player.pos.y - asset.anchor_y,
        };
        let Some(pos) = self.find_best_art_placement(asset, preferred) else {
            self.push_event(format!(
                "{} could not fit the queen in this cavern",
                player.name
            ));
            return;
        };

        let Some(player) = self.players.get_mut(&player_id) else {
            return;
        };
        let player_name = player.name.clone();
        let queen_food = player.inventory.get("food").copied().unwrap_or(0);
        let hive_id = player.hive_id.unwrap_or_else(|| {
            let hive_id = self.next_hive_id;
            self.next_hive_id = self.next_hive_id.saturating_add(1);
            player.hive_id = Some(hive_id);
            hive_id
        });
        if !remove_inventory(&mut player.inventory, "queen", 1) {
            let _ = player;
            self.push_event(format!("{player_name} has no queen to place"));
            return;
        }
        let _ = player.inventory.insert("food".to_string(), 0);
        let _ = player;

        let queen_pos = Position {
            x: pos.x + asset.world_anchor_x(),
            y: pos.y + asset.anchor_y,
        };
        self.placed_art.push(PlacedArt {
            asset_id: QUEEN_ART_ID.to_string(),
            pos,
            hive_id: Some(hive_id),
        });
        self.npcs.push(NpcAnt {
            id: self.next_npc_id,
            pos: queen_pos,
            inventory: default_npc_inventory(),
            kind: NpcKind::Queen,
            health: NpcKind::Queen.max_health(),
            food: queen_food.min(NpcKind::Queen.max_food()),
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
            role_state: NpcRoleState::None,
        });
        self.next_npc_id = self.next_npc_id.saturating_add(1);
        self.players_dirty = true;
        self.npcs_dirty = true;
        self.placed_art_dirty = true;
        self.push_event(format!(
            "{player_name} placed the queen with {} food",
            queen_food.min(NpcKind::Queen.max_food())
        ));
    }

    fn find_best_art_placement(
        &self,
        asset: &crate::AsciiArtAsset,
        preferred: Position,
    ) -> Option<Position> {
        let search_radius: i32 = 18;
        for radius in 0..=search_radius {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if radius != 0 && dx.abs().max(dy.abs()) != radius {
                        continue;
                    }
                    let candidate = preferred.offset(dx, dy);
                    if self.art_fits_at(asset, candidate) {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }

    fn art_fits_at(&self, asset: &crate::AsciiArtAsset, origin: Position) -> bool {
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
                if !self.world.in_bounds(pos) {
                    return false;
                }
                if !matches!(self.world.tile(pos), Some(Tile::Empty)) {
                    return false;
                }
                if self.art_occupies_cell(pos) {
                    return false;
                }
            }
        }
        true
    }
}

fn normalize_resource_key(resource: &str) -> Option<&'static str> {
    match resource {
        "d" | "dirt" => Some("dirt"),
        "o" | "ore" => Some("ore"),
        "s" | "stone" => Some("stone"),
        "f" | "food" => Some("food"),
        "q" | "queen" => Some("queen"),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct NpcSelector {
    kind: Option<NpcKind>,
    hive: Option<Option<u16>>,
}

impl NpcSelector {
    fn matches(self, npc: &NpcAnt) -> bool {
        if let Some(kind) = self.kind
            && npc.kind != kind
        {
            return false;
        }
        if let Some(hive) = self.hive
            && npc.hive_id != hive
        {
            return false;
        }
        true
    }
}

fn parse_npc_selector(selector: &str) -> Result<NpcSelector, String> {
    let trimmed = selector.trim();
    if trimmed == "@e" {
        return Ok(NpcSelector {
            kind: None,
            hive: None,
        });
    }
    let Some(rest) = trimmed.strip_prefix("@e[") else {
        return Err("expected selector like @e or @e[type=worker,hive=none]".to_string());
    };
    let Some(inner) = rest.strip_suffix(']') else {
        return Err("selector must end with ']'".to_string());
    };
    let mut parsed = NpcSelector {
        kind: None,
        hive: None,
    };
    for raw_filter in inner.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some((key, value)) = raw_filter.split_once('=') else {
            return Err(format!("invalid selector filter: {raw_filter}"));
        };
        match key.trim() {
            "type" | "kind" => {
                parsed.kind = Some(match value.trim() {
                    "worker" => NpcKind::Worker,
                    "queen" => NpcKind::Queen,
                    "egg" => NpcKind::Egg,
                    other => return Err(format!("unknown NPC type: {other}")),
                });
            }
            "hive" => {
                parsed.hive = Some(match value.trim() {
                    "none" => None,
                    raw => Some(
                        raw.parse::<u16>()
                            .map_err(|_| format!("invalid hive selector: {raw}"))?,
                    ),
                });
            }
            other => return Err(format!("unsupported selector key: {other}")),
        }
    }
    Ok(parsed)
}

fn put_resource_tile(resource: &str) -> Option<Tile> {
    match resource {
        "d" | "dirt" => Some(Tile::Dirt),
        "s" | "stone" => Some(Tile::Stone),
        "f" | "food" => Some(Tile::Food),
        "o" | "ore" | "resource" => Some(Tile::Resource),
        _ => None,
    }
}
