use std::collections::{HashSet, VecDeque};

use rand::Rng;

use crate::{
    NpcDebugEvent,
    game_state::GameState,
    inventory::add_inventory,
    types::{Position, QueenChamberGrowthMode, QueenChamberState, Tile},
};

const INITIAL_QUEEN_CHAMBER_RADIUS: i32 = 2;

pub(crate) fn tick(
    game: &mut GameState,
    index: usize,
    queen_pos: Option<Position>,
    events: &mut Vec<String>,
) {
    game.set_worker_idle(index);
    let Some(queen_pos) = queen_pos else {
        return;
    };

    let npc_pos = game.npcs[index].pos;
    let queen_distance = floor_euclidean_distance(npc_pos, queen_pos);
    let next_step = choose_step(game, index, queen_pos);
    let Some(next_step) = next_step else {
        game.push_npc_debug_event(NpcDebugEvent {
            tick: game.tick,
            npc_id: game.npcs[index].id,
            hive_id: game.npcs[index].hive_id,
            event_type: "queen_chamber_hold".to_string(),
            pos: npc_pos,
            details: serde_json::json!({
                "queen_pos": { "x": queen_pos.x, "y": queen_pos.y },
                "queen_distance": queen_distance,
            }),
        });
        return;
    };

    match game.world.tile(next_step) {
        Some(Tile::Empty) => {
            game.npcs[index].pos = next_step;
            remember_recent_position(&mut game.npcs[index].recent_positions, next_step);
            game.mark_npcs_dirty();
            game.push_npc_debug_event(NpcDebugEvent {
                tick: game.tick,
                npc_id: game.npcs[index].id,
                hive_id: game.npcs[index].hive_id,
                event_type: "queen_chamber_move".to_string(),
                pos: npc_pos,
                details: serde_json::json!({
                    "next_step": { "x": next_step.x, "y": next_step.y },
                    "queen_pos": { "x": queen_pos.x, "y": queen_pos.y },
                    "queen_distance_before": queen_distance,
                    "queen_distance_after": floor_euclidean_distance(next_step, queen_pos),
                }),
            });
        }
        Some(Tile::Dirt) | Some(Tile::Resource) | Some(Tile::Food) => {
            let traversed_tile = game.world.tile(next_step).unwrap_or(Tile::Empty);
            match traversed_tile {
                Tile::Dirt => add_inventory(&mut game.npcs[index].inventory, "dirt", 1),
                Tile::Resource => add_inventory(&mut game.npcs[index].inventory, "ore", 1),
                _ => {}
            };
            game.set_world_tile(next_step, Tile::Empty);
            game.npcs[index].pos = next_step;
            remember_recent_position(&mut game.npcs[index].recent_positions, next_step);
            game.mark_npcs_dirty();
            events.push(format!(
                "NPC ant {} cleared queen chamber path at {},{}",
                game.npcs[index].id, next_step.x, next_step.y
            ));
            game.push_npc_debug_event(NpcDebugEvent {
                tick: game.tick,
                npc_id: game.npcs[index].id,
                hive_id: game.npcs[index].hive_id,
                event_type: "queen_chamber_dig".to_string(),
                pos: npc_pos,
                details: serde_json::json!({
                    "target": { "x": next_step.x, "y": next_step.y },
                    "queen_pos": { "x": queen_pos.x, "y": queen_pos.y },
                    "queen_distance_before": queen_distance,
                }),
            });
            game.push_npc_debug_event(NpcDebugEvent {
                tick: game.tick,
                npc_id: game.npcs[index].id,
                hive_id: game.npcs[index].hive_id,
                event_type: "queen_chamber_move".to_string(),
                pos: npc_pos,
                details: serde_json::json!({
                    "next_step": { "x": next_step.x, "y": next_step.y },
                    "queen_pos": { "x": queen_pos.x, "y": queen_pos.y },
                    "queen_distance_before": queen_distance,
                    "queen_distance_after": floor_euclidean_distance(next_step, queen_pos),
                    "moved_through": tile_name(traversed_tile),
                }),
            });
        }
        Some(Tile::Stone | Tile::Bedrock) | None => {}
    }
}

pub(crate) fn on_hatch(game: &mut GameState, index: usize) {
    let (max_x, max_y) = game.queen_chamber_max_radii();
    let growth_mode = game.next_queen_chamber_growth_mode();
    let (radius_x, radius_y) = queen_chamber_initial_radii_for_mode(growth_mode, max_x, max_y);
    let worker = &mut game.npcs[index];
    worker.behavior = crate::AntBehaviorState::Idle;
    worker.home_trail_steps = None;
    worker.set_queen_chamber_state(QueenChamberState {
        radius_x: Some(radius_x),
        radius_y: Some(radius_y),
        anchor: None,
        has_left_anchor: false,
        growth_mode,
    });
}

