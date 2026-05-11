use std::collections::{HashSet, VecDeque};

use crate::{
    game_state::GameState,
    types::{Position, Tile},
};

pub(crate) fn choose_clockwise_ring_step(
    game: &mut GameState,
    index: usize,
    center: Position,
    ring: &[Position],
) -> Option<Position> {
    if ring.is_empty() {
        game.npcs[index].search_destination = None;
        return None;
    }

    let origin = game.npcs[index].pos;
    let current_index = ring.iter().position(|pos| *pos == origin);
    let mut target_index = game.npcs[index]
        .search_destination
        .and_then(|target| ring.iter().position(|pos| *pos == target));

    if let Some(current_index) = current_index {
        let default_next = (current_index + 1) % ring.len();
        if target_index.is_none() || target_index == Some(current_index) {
            target_index = Some(default_next);
        }
        for offset in 0..ring.len() {
            let candidate_index = (target_index.unwrap_or(default_next) + offset) % ring.len();
            let target = ring[candidate_index];
            if let Some(step) = bfs_first_step(game, index, origin, target, center) {
                game.npcs[index].search_destination = Some(target);
                return Some(step);
            }
        }
        game.npcs[index].search_destination = None;
        return None;
    }

    if let Some(target_index) = target_index {
        let target = ring[target_index];
        if let Some(step) = bfs_first_step(game, index, origin, target, center) {
            game.npcs[index].search_destination = Some(target);
            return Some(step);
        }
    }

    let (step, target) = bfs_to_any_ring_cell(game, index, origin, ring, center)?;
    game.npcs[index].search_destination = Some(target);
    Some(step)
}

pub(crate) fn perimeter(
    center: Position,
    radius_x: i32,
    radius_y: i32,
    world_width: i32,
    world_height: i32,
) -> Vec<Position> {
    let min_x = (center.x - radius_x - 1).max(0);
    let max_x = (center.x + radius_x + 1).min(world_width.saturating_sub(1));
    let min_y = (center.y - radius_y - 1).max(0);
    let max_y = (center.y + radius_y + 1).min(world_height.saturating_sub(1));
    let mut ring = Vec::new();

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let pos = Position { x, y };
            let value = ellipse_boundary_value(pos, center, radius_x, radius_y);
            if value > 1.0 {
                continue;
            }
            let touches_outside = [
                pos.offset(-1, 0),
                pos.offset(1, 0),
                pos.offset(0, -1),
                pos.offset(0, 1),
            ]
            .into_iter()
            .any(|neighbor| {
                !in_ellipse_bounds(neighbor, world_width, world_height)
                    || ellipse_boundary_value(neighbor, center, radius_x, radius_y) > 1.0
            });
            if touches_outside {
                ring.push(pos);
            }
        }
    }

    ring.sort_by(|left, right| {
        clockwise_angle(center, *left)
            .total_cmp(&clockwise_angle(center, *right))
            .then_with(|| {
                ellipse_boundary_value(*right, center, radius_x, radius_y)
                    .total_cmp(&ellipse_boundary_value(*left, center, radius_x, radius_y))
            })
            .then_with(|| left.y.cmp(&right.y))
            .then_with(|| left.x.cmp(&right.x))
    });
    ring.dedup();
    ring
}

fn tile_traversable(game: &GameState, index: usize, pos: Position) -> bool {
    if !game.world.in_bounds(pos)
        || game.players.values().any(|player| player.pos == pos)
        || game.npc_blocks_movement(pos, index)
    {
        return false;
    }
    matches!(
        game.world.tile(pos),
        Some(Tile::Empty | Tile::Dirt | Tile::Resource | Tile::Food)
    )
}

fn bfs_first_step(
    game: &GameState,
    index: usize,
    origin: Position,
    destination: Position,
    center: Position,
) -> Option<Position> {
    if origin == destination {
        return None;
    }

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(origin);
    queue.push_back((origin, None::<Position>));

    while let Some((pos, first_step)) = queue.pop_front() {
        for next in neighbor_order(pos, center) {
            if visited.contains(&next) || !tile_traversable(game, index, next) {
                continue;
            }
            let first_step = first_step.or(Some(next));
            if next == destination {
                return first_step;
            }
            visited.insert(next);
            queue.push_back((next, first_step));
        }
    }

    None
}

fn bfs_to_any_ring_cell(
    game: &GameState,
    index: usize,
    origin: Position,
    ring: &[Position],
    center: Position,
) -> Option<(Position, Position)> {
    let ring_positions: HashSet<_> = ring.iter().copied().collect();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(origin);
    queue.push_back((origin, None::<Position>));

    while let Some((pos, first_step)) = queue.pop_front() {
        for next in neighbor_order(pos, center) {
            if visited.contains(&next) || !tile_traversable(game, index, next) {
                continue;
            }
            let first_step = first_step.or(Some(next));
            if ring_positions.contains(&next) {
                return first_step.map(|step| (step, next));
            }
            visited.insert(next);
            queue.push_back((next, first_step));
        }
    }

    None
}

fn in_ellipse_bounds(pos: Position, world_width: i32, world_height: i32) -> bool {
    pos.x >= 0 && pos.y >= 0 && pos.x < world_width && pos.y < world_height
}

fn clockwise_tangent(current: Position, center: Position) -> (i32, i32) {
    let radial_x = current.x - center.x;
    let radial_y = current.y - center.y;
    let tangent = (-radial_y, radial_x);
    if tangent.0 == 0 && tangent.1 == 0 {
        (1, 0)
    } else {
        tangent
    }
}

fn neighbor_order(current: Position, center: Position) -> [Position; 4] {
    let tangent = clockwise_tangent(current, center);
    let radial = (current.x - center.x, current.y - center.y);
    let clockwise = unit_cardinal_step(tangent);
    let inward = unit_cardinal_step((-radial.0, -radial.1));
    let outward = unit_cardinal_step(radial);
    let counter_clockwise = (-clockwise.0, -clockwise.1);
    [
        current.offset(clockwise.0, clockwise.1),
        current.offset(inward.0, inward.1),
        current.offset(outward.0, outward.1),
        current.offset(counter_clockwise.0, counter_clockwise.1),
    ]
}

fn unit_cardinal_step((dx, dy): (i32, i32)) -> (i32, i32) {
    if dx == 0 && dy == 0 {
        return (1, 0);
    }
    if dx.abs() >= dy.abs() {
        (dx.signum(), 0)
    } else {
        (0, dy.signum())
    }
}

fn clockwise_angle(center: Position, pos: Position) -> f64 {
    let dx = f64::from(pos.x - center.x);
    let dy = f64::from(center.y - pos.y);
    let angle = dx.atan2(dy);
    if angle < 0.0 {
        angle + std::f64::consts::TAU
    } else {
        angle
    }
}

fn ellipse_boundary_value(pos: Position, center: Position, radius_x: i32, radius_y: i32) -> f64 {
    let dx = f64::from(pos.x - center.x);
    let dy = f64::from(pos.y - center.y);
    let rx = f64::from(radius_x.max(1));
    let ry = f64::from(radius_y.max(1));
    ((dx * dx) / (rx * rx)) + ((dy * dy) / (ry * ry))
}
