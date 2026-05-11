use serde_json::Value;
use std::collections::{HashSet, VecDeque};

use crate::{
    NpcDebugEvent, SURFACE_Y,
    game_state::GameState,
    inventory::{add_inventory, inventory_count, remove_inventory},
    pheromones::PheromoneChannel,
    role_helpers::orbit::{choose_clockwise_ring_step, perimeter},
    types::{HubPatrolLeg, HubPheromoneTrail, HubState, Position, Tile},
};

const INITIAL_HUB_ORBIT_RADIUS: i32 = 1;
const HUB_PHEROMONE_RADIUS: i32 = 60;
const HUB_PHEROMONE_PEAK: u8 = 240;
const TUNNEL_PHEROMONE_THRESHOLD: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HubPlanStep {
    MoveRelative { x: i32, y: i32, dig: bool },
    BeginPheromoneTrail {
        channel: PheromoneChannel,
        initial_value: u8,
        change_on_step: i16,
    },
    EndPheromoneTrail { channel: PheromoneChannel },
    SetHubLocation,
    MoveToSurface { dig: bool },
    MoveToHub { dig: bool },
    HoldAtHub,
}

pub(crate) fn tick(
    game: &mut GameState,
    index: usize,
    _queen_pos: Option<Position>,
    events: &mut Vec<String>,
) {
    let mut state = match game.npcs[index].hub_state() {
        Some(state) => state,
        None => {
            game.set_worker_idle(index);
            return;
        }
    };
    let plan = configured_plan(&game.config);

    if state.has_hub_location
        && let Some(hive_id) = game.npcs[index].hive_id
    {
        emit_hub_pheromone(game, hive_id, state.hub_center);
    }

    game.set_worker_idle(index);

    if usize::from(state.step_index) >= plan.len() {
        game.npcs[index].set_hub_state(state);
        return;
    }

    let completed = match plan[usize::from(state.step_index)] {
        HubPlanStep::MoveRelative { x, y, dig } => {
            let target = state.current_target.unwrap_or_else(|| {
                let pos = game.npcs[index].pos;
                pos.offset(x, y)
            });
            state.current_target = Some(target);
            tick_toward(game, index, &mut state, target, true, true, dig, events)
        }
        HubPlanStep::BeginPheromoneTrail {
            channel,
            initial_value,
            change_on_step,
        } => {
            state.active_pheromone_trail = Some(HubPheromoneTrail {
                channel,
                next_value: initial_value,
                change_on_step,
            });
            true
        }
        HubPlanStep::EndPheromoneTrail { channel } => {
            if state
                .active_pheromone_trail
                .is_some_and(|trail| trail.channel == channel)
            {
                state.active_pheromone_trail = None;
            }
            true
        }
        HubPlanStep::SetHubLocation => {
            let pos = game.npcs[index].pos;
            state.hub_center = pos;
            state.has_hub_location = true;
            state.current_target = None;
            true
        }
        HubPlanStep::MoveToSurface { dig } => {
            let target = state.current_target.unwrap_or_else(|| Position {
                x: game.npcs[index].pos.x,
                y: SURFACE_Y + 1,
            });
            state.current_target = Some(target);
            tick_toward(game, index, &mut state, target, true, false, dig, events)
        }
        HubPlanStep::MoveToHub { dig } => {
            if !state.has_hub_location {
                false
            } else {
                let hub_center = state.hub_center;
                state.current_target = Some(hub_center);
                tick_toward(game, index, &mut state, hub_center, true, true, dig, events)
            }
        }
        HubPlanStep::HoldAtHub => {
            tick_hold_at_hub(game, index, &mut state, events);
            false
        }
    };

    if completed {
        state.step_index = state.step_index.saturating_add(1);
        state.current_target = None;
    }
    game.npcs[index].set_hub_state(state);
}

