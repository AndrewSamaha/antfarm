use crate::{
    inventory::default_npc_inventory,
    pheromones::AntBehaviorState,
    types::{NpcAnt, NpcKind, Position},
    world::World,
};

pub(crate) fn default_npcs(world: &World) -> Vec<NpcAnt> {
    let surface_1 = world.spawn_y_for_column(20).saturating_add(1);
    let surface_2 = world.spawn_y_for_column(120).saturating_add(3);
    vec![
        NpcAnt {
            id: 1,
            pos: Position {
                x: 20,
                y: surface_1.min(world.height() - 2),
            },
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
        },
        NpcAnt {
            id: 2,
            pos: Position {
                x: 120,
                y: surface_2.min(world.height() - 2),
            },
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
        },
    ]
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
