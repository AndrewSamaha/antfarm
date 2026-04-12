use crate::{
    types::{NpcAnt, Position},
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
        },
        NpcAnt {
            id: 2,
            pos: Position {
                x: 120,
                y: surface_2.min(world.height() - 2),
            },
        },
    ]
}

pub(crate) fn nearest_target(origin: Position, positions: &[Position]) -> Option<Position> {
    positions
        .iter()
        .copied()
        .min_by_key(|pos| (pos.x - origin.x).abs() + (pos.y - origin.y).abs())
}