pub(crate) fn on_hatch(game: &mut GameState, index: usize) {
    let origin = game.npcs[index].pos;
    let plan = configured_plan(&game.config);
    let hub_center = derive_hub_center(origin, &plan).unwrap_or(origin);
    let worker = &mut game.npcs[index];
    worker.set_hub_state(HubState {
        origin,
        hub_center,
        has_hub_location: false,
        step_index: 0,
        current_target: None,
        ..HubState::default()
    });
}

fn configured_plan(config: &Value) -> Vec<HubPlanStep> {
    let Some(items) = config
        .pointer("/colony/roles/hive_maintenance/hub/plan")
        .and_then(Value::as_array)
    else {
        return default_plan();
    };

    let mut steps = Vec::new();
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        if let Some(move_relative) = object.get("move_relative").and_then(Value::as_object) {
            let x = move_relative
                .get("x")
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(0);
            let y = move_relative
                .get("y")
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(0);
            let dig = move_relative
                .get("dig")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            steps.push(HubPlanStep::MoveRelative { x, y, dig });
            continue;
        }
        if let Some(begin) = object.get("begin_pheromone_trail").and_then(Value::as_object) {
            let Some(channel) = begin
                .get("pheromone")
                .and_then(Value::as_str)
                .and_then(parse_plan_pheromone_channel)
            else {
                continue;
            };
            let initial_value = begin
                .get("initial_value")
                .and_then(Value::as_i64)
                .map(clamp_to_u8)
                .unwrap_or(0);
            let change_on_step = begin
                .get("change_on_step")
                .and_then(Value::as_i64)
                .map(clamp_to_i16)
                .unwrap_or(0);
            steps.push(HubPlanStep::BeginPheromoneTrail {
                channel,
                initial_value,
                change_on_step,
            });
            continue;
        }
        if let Some(channel) = object
            .get("end_pheromone_trail")
            .and_then(Value::as_str)
            .and_then(parse_plan_pheromone_channel)
        {
            steps.push(HubPlanStep::EndPheromoneTrail { channel });
            continue;
        }
        if object
            .get("set_hub_location")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            steps.push(HubPlanStep::SetHubLocation);
            continue;
        }
        if let Some(move_to_surface) = object.get("move_to_surface") {
            let dig = move_to_surface
                .as_object()
                .and_then(|value| value.get("dig"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            steps.push(HubPlanStep::MoveToSurface { dig });
            continue;
        }
        if let Some(move_to_hub) = object.get("move_to_hub") {
            let dig = move_to_hub
                .as_object()
                .and_then(|value| value.get("dig"))
                .and_then(Value::as_bool)
                .unwrap_or_else(|| move_to_hub.as_bool().unwrap_or(false));
            if move_to_hub.is_boolean() && !dig {
                continue;
            }
            steps.push(HubPlanStep::MoveToHub { dig });
            continue;
        }
        if object
            .get("hold_at_hub")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            steps.push(HubPlanStep::HoldAtHub);
        }
    }

    if steps.is_empty() {
        default_plan()
    } else {
        steps
    }
}

fn orbit_max_radius(config: &Value) -> i32 {
    config
        .pointer("/colony/roles/hive_maintenance/hub/orbit_max_radius")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(9)
        .max(1)
}

fn default_plan() -> Vec<HubPlanStep> {
    vec![
        HubPlanStep::BeginPheromoneTrail {
            channel: PheromoneChannel::QueenChamberTunnel,
            initial_value: 100,
            change_on_step: -1,
        },
        HubPlanStep::MoveRelative {
            x: 50,
            y: 50,
            dig: true,
        },
        HubPlanStep::MoveRelative {
            x: 10,
            y: 0,
            dig: true,
        },
        HubPlanStep::SetHubLocation,
        HubPlanStep::EndPheromoneTrail {
            channel: PheromoneChannel::QueenChamberTunnel,
        },
        HubPlanStep::MoveToSurface { dig: true },
        HubPlanStep::BeginPheromoneTrail {
            channel: PheromoneChannel::EntryTunnel,
            initial_value: 100,
            change_on_step: -1,
        },
        HubPlanStep::MoveToHub { dig: true },
        HubPlanStep::EndPheromoneTrail {
            channel: PheromoneChannel::EntryTunnel,
        },
        HubPlanStep::HoldAtHub,
    ]
}

