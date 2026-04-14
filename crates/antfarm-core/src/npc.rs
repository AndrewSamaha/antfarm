use crate::{
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
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 0,
            hive_id: None,
            age_ticks: 0,
        },
        NpcAnt {
            id: 2,
            pos: Position {
                x: 120,
                y: surface_2.min(world.height() - 2),
            },
            kind: NpcKind::Worker,
            health: NpcKind::Worker.max_health(),
            food: 0,
            hive_id: None,
            age_ticks: 0,
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
