use serde_json::Value;

use crate::{
    NpcDebugEvent, SURFACE_Y,
    game_state::GameState,
    pheromones::PheromoneChannel,
    types::{HubState, Position, Tile},
};

const HUB_PHEROMONE_RADIUS: i32 = 60;
const HUB_PHEROMONE_PEAK: u8 = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HubPlanStep {
    MoveRelative { x: i32, y: i32, dig: bool },
    SetHubLocation,
    MoveToSurface { dig: bool },
    MoveToHub,
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
            tick_toward(game, index, target, true, true, dig, events)
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
            tick_toward(game, index, target, true, false, dig, events)
        }
        HubPlanStep::MoveToHub => {
            if !state.has_hub_location {
                false
            } else {
                state.current_target = Some(state.hub_center);
                tick_toward(game, index, state.hub_center, true, true, false, events)
            }
        }
        HubPlanStep::HoldAtHub => {
            if state.has_hub_location
                && game.npcs[index].pos != state.hub_center
            {
                let _ = tick_toward(game, index, state.hub_center, true, true, false, events);
            }
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
    let worker = &mut game.npcs[index];
    worker.set_hub_state(HubState {
        origin,
        hub_center: origin,
        has_hub_location: false,
        step_index: 0,
        current_target: None,
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
        if object
            .get("move_to_hub")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            steps.push(HubPlanStep::MoveToHub);
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

fn default_plan() -> Vec<HubPlanStep> {
    vec![
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
        HubPlanStep::MoveToSurface { dig: true },
        HubPlanStep::MoveToHub,
        HubPlanStep::HoldAtHub,
    ]
}

fn tick_toward(
    game: &mut GameState,
    index: usize,
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
    move_or_dig(game, index, next, dig, events);
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
    game.remember_recent_position(index, next);
    game.mark_npcs_dirty();
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