fn derive_hub_center(origin: Position, plan: &[HubPlanStep]) -> Option<Position> {
    let mut cursor = origin;
    for step in plan {
        match step {
            HubPlanStep::MoveRelative { x, y, .. } => {
                cursor = cursor.offset(*x, *y);
            }
            HubPlanStep::SetHubLocation => return Some(cursor),
            HubPlanStep::BeginPheromoneTrail { .. } | HubPlanStep::EndPheromoneTrail { .. } => {}
            HubPlanStep::MoveToSurface { .. }
            | HubPlanStep::MoveToHub { .. }
            | HubPlanStep::HoldAtHub => return Some(cursor),
        }
    }
    None
}

fn tick_hold_at_hub(game: &mut GameState, index: usize, state: &mut HubState, events: &mut Vec<String>) {
    if !state.has_hub_location {
        return;
    }

    let pos = game.npcs[index].pos;
    if should_orbit_at_hub(state, pos) {
        start_orbit_cycle(state);
    }
    if state.orbit_radius.is_some() || state.orbit_returning_to_center {
        tick_hub_orbit(game, index, state, events);
        let current_pos = game.npcs[index].pos;
        if state.orbit_radius.is_some() || state.orbit_returning_to_center || current_pos != state.hub_center
        {
            return;
        }
        advance_patrol_leg(state);
    }

    let pos = game.npcs[index].pos;
    let mut target = patrol_target(state);
    if pos == target {
        advance_patrol_leg(state);
        target = patrol_target(state);
    }
    let Some(next) = choose_tunnel_patrol_step(game, index, state, pos, target) else {
        return;
    };
    move_or_dig(game, index, state, next, true, events);
    if game.npcs[index].pos == next {
        let _ = try_fill_tunnel_gap(game, index, state, events);
    }
}

fn should_orbit_at_hub(state: &HubState, pos: Position) -> bool {
    pos == state.hub_center
        && state.orbit_radius.is_none()
        && !state.orbit_returning_to_center
        && matches!(
            state.patrol_leg,
            HubPatrolLeg::ToHubFromQueenChamber | HubPatrolLeg::ToHubFromSurface
        )
}

fn start_orbit_cycle(state: &mut HubState) {
    state.orbit_radius = Some(INITIAL_HUB_ORBIT_RADIUS);
    state.orbit_anchor = None;
    state.orbit_has_left_anchor = false;
    state.orbit_returning_to_center = false;
}

fn tick_hub_orbit(game: &mut GameState, index: usize, state: &mut HubState, events: &mut Vec<String>) {
    if state.orbit_returning_to_center {
        let hub_center = state.hub_center;
        let _ = tick_toward(game, index, state, hub_center, true, true, true, events);
        if game.npcs[index].pos == hub_center {
            state.orbit_radius = None;
            state.orbit_anchor = None;
            state.orbit_has_left_anchor = false;
            state.orbit_returning_to_center = false;
        }
        return;
    }

    ensure_orbit_radius_initialized(state);
    let radius = state.orbit_radius.unwrap_or(INITIAL_HUB_ORBIT_RADIUS);
    let ring = perimeter(
        state.hub_center,
        radius,
        radius,
        game.world.width(),
        game.world.height(),
    );
    if ring.is_empty() {
        state.orbit_returning_to_center = true;
        return;
    }

    let pos = game.npcs[index].pos;
    if ring.contains(&pos) {
        update_hub_orbit_growth(state, pos, &ring, orbit_max_radius(&game.config));
        if state.orbit_returning_to_center {
            return;
        }
    }

    let radius = state.orbit_radius.unwrap_or(INITIAL_HUB_ORBIT_RADIUS);
    let ring = perimeter(
        state.hub_center,
        radius,
        radius,
        game.world.width(),
        game.world.height(),
    );
    if let Some(next) = choose_clockwise_ring_step(game, index, state.hub_center, &ring) {
        move_or_dig(game, index, state, next, true, events);
    }
}

