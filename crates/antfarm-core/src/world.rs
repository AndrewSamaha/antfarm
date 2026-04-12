use serde::{Deserialize, Serialize};

use crate::{
    generation::generate_world,
    types::{Position, Tile},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    width: i32,
    height: i32,
    tiles: Vec<Tile>,
}

impl World {
    pub fn empty(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            tiles: vec![Tile::Empty; (width * height) as usize],
        }
    }

    pub fn generate(seed: u64, width: i32, config: &serde_json::Value) -> Self {
        generate_world(seed, width, config)
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

    pub fn row_tiles(&self, row: i32) -> Vec<Tile> {
        if row < 0 || row >= self.height {
            return Vec::new();
        }
        (0..self.width)
            .filter_map(|x| self.tile(Position { x, y: row }))
            .collect()
    }

    pub fn set_row_tiles(&mut self, row: i32, tiles: &[Tile]) {
        if row < 0 || row >= self.height {
            return;
        }
        for (x, tile) in tiles.iter().enumerate() {
            if x as i32 >= self.width {
                break;
            }
            let _ = self.set_tile(
                Position {
                    x: x as i32,
                    y: row,
                },
                *tile,
            );
        }
    }

    pub fn is_walkable(&self, pos: Position) -> bool {
        matches!(self.tile(pos), Some(Tile::Empty))
    }

    pub fn spawn_y_for_column(&self, x: i32) -> i32 {
        for y in 0..self.height {
            if self.tile(Position { x, y }) != Some(Tile::Empty) {
                return y.saturating_sub(1);
            }
        }
        0
    }
}
