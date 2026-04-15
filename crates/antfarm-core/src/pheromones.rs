use serde::{Deserialize, Serialize};

use crate::types::Position;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PheromoneChannel {
    Home,
    Food,
    Threat,
    Defense,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AntBehaviorState {
    #[default]
    Searching,
    ReturningFood,
    Defending,
    Idle,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HivePheromone {
    pub hive_id: u16,
    pub home: u8,
    pub food: u8,
    pub threat: u8,
    pub defense: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PheromoneCell {
    pub entries: Vec<HivePheromone>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PheromoneGrid {
    width: i32,
    height: i32,
    cells: Vec<PheromoneCell>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PheromoneMap {
    pub width: i32,
    pub height: i32,
    pub hive_id: u16,
    pub channel: PheromoneChannel,
    pub values: Vec<u8>,
}

impl PheromoneGrid {
    pub fn empty(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            cells: vec![PheromoneCell::default(); (width * height).max(0) as usize],
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

    pub fn value(&self, pos: Position, hive_id: u16, channel: PheromoneChannel) -> u8 {
        let Some(cell) = self.cell(pos) else {
            return 0;
        };
        let Some(entry) = cell.entries.iter().find(|entry| entry.hive_id == hive_id) else {
            return 0;
        };
        channel_value(entry, channel)
    }

    pub fn deposit(&mut self, pos: Position, hive_id: u16, channel: PheromoneChannel, amount: u8) {
        if amount == 0 {
            return;
        }
        let Some(cell) = self.cell_mut(pos) else {
            return;
        };
        let entry = match cell.entries.iter_mut().find(|entry| entry.hive_id == hive_id) {
            Some(entry) => entry,
            None => {
                cell.entries.push(HivePheromone {
                    hive_id,
                    ..HivePheromone::default()
                });
                cell.entries.last_mut().expect("entry was just pushed")
            }
        };
        let value = channel_value_mut(entry, channel);
        *value = value.saturating_add(amount);
    }

    pub fn emit_radius(
        &mut self,
        origin: Position,
        hive_id: u16,
        channel: PheromoneChannel,
        radius: i32,
        peak: u8,
    ) {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let distance = dx.abs() + dy.abs();
                if distance > radius {
                    continue;
                }
                let pos = origin.offset(dx, dy);
                if !self.in_bounds(pos) {
                    continue;
                }
                let falloff = distance as u8;
                let amount = peak.saturating_sub(falloff.saturating_mul(peak.max(1) / (radius.max(1) as u8 + 1)));
                if amount > 0 {
                    self.deposit(pos, hive_id, channel, amount);
                }
            }
        }
    }

    pub fn decay_all(&mut self, amount: u8) {
        if amount == 0 {
            return;
        }
        for cell in &mut self.cells {
            for entry in &mut cell.entries {
                entry.home = entry.home.saturating_sub(amount);
                entry.food = entry.food.saturating_sub(amount);
                entry.threat = entry.threat.saturating_sub(amount);
                entry.defense = entry.defense.saturating_sub(amount);
            }
            cell.entries.retain(|entry| {
                entry.home > 0 || entry.food > 0 || entry.threat > 0 || entry.defense > 0
            });
        }
    }

    pub fn export_map(&self, hive_id: u16, channel: PheromoneChannel) -> PheromoneMap {
        let mut values = Vec::with_capacity(self.cells.len());
        for y in 0..self.height {
            for x in 0..self.width {
                values.push(self.value(Position { x, y }, hive_id, channel));
            }
        }
        PheromoneMap {
            width: self.width,
            height: self.height,
            hive_id,
            channel,
            values,
        }
    }

    fn cell(&self, pos: Position) -> Option<&PheromoneCell> {
        self.in_bounds(pos)
            .then(|| &self.cells[(pos.y * self.width + pos.x) as usize])
    }

    fn cell_mut(&mut self, pos: Position) -> Option<&mut PheromoneCell> {
        self.in_bounds(pos)
            .then(|| &mut self.cells[(pos.y * self.width + pos.x) as usize])
    }
}

fn channel_value(entry: &HivePheromone, channel: PheromoneChannel) -> u8 {
    match channel {
        PheromoneChannel::Home => entry.home,
        PheromoneChannel::Food => entry.food,
        PheromoneChannel::Threat => entry.threat,
        PheromoneChannel::Defense => entry.defense,
    }
}

fn channel_value_mut(entry: &mut HivePheromone, channel: PheromoneChannel) -> &mut u8 {
    match channel {
        PheromoneChannel::Home => &mut entry.home,
        PheromoneChannel::Food => &mut entry.food,
        PheromoneChannel::Threat => &mut entry.threat,
        PheromoneChannel::Defense => &mut entry.defense,
    }
}