fn ensure_orbit_radius_initialized(state: &mut HubState) {
    if state.orbit_radius.is_none() {
        state.orbit_radius = Some(INITIAL_HUB_ORBIT_RADIUS);
    }
}

fn update_hub_orbit_growth(state: &mut HubState, pos: Position, ring: &[Position], max_radius: i32) {
    if !ring.contains(&pos) {
        return;
    }
    match state.orbit_anchor {
        None => {
            state.orbit_anchor = Some(pos);
            state.orbit_has_left_anchor = false;
        }
        Some(anchor) if !state.orbit_has_left_anchor && pos != anchor => {
            state.orbit_has_left_anchor = true;
        }
        Some(anchor) if state.orbit_has_left_anchor && pos == anchor => {
            let current_radius = state.orbit_radius.unwrap_or(INITIAL_HUB_ORBIT_RADIUS);
            if current_radius < max_radius {
                state.orbit_radius = Some(current_radius + 1);
                state.orbit_anchor = None;
                state.orbit_has_left_anchor = false;
            } else {
                state.orbit_returning_to_center = true;
                state.orbit_anchor = None;
                state.orbit_has_left_anchor = false;
            }
        }
        _ => {}
    }
}

fn tick_toward(
    game: &mut GameState,
    index: usize,
    state: &mut HubState,
    target: Position,
    allow_vertical: bool,
    allow_horizontal: bool,
    dig: bool,
    events: &mut Vec<String>,
) -> bool {
    let pos = game.npcs[index].pos;
    if pos == target {
        return true;
    }

    let next = next_step(pos, target, allow_vertical, allow_horizontal);
    let Some(next) = next else {
        return true;
    };
    move_or_dig(game, index, state, next, dig, events);
    game.npcs[index].pos == target
}

fn next_step(
    pos: Position,
    target: Position,
    allow_vertical: bool,
    allow_horizontal: bool,
) -> Option<Position> {
    if allow_vertical && pos.y != target.y {
        let dy = if target.y > pos.y { 1 } else { -1 };
        return Some(pos.offset(0, dy));
    }
    if allow_horizontal && pos.x != target.x {
        let dx = if target.x > pos.x { 1 } else { -1 };
        return Some(pos.offset(dx, 0));
    }
    None
}

fn move_or_dig(
    game: &mut GameState,
    index: usize,
    state: &mut HubState,
    next: Position,
    dig: bool,
    events: &mut Vec<String>,
) {
    if !game.world.in_bounds(next) || game.npc_blocks_movement(next, index) {
        return;
    }
    let Some(tile) = game.world.tile(next) else {
        return;
    };
    if matches!(tile, Tile::Bedrock) {
        return;
    }
    if !dig && !matches!(tile, Tile::Empty) {
        return;
    }

    let npc_pos = game.npcs[index].pos;
    if dig && !matches!(tile, Tile::Empty) {
        match tile {
            Tile::Dirt => add_inventory(&mut game.npcs[index].inventory, "dirt", 1),
            Tile::Resource => add_inventory(&mut game.npcs[index].inventory, "ore", 1),
            Tile::Empty | Tile::Stone | Tile::Food | Tile::Bedrock => {}
        }
        game.set_world_tile(next, Tile::Empty);
        events.push(format!(
            "NPC ant {} dug hub tunnel at {},{}",
            game.npcs[index].id, next.x, next.y
        ));
        game.push_npc_debug_event(NpcDebugEvent {
            tick: game.tick,
            npc_id: game.npcs[index].id,
            hive_id: game.npcs[index].hive_id,
            event_type: "hub_dig".to_string(),
            pos: npc_pos,
            details: serde_json::json!({
                "target": { "x": next.x, "y": next.y },
                "tile": tile_name(tile),
            }),
        });
    }
    game.npcs[index].pos = next;
    emit_active_trail(game, index, state, next);
    game.remember_recent_position(index, next);
    game.mark_npcs_dirty();
}