pub(crate) fn random_queen_chamber_growth_mode<R: Rng + ?Sized>(
    rng: &mut R,
) -> QueenChamberGrowthMode {
    if rng.random::<bool>() {
        QueenChamberGrowthMode::Outward
    } else {
        QueenChamberGrowthMode::Inward
    }
}

pub(crate) fn queen_chamber_initial_radii_for_mode(
    mode: QueenChamberGrowthMode,
    max_x: i32,
    max_y: i32,
) -> (i32, i32) {
    let initial_radius_x = INITIAL_QUEEN_CHAMBER_RADIUS.min(max_x);
    let initial_radius_y = INITIAL_QUEEN_CHAMBER_RADIUS.min(max_y);
    match mode {
        QueenChamberGrowthMode::Outward => (initial_radius_x, initial_radius_y),
        QueenChamberGrowthMode::Inward => (max_x, max_y),
    }
}

fn choose_step(game: &mut GameState, index: usize, queen_pos: Position) -> Option<Position> {
    let origin = game.npcs[index].pos;
    ensure_radii_initialized(game, index);
    let (mut radius_x, mut radius_y) = current_radii(game, index);
    let mut ring = perimeter(queen_pos, radius_x, radius_y, game.world.width(), game.world.height());
    if ring.is_empty() {
        game.npcs[index].search_destination = None;
        return None;
    }

    let current_index = ring.iter().position(|pos| *pos == origin);
    let mut target_index = game.npcs[index]
        .search_destination
        .and_then(|target| ring.iter().position(|pos| *pos == target));

    if let Some(initial_current_index) = current_index {
        update_growth_state(game, index, origin, &ring);
        let (updated_radius_x, updated_radius_y) = current_radii(game, index);
        let current_index = if updated_radius_x != radius_x || updated_radius_y != radius_y {
            radius_x = updated_radius_x;
            radius_y = updated_radius_y;
            ring = perimeter(
                queen_pos,
                radius_x,
                radius_y,
                game.world.width(),
                game.world.height(),
            );
            target_index = None;
            if ring.is_empty() {
                game.npcs[index].search_destination = None;
                return None;
            }
            match ring.iter().position(|pos| *pos == origin) {
                Some(current_index) => current_index,
                None => {
                    let (step, target) = bfs_to_any_ring_cell(game, index, origin, &ring, queen_pos)?;
                    game.npcs[index].search_destination = Some(target);
                    return Some(step);
                }
            }
        } else {
            initial_current_index
        };
        let default_next = (current_index + 1) % ring.len();
        if target_index.is_none() || target_index == Some(current_index) {
            target_index = Some(default_next);
        }
        for offset in 0..ring.len() {
            let candidate_index = (target_index.unwrap_or(default_next) + offset) % ring.len();
            let target = ring[candidate_index];
            if let Some(step) = bfs_first_step(game, index, origin, target, queen_pos) {
                game.npcs[index].search_destination = Some(target);
                return Some(step);
            }
        }
        game.npcs[index].search_destination = None;
        return None;
    }

    if let Some(target_index) = target_index {
        let target = ring[target_index];
        if let Some(step) = bfs_first_step(game, index, origin, target, queen_pos) {
            game.npcs[index].search_destination = Some(target);
            return Some(step);
        }
    }

    let (step, target) = bfs_to_any_ring_cell(game, index, origin, &ring, queen_pos)?;
    game.npcs[index].search_destination = Some(target);
    Some(step)
}

fn ensure_radii_initialized(game: &mut GameState, index: usize) {
    let state = game.npcs[index].queen_chamber_state();
    if state.radius_x.is_some() && state.radius_y.is_some() {
        return;
    }
    let (max_x, max_y) = game.queen_chamber_max_radii();
    let (radius_x, radius_y) = queen_chamber_initial_radii_for_mode(state.growth_mode, max_x, max_y);
    game.npcs[index].set_queen_chamber_state(QueenChamberState {
        radius_x: Some(radius_x),
        radius_y: Some(radius_y),
        anchor: None,
        has_left_anchor: false,
        growth_mode: state.growth_mode,
    });
}

fn current_radii(game: &GameState, index: usize) -> (i32, i32) {
    let state = game.npcs[index].queen_chamber_state();
    (
        state.radius_x.unwrap_or(INITIAL_QUEEN_CHAMBER_RADIUS),
        state.radius_y.unwrap_or(INITIAL_QUEEN_CHAMBER_RADIUS),
    )
}

