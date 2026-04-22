use crate::{
    inventory::default_npc_inventory,
    pheromones::AntBehaviorState,
    types::{NpcAnt, NpcKind, Position},
    world::World,
};

pub(crate) fn default_npcs_with_count(world: &World, count: u16) -> Vec<NpcAnt> {
    if count == 0 {
        return Vec::new();
    }

    let usable_width = (world.width() - 16).max(1);
    let divisor = i32::from(count).saturating_add(1).max(1);
    let mut npcs = Vec::with_capacity(usize::from(count));
    for index in 0..count {
        let slot = i32::from(index).saturating_add(1);
        let x = 8 + (usable_width * slot) / divisor;
        let y_offset = if index % 2 == 0 { 1 } else { 3 };
        let y = world
            .spawn_y_for_column(x)
            .saturating_add(y_offset)
            .min(world.height() - 2);
        npcs.push(NpcAnt {
            id: index.saturating_add(1),
            pos: Position { x, y },
            inventory: default_npc_inventory(),
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 0,
            hive_id: None,
            age_ticks: 0,
            behavior: AntBehaviorState::Searching,
            carrying_food: false,
            carrying_food_ticks: 0,
            home_trail_steps: None,
            recent_home_dir: None,
            recent_food_dir: None,
            recent_home_memory_ticks: 0,
            recent_food_memory_ticks: 0,
            recent_positions: Vec::new(),
            last_dirt_place_tick: None,
        });
    }
    npcs
}

pub(crate) fn nearest_open_tile(world: &World, occupied: &[Position], origin: Position) -> Option<Position> {
    for radius in 1_i32..=6_i32 {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs().max(dy.abs()) != radius {
                    continue;
                }
                let pos = origin.offset(dx, dy);
                if !world.in_bounds(pos) {
                    continue;
                }
                if world.tile(pos) != Some(crate::Tile::Empty) {
                    continue;
                }
                if occupied.contains(&pos) {
                    continue;
                }
                return Some(pos);
            }
        }
    }
    None
}