fn emit_active_trail(game: &mut GameState, index: usize, state: &mut HubState, pos: Position) {
    let Some(hive_id) = game.npcs[index].hive_id else {
        return;
    };
    let Some(mut trail) = state.active_pheromone_trail else {
        return;
    };
    if trail.next_value > 0 {
        game.pheromones
            .deposit(pos, hive_id, trail.channel, trail.next_value);
    }
    trail.next_value = apply_trail_step_change(trail.next_value, trail.change_on_step);
    state.active_pheromone_trail = Some(trail);
}

fn patrol_target(state: &HubState) -> Position {
    match state.patrol_leg {
        HubPatrolLeg::ToQueenChamber => state.origin,
        HubPatrolLeg::ToHubFromQueenChamber => state.hub_center,
        HubPatrolLeg::ToSurface => Position {
            x: state.hub_center.x,
            y: SURFACE_Y + 1,
        },
        HubPatrolLeg::ToHubFromSurface => state.hub_center,
    }
}

fn choose_tunnel_patrol_step(
    game: &GameState,
    index: usize,
    state: &HubState,
    origin: Position,
    target: Position,
) -> Option<Position> {
    if origin == target {
        return None;
    }

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(origin);
    queue.push_back((origin, None::<Position>));

    while let Some((pos, first_step)) = queue.pop_front() {
        for next in ordered_neighbors_toward(pos, target) {
            if visited.contains(&next) || !hub_tunnel_traversable(game, index, state, next, target) {
                continue;
            }
            let first_step = first_step.or(Some(next));
            if next == target {
                return first_step;
            }
            visited.insert(next);
            queue.push_back((next, first_step));
        }
    }

    None
}

fn hub_tunnel_traversable(
    game: &GameState,
    index: usize,
    state: &HubState,
    pos: Position,
    target: Position,
) -> bool {
    if !game.world.in_bounds(pos)
        || game.players.values().any(|player| player.pos == pos)
        || game.npc_blocks_movement(pos, index)
    {
        return false;
    }
    let Some(tile) = game.world.tile(pos) else {
        return false;
    };
    if matches!(tile, Tile::Bedrock | Tile::Stone) {
        return false;
    }
    if pos == target {
        return true;
    }
    patrol_leg_channel(state.patrol_leg)
        .is_some_and(|channel| game.pheromones.value(pos, game.npcs[index].hive_id.unwrap_or(0), channel) >= TUNNEL_PHEROMONE_THRESHOLD)
}

fn patrol_leg_channel(leg: HubPatrolLeg) -> Option<PheromoneChannel> {
    match leg {
        HubPatrolLeg::ToQueenChamber | HubPatrolLeg::ToHubFromQueenChamber => {
            Some(PheromoneChannel::QueenChamberTunnel)
        }
        HubPatrolLeg::ToSurface | HubPatrolLeg::ToHubFromSurface => {
            Some(PheromoneChannel::EntryTunnel)
        }
    }
}

fn ordered_neighbors_toward(pos: Position, target: Position) -> [Position; 4] {
    let dx = target.x - pos.x;
    let dy = target.y - pos.y;
    let horizontal_first = dx.abs() >= dy.abs();
    let x_step = pos.offset(dx.signum(), 0);
    let y_step = pos.offset(0, dy.signum());
    let x_back = pos.offset(-dx.signum(), 0);
    let y_back = pos.offset(0, -dy.signum());

    if horizontal_first {
        [x_step, y_step, y_back, x_back]
    } else {
        [y_step, x_step, x_back, y_back]
    }
}

