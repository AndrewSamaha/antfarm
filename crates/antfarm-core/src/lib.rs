use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_PLAYERS: usize = 5;
pub const WORLD_WIDTH: i32 = 160;
pub const WORLD_HEIGHT: i32 = 80;
pub const SURFACE_Y: i32 = 18;
pub const TICK_MILLIS: u64 = 120;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
}

impl Tile {
    pub fn glyph(self) -> char {
        match self {
            Self::Empty => ' ',
            Self::Dirt => '.',
            Self::Stone => '#',
            Self::Resource => '*',
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
    pub dirt: u16,
    pub resources: u16,
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
    pub fn new(seed: u64, width: i32, height: i32) -> Self {
        let mut tiles = vec![Tile::Empty; (width * height) as usize];
        let mut rng = StdRng::seed_from_u64(seed);

        for y in 0..height {
            for x in 0..width {
                let tile = if y < SURFACE_Y {
                    Tile::Empty
                } else if y == SURFACE_Y {
                    Tile::Dirt
                } else {
                    let roll = rng.random_range(0..100);
                    if roll < 7 {
                        Tile::Stone
                    } else if roll < 11 {
                        Tile::Resource
                    } else {
                        Tile::Dirt
                    }
                };
                tiles[(y * width + x) as usize] = tile;
            }
        }

        Self {
            width,
            height,
            tiles,
        }
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

    pub fn is_walkable(&self, pos: Position) -> bool {
        matches!(self.tile(pos), Some(Tile::Empty))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub world: World,
    pub players: Vec<Player>,
    pub npcs: Vec<NpcAnt>,
    pub event_log: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinAck {
    pub player_id: u8,
    pub snapshot: Snapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    Join { name: String },
    Action(Action),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    Joined(JoinAck),
    Snapshot(Snapshot),
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    Move(MoveDir),
    Dig(MoveDir),
    Place(MoveDir),
}

#[derive(Debug, Clone)]
pub struct GameState {
    pub tick: u64,
    pub world: World,
    pub players: HashMap<u8, Player>,
    pub npcs: Vec<NpcAnt>,
    pub event_log: Vec<String>,
    next_player_id: u8,
}

impl GameState {
    pub fn new() -> Self {
        let world = World::new(7, WORLD_WIDTH, WORLD_HEIGHT);
        let npcs = vec![
            NpcAnt {
                id: 1,
                pos: Position {
                    x: 20,
                    y: SURFACE_Y,
                },
            },
            NpcAnt {
                id: 2,
                pos: Position {
                    x: 120,
                    y: SURFACE_Y + 2,
                },
            },
        ];

        Self {
            tick: 0,
            world,
            players: HashMap::new(),
            npcs,
            event_log: vec!["Server booted ant colony".to_string()],
            next_player_id: 1,
        }
    }

    pub fn add_player(&mut self, name: String) -> Result<(u8, Snapshot), String> {
        if self.players.len() >= MAX_PLAYERS {
            return Err(format!("Room full: max {} players", MAX_PLAYERS));
        }

        let player_id = self.next_player_id;
        self.next_player_id = self.next_player_id.saturating_add(1);

        let spawn_x = 8 + (self.players.len() as i32 * 6);
        let player = Player {
            id: player_id,
            name: name.clone(),
            pos: Position {
                x: spawn_x.min(self.world.width() - 2),
                y: SURFACE_Y - 1,
            },
            facing: Facing::Right,
            dirt: 8,
            resources: 0,
        };

        self.players.insert(player_id, player);
        self.push_event(format!("{name} joined as ant {player_id}"));
        let snapshot = self.snapshot();
        Ok((player_id, snapshot))
    }

    pub fn remove_player(&mut self, player_id: u8) {
        if let Some(player) = self.players.remove(&player_id) {
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
        }
    }

    pub fn apply_action(&mut self, player_id: u8, action: Action) {
        match action {
            Action::Move(dir) => self.move_player(player_id, dir),
            Action::Dig(dir) => self.dig(player_id, dir),
            Action::Place(dir) => self.place(player_id, dir),
        }
    }

    pub fn tick(&mut self) {
        self.tick += 1;

        let player_positions: Vec<_> = self.players.values().map(|player| player.pos).collect();
        let mut events = Vec::new();
        for npc in &mut self.npcs {
            let Some(target) = nearest_target(npc.pos, &player_positions) else {
                continue;
            };

            let dx = (target.x - npc.pos.x).signum();
            let dy = (target.y - npc.pos.y).signum();

            for next in [npc.pos.offset(dx, 0), npc.pos.offset(0, dy)] {
                if !self.world.in_bounds(next) {
                    continue;
                }
                match self.world.tile(next) {
                    Some(Tile::Empty) => {
                        npc.pos = next;
                        break;
                    }
                    Some(Tile::Dirt) | Some(Tile::Resource) => {
                        self.world.set_tile(next, Tile::Empty);
                        events.push(format!(
                            "NPC ant {} tunneled at {},{}",
                            npc.id, next.x, next.y
                        ));
                        break;
                    }
                    Some(Tile::Stone) | None => {}
                }
            }
        }

        let npc_positions: Vec<_> = self.npcs.iter().map(|npc| npc.pos).collect();
        for player in self.players.values_mut() {
            if npc_positions.contains(&player.pos) {
                player.dirt = player.dirt.saturating_sub(1);
                events.push(format!("{} was disturbed by an NPC ant", player.name));
            }
        }

        for event in events {
            self.push_event(event);
        }
    }

    fn move_player(&mut self, player_id: u8, dir: MoveDir) {
        let Some(current) = self.players.get(&player_id).cloned() else {
            return;
        };

        let (dx, dy) = dir.delta();
        let next = current.pos.offset(dx, dy);
        if !self.world.in_bounds(next) {
            return;
        }

        let target_tile = self.world.tile(next);
        if matches!(target_tile, Some(Tile::Empty)) && !self.occupied_by_actor(next) {
            if let Some(player) = self.players.get_mut(&player_id) {
                player.pos = next;
                if dx < 0 {
                    player.facing = Facing::Left;
                } else if dx > 0 {
                    player.facing = Facing::Right;
                }
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
            return;
        };

        if tile == Tile::Stone {
            self.push_event(format!("{} hit solid stone", current.name));
            return;
        }
        if tile == Tile::Empty {
            return;
        }

        self.world.set_tile(target, Tile::Empty);
        let mut event = None;
        if let Some(player) = self.players.get_mut(&player_id) {
            player.dirt = player.dirt.saturating_add(1);
            if tile == Tile::Resource {
                player.resources = player.resources.saturating_add(1);
                event = Some(format!("{} found a resource vein", player.name));
            }
        }
        if let Some(event) = event {
            self.push_event(event);
        }
    }

    fn place(&mut self, player_id: u8, dir: MoveDir) {
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
        if player.dirt == 0 {
            let name = player.name.clone();
            let _ = player;
            self.push_event(format!("{name} has no dirt to place"));
            return;
        }

        player.dirt -= 1;
        self.world.set_tile(target, Tile::Dirt);
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
    }
}

fn nearest_target(origin: Position, positions: &[Position]) -> Option<Position> {
    positions
        .iter()
        .copied()
        .min_by_key(|pos| (pos.x - origin.x).abs() + (pos.y - origin.y).abs())
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