fn update_growth_state(game: &mut GameState, index: usize, pos: Position, ring: &[Position]) {
    if !ring.contains(&pos) {
        return;
    }
    let state = game.npcs[index].queen_chamber_state();
    match state.anchor {
        None => {
            game.npcs[index].set_queen_chamber_state(QueenChamberState {
                anchor: Some(pos),
                has_left_anchor: false,
                ..state
            });
        }
        Some(anchor) if !state.has_left_anchor && pos != anchor => {
            game.npcs[index].set_queen_chamber_state(QueenChamberState {
                has_left_anchor: true,
                ..state
            });
        }
        Some(anchor) if state.has_left_anchor && pos == anchor => {
            let (max_x, max_y) = game.queen_chamber_max_radii();
            let min_x = INITIAL_QUEEN_CHAMBER_RADIUS.min(max_x);
            let min_y = INITIAL_QUEEN_CHAMBER_RADIUS.min(max_y);
            let current_x = state.radius_x.unwrap_or(INITIAL_QUEEN_CHAMBER_RADIUS);
            let current_y = state.radius_y.unwrap_or(INITIAL_QUEEN_CHAMBER_RADIUS);
            let (next_x, next_y) = match state.growth_mode {
                QueenChamberGrowthMode::Outward => {
                    ((current_x + 1).min(max_x), (current_y + 1).min(max_y))
                }
                QueenChamberGrowthMode::Inward => {
                    ((current_x - 1).max(min_x), (current_y - 1).max(min_y))
                }
            };
            game.npcs[index].set_queen_chamber_state(QueenChamberState {
                radius_x: Some(next_x),
                radius_y: Some(next_y),
                anchor: None,
                has_left_anchor: false,
                growth_mode: state.growth_mode,
            });
            game.npcs[index].search_destination = None;
        }
        _ => {}
    }
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
    queen_pos: Position,
) -> Option<Position> {
    if origin == destination {
        return None;
    }

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(origin);
    queue.push_back((origin, None::<Position>));

    while let Some((pos, first_step)) = queue.pop_front() {
        for next in neighbor_order(pos, queen_pos) {
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
    queen_pos: Position,
) -> Option<(Position, Position)> {
    let ring_positions: HashSet<_> = ring.iter().copied().collect();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(origin);
    queue.push_back((origin, None::<Position>));

    while let Some((pos, first_step)) = queue.pop_front() {
        for next in neighbor_order(pos, queen_pos) {
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

fn perimeter(
    queen: Position,
    radius_x: i32,
    radius_y: i32,
    world_width: i32,
    world_height: i32,
) -> Vec<Position> {
    let min_x = (queen.x - radius_x - 1).max(0);
    let max_x = (queen.x + radius_x + 1).min(world_width.saturating_sub(1));
    let min_y = (queen.y - radius_y - 1).max(0);
    let max_y = (queen.y + radius_y + 1).min(world_height.saturating_sub(1));
    let mut ring = Vec::new();

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let pos = Position { x, y };
            let value = ellipse_boundary_value(pos, queen, radius_x, radius_y);
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
                    || ellipse_boundary_value(neighbor, queen, radius_x, radius_y) > 1.0
            });
            if touches_outside {
                ring.push(pos);
            }
        }
    }

    ring.sort_by(|left, right| {
        clockwise_angle(queen, *left)
            .total_cmp(&clockwise_angle(queen, *right))
            .then_with(|| {
                ellipse_boundary_value(*right, queen, radius_x, radius_y)
                    .total_cmp(&ellipse_boundary_value(*left, queen, radius_x, radius_y))
            })
            .then_with(|| left.y.cmp(&right.y))
            .then_with(|| left.x.cmp(&right.x))
    });
    ring.dedup();
    ring
}

fn in_ellipse_bounds(pos: Position, world_width: i32, world_height: i32) -> bool {
    pos.x >= 0 && pos.y >= 0 && pos.x < world_width && pos.y < world_height
}

fn clockwise_tangent(current: Position, queen: Position) -> (i32, i32) {
    let radial_x = current.x - queen.x;
    let radial_y = current.y - queen.y;
    let tangent = (-radial_y, radial_x);
    if tangent.0 == 0 && tangent.1 == 0 {
        (1, 0)
    } else {
        tangent
    }
}

fn neighbor_order(current: Position, queen: Position) -> [Position; 4] {
    let tangent = clockwise_tangent(current, queen);
    let radial = (current.x - queen.x, current.y - queen.y);
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

fn floor_euclidean_distance(a: Position, b: Position) -> i32 {
    let dx = f64::from(a.x - b.x);
    let dy = f64::from(a.y - b.y);
    ((dx * dx + dy * dy).sqrt()).floor() as i32
}

fn remember_recent_position(recent_positions: &mut Vec<Position>, pos: Position) {
    recent_positions.push(pos);
    if recent_positions.len() > 5 {
        let extra = recent_positions.len() - 5;
        recent_positions.drain(0..extra);
    }
}

fn tile_name(tile: Tile) -> &'static str {
    match tile {
        Tile::Empty => "empty",
        Tile::Dirt => "dirt",
        Tile::Stone => "stone",
        Tile::Resource => "resource",
        Tile::Food => "food",
        Tile::Bedrock => "bedrock",
    }
}