fn advance_patrol_leg(state: &mut HubState) {
    state.patrol_leg = match state.patrol_leg {
        HubPatrolLeg::ToQueenChamber => HubPatrolLeg::ToHubFromQueenChamber,
        HubPatrolLeg::ToHubFromQueenChamber => HubPatrolLeg::ToSurface,
        HubPatrolLeg::ToSurface => HubPatrolLeg::ToHubFromSurface,
        HubPatrolLeg::ToHubFromSurface => HubPatrolLeg::ToQueenChamber,
    };
}

fn try_fill_tunnel_gap(
    game: &mut GameState,
    index: usize,
    state: &HubState,
    events: &mut Vec<String>,
) -> bool {
    if inventory_count(&game.npcs[index].inventory, "dirt") == 0 {
        return false;
    }
    let Some(hive_id) = game.npcs[index].hive_id else {
        return false;
    };

    let pos = game.npcs[index].pos;
    let candidates = [
        pos.offset(-1, 0),
        pos.offset(1, 0),
        pos.offset(0, -1),
        pos.offset(0, 1),
    ];
    let mut best_target = None;
    let mut best_score = i32::MIN;
    for target in candidates {
        let Some(score) = tunnel_fill_score(game, index, state, hive_id, target) else {
            continue;
        };
        if score > best_score {
            best_score = score;
            best_target = Some(target);
        }
    }

    let Some(target) = best_target else {
        return false;
    };
    if !remove_inventory(&mut game.npcs[index].inventory, "dirt", 1) {
        return false;
    }
    game.set_world_tile(target, Tile::Dirt);
    let npc_id = game.npcs[index].id;
    events.push(format!("NPC ant {} repaired tunnel wall at {},{}", npc_id, target.x, target.y));
    game.push_npc_debug_event(NpcDebugEvent {
        tick: game.tick,
        npc_id,
        hive_id: Some(hive_id),
        event_type: "hub_fill".to_string(),
        pos,
        details: serde_json::json!({
            "target": { "x": target.x, "y": target.y },
            "remaining_dirt": inventory_count(&game.npcs[index].inventory, "dirt"),
        }),
    });
    true
}

fn tunnel_fill_score(
    game: &GameState,
    index: usize,
    state: &HubState,
    hive_id: u16,
    target: Position,
) -> Option<i32> {
    if !game.world.in_bounds(target) || game.world.tile(target) != Some(Tile::Empty) {
        return None;
    }
    if game
        .npcs
        .iter()
        .enumerate()
        .any(|(other_index, npc)| other_index != index && npc.pos == target)
        || game.players.values().any(|player| player.pos == target)
    {
        return None;
    }
    if is_tunnel_tile(game, hive_id, target)
        || is_inside_hub_zone(state, target, orbit_max_radius(&game.config))
        || is_inside_queen_chamber_zone(game, hive_id, target)
    {
        return None;
    }

    let cardinal_neighbors = [
        target.offset(-1, 0),
        target.offset(1, 0),
        target.offset(0, -1),
        target.offset(0, 1),
    ];
    let mut tunnel_neighbor_count = 0;
    let mut tunnel_neighbor_strength = 0;
    let mut solid_neighbor_count = 0;
    let mut open_nontunnel_neighbor_count = 0;
    for neighbor in cardinal_neighbors {
        if !game.world.in_bounds(neighbor) {
            continue;
        }
        if is_tunnel_tile(game, hive_id, neighbor) {
            tunnel_neighbor_count += 1;
            tunnel_neighbor_strength += i32::from(tunnel_strength(game, hive_id, neighbor));
            continue;
        }
        match game.world.tile(neighbor) {
            Some(Tile::Dirt | Tile::Stone | Tile::Bedrock | Tile::Resource) => {
                solid_neighbor_count += 1;
            }
            Some(Tile::Empty) => {
                open_nontunnel_neighbor_count += 1;
            }
            Some(Tile::Food) | None => {}
        }
    }

    if tunnel_neighbor_count < 1 || solid_neighbor_count < 2 || open_nontunnel_neighbor_count > 1 {
        return None;
    }

    Some(tunnel_neighbor_strength + solid_neighbor_count * 10 - open_nontunnel_neighbor_count * 5)
}

