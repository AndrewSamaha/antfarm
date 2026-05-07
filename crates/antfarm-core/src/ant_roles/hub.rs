use crate::{
    NpcDebugEvent, SURFACE_Y,
    game_state::GameState,
    pheromones::PheromoneChannel,
    types::{HubPhase, HubState, Position, Tile},
};

const HUB_DIAGONAL_X_OFFSET: i32 = 50;
const HUB_DIAGONAL_Y_OFFSET: i32 = 50;
const HUB_RIGHT_SPUR_OFFSET: i32 = 10;
const HUB_PHEROMONE_RADIUS: i32 = 60;
const HUB_PHEROMONE_PEAK: u8 = 240;

pub(crate) fn tick(
    game: &mut GameState,
    index: usize,
    _queen_pos: Option<Position>,
    events: &mut Vec<String>,
) {
    let Some(state) = game.npcs[index].hub_state() else {
        game.set_worker_idle(index);
        return;
    };

    if matches!(state.phase, HubPhase::DigToSurface | HubPhase::ReturnToHub | HubPhase::HoldAtHub)
        && let Some(hive_id) = game.npcs[index].hive_id
    {
        emit_hub_pheromone(game, hive_id, state.hub_center);
    }

    game.set_worker_idle(index);
    match state.phase {
        HubPhase::DigToHub => {
            if tick_toward(game, index, diagonal_target(state), true, true, events) {
                advance_phase(game, index, HubPhase::DigRightSpur);
            }
        }
        HubPhase::DigRightSpur => {
            if tick_toward(game, index, state.hub_center, false, true, events) {
                advance_phase(game, index, HubPhase::DigToSurface);
            }
        }
        HubPhase::DigToSurface => {
            if tick_toward(game, index, state.surface_entry, true, false, events) {
                advance_phase(game, index, HubPhase::ReturnToHub);
            }
        }
        HubPhase::ReturnToHub => {
            if tick_toward(game, index, state.hub_center, true, false, events) {
                advance_phase(game, index, HubPhase::HoldAtHub);
            }
        }
        HubPhase::HoldAtHub => {
            if game.npcs[index].pos != state.hub_center
                && !game.npc_blocks_movement(state.hub_center, index)
            {
                game.npcs[index].pos = state.hub_center;
                game.remember_recent_position(index, state.hub_center);
                game.mark_npcs_dirty();
            }
        }
    }
}

pub(crate) fn on_hatch(game: &mut GameState, index: usize) {
    let origin = game.npcs[index].pos;
    let hub_center = Position {
        x: origin.x + HUB_DIAGONAL_X_OFFSET + HUB_RIGHT_SPUR_OFFSET,
        y: origin.y + HUB_DIAGONAL_Y_OFFSET,
    };
    let surface_entry = Position {
        x: hub_center.x,
        y: SURFACE_Y + 1,
    };
    let worker = &mut game.npcs[index];
    worker.set_hub_state(HubState {
        origin,
        hub_center,
        surface_entry,
        phase: HubPhase::DigToHub,
    });
}

fn advance_phase(game: &mut GameState, index: usize, phase: HubPhase) {
    let Some(mut state) = game.npcs[index].hub_state() else {
        return;
    };
    state.phase = phase;
    game.npcs[index].set_hub_state(state);
}

fn tick_toward(
    game: &mut GameState,
    index: usize,
    target: Position,
    allow_vertical: bool,
    allow_horizontal: bool,
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
    move_or_dig(game, index, next, events);
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

fn move_or_dig(game: &mut GameState, index: usize, next: Position, events: &mut Vec<String>) {
    if !game.world.in_bounds(next) || game.npc_blocks_movement(next, index) {
        return;
    }
    let Some(tile) = game.world.tile(next) else {
        return;
    };
    if matches!(tile, Tile::Bedrock) {
        return;
    }

    let npc_pos = game.npcs[index].pos;
    if !matches!(tile, Tile::Empty) {
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

fn diagonal_target(state: HubState) -> Position {
    Position {
        x: state.origin.x + HUB_DIAGONAL_X_OFFSET,
        y: state.origin.y + HUB_DIAGONAL_Y_OFFSET,
    }
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