fn is_tunnel_tile(game: &GameState, hive_id: u16, pos: Position) -> bool {
    tunnel_strength(game, hive_id, pos) >= TUNNEL_PHEROMONE_THRESHOLD
}

fn tunnel_strength(game: &GameState, hive_id: u16, pos: Position) -> u8 {
    let queen_tunnel = game
        .pheromones
        .value(pos, hive_id, PheromoneChannel::QueenChamberTunnel);
    let entry_tunnel = game
        .pheromones
        .value(pos, hive_id, PheromoneChannel::EntryTunnel);
    queen_tunnel.max(entry_tunnel)
}

fn is_inside_hub_zone(state: &HubState, pos: Position, radius: i32) -> bool {
    (pos.x - state.hub_center.x)
        .abs()
        .max((pos.y - state.hub_center.y).abs())
        <= radius
}

fn is_inside_queen_chamber_zone(game: &GameState, hive_id: u16, pos: Position) -> bool {
    let Some(queen_pos) = game.find_queen_pos(hive_id) else {
        return false;
    };
    let (radius_x, radius_y) = game.queen_chamber_max_radii();
    let dx = f64::from(pos.x - queen_pos.x);
    let dy = f64::from(pos.y - queen_pos.y);
    ((dx * dx) / f64::from(radius_x * radius_x))
        + ((dy * dy) / f64::from(radius_y * radius_y))
        <= 1.0
}

fn apply_trail_step_change(value: u8, delta: i16) -> u8 {
    let next = i32::from(value) + i32::from(delta);
    next.clamp(0, i32::from(u8::MAX)) as u8
}

fn parse_plan_pheromone_channel(value: &str) -> Option<PheromoneChannel> {
    match value {
        "home" => Some(PheromoneChannel::Home),
        "food" => Some(PheromoneChannel::Food),
        "hub" => Some(PheromoneChannel::Hub),
        "queen_chamber_tunnel" | "queen_chamber_tunnel_pheromone" => {
            Some(PheromoneChannel::QueenChamberTunnel)
        }
        "entry_tunnel" | "entry_tunnel_pheromone" => Some(PheromoneChannel::EntryTunnel),
        "threat" => Some(PheromoneChannel::Threat),
        "defense" => Some(PheromoneChannel::Defense),
        _ => None,
    }
}

fn clamp_to_u8(value: i64) -> u8 {
    value.clamp(0, i64::from(u8::MAX)) as u8
}

fn clamp_to_i16(value: i64) -> i16 {
    value.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

fn emit_hub_pheromone(game: &mut GameState, hive_id: u16, center: Position) {
    for dy in -HUB_PHEROMONE_RADIUS..=HUB_PHEROMONE_RADIUS {
        for dx in -HUB_PHEROMONE_RADIUS..=HUB_PHEROMONE_RADIUS {
            let distance = dx.abs() + dy.abs();
            if distance > HUB_PHEROMONE_RADIUS {
                continue;
            }
            let pos = center.offset(dx, dy);
            if pos.y <= SURFACE_Y || !game.pheromones.in_bounds(pos) {
                continue;
            }
            let falloff = distance as u8;
            let amount = HUB_PHEROMONE_PEAK.saturating_sub(
                falloff.saturating_mul(
                    HUB_PHEROMONE_PEAK.max(1) / (HUB_PHEROMONE_RADIUS.max(1) as u8 + 1),
                ),
            );
            if amount > 0 {
                game.pheromones
                    .deposit(pos, hive_id, PheromoneChannel::Hub, amount);
            }
        }
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
