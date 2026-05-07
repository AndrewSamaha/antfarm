use serde_json::{Value, json};

use crate::{
    NpcDebugEvent,
    game_state::GameState,
    pheromones::{AntBehaviorState, PheromoneChannel},
    types::{MoveDir, Position, Tile},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchBehaviorProfile {
    Baseline,
    OutwardBiasV1,
    LocalFieldV1,
    LocalFieldV2,
    OutwardBiasWithLocalFieldV1,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LocalFieldSearchScore {
    pub(crate) visible_food_bonus: u32,
    pub(crate) visible_food_distance: Option<u32>,
    pub(crate) food_field_score: u32,
    pub(crate) home_field_penalty: u32,
    pub(crate) stone_penalty: u32,
}

pub(crate) fn tick(
    game: &mut GameState,
    index: usize,
    queen_pos: Option<Position>,
    events: &mut Vec<String>,
) {
    game.tick_worker_memory(index);
    let npc_hive = game.npcs[index].hive_id;
    let npc_pos = game.npcs[index].pos;
    let npc_id = game.npcs[index].id;
    let behavior = game.npcs[index].behavior;
    let home_axes =
        npc_hive.map(|hive_id| game.sample_gradient_axes(npc_pos, hive_id, PheromoneChannel::Home));
    let food_axes =
        npc_hive.map(|hive_id| game.sample_gradient_axes(npc_pos, hive_id, PheromoneChannel::Food));
    let current_food_pheromone = u32::from(
        npc_hive
            .map(|hive_id| game.pheromones.value(npc_pos, hive_id, PheromoneChannel::Food))
            .unwrap_or(0),
    );
    let search_profile = game.search_behavior_profile();
    let effective_search_profile = game.effective_search_behavior_profile(index, search_profile);
    let food_carry_max = game.food_carry_max();
    let carry_visible_food_targets = if behavior == AntBehaviorState::Searching
        && game.npcs[index].carrying_food
        && game.npcs[index].food < food_carry_max
    {
        game.adjacent_food_positions(npc_pos)
    } else {
        Vec::new()
    };
    let search_destination = match (behavior, effective_search_profile, npc_hive) {
        (AntBehaviorState::Searching, SearchBehaviorProfile::LocalFieldV2, Some(hive_id)) => {
            game.refresh_local_field_destination(index, hive_id, queen_pos)
        }
        _ => {
            game.npcs[index].search_destination = None;
            game.npcs[index].search_destination_stuck_ticks = 0;
            None
        }
    };

    if let Some(hive_id) = npc_hive {
        match behavior {
            AntBehaviorState::Searching => {}
            AntBehaviorState::ReturningFood => {
                let deposit = food_deposit_for_carry_ticks(game.npcs[index].carrying_food_ticks);
                game.pheromones
                    .deposit(npc_pos, hive_id, PheromoneChannel::Food, deposit);
                if game.try_deliver_food_to_queen(index) {
                    game.mark_npcs_dirty();
                    return;
                }
            }
            AntBehaviorState::Defending | AntBehaviorState::Idle => {}
        }
    }

    if behavior == AntBehaviorState::Searching && game.try_place_dirt(index, queen_pos, events) {
        game.mark_npcs_dirty();
        return;
    }

    let directions = [
        npc_pos.offset(-1, 0),
        npc_pos.offset(1, 0),
        npc_pos.offset(0, 1),
        npc_pos.offset(0, -1),
    ];
    let mut raw_candidates = Vec::new();
    for next in directions {
        if next.y <= crate::SURFACE_Y {
            continue;
        }
        if !game.world.in_bounds(next) || game.npc_blocks_movement(next, index) {
            continue;
        }
        let tile = game.world.tile(next);
        let food_pheromone = u32::from(
            npc_hive
                .map(|hive_id| game.pheromones.value(next, hive_id, PheromoneChannel::Food))
                .unwrap_or(0),
        );
        let home_pheromone = u32::from(
            npc_hive
                .map(|hive_id| game.pheromones.value(next, hive_id, PheromoneChannel::Home))
                .unwrap_or(0),
        );
        let random_score = u32::from(game.random_u8_inclusive(0, 24));
        let tile_bonus = match tile {
            Some(Tile::Food) if !matches!(behavior, AntBehaviorState::ReturningFood) => 80_u32,
            Some(Tile::Food) => 0_u32,
            Some(Tile::Empty) => 12_u32,
            Some(Tile::Dirt) | Some(Tile::Resource) => 2_u32,
            Some(Tile::Stone) | Some(Tile::Bedrock) | None => 0,
        };
        let queen_bias = match (behavior, queen_pos) {
            (AntBehaviorState::ReturningFood, Some(queen_pos)) => {
                let current = (queen_pos.x - npc_pos.x).abs() + (queen_pos.y - npc_pos.y).abs();
                let next_dist = (queen_pos.x - next.x).abs() + (queen_pos.y - next.y).abs();
                if next_dist < current {
                    120_u32
                } else if next_dist == current {
                    12_u32
                } else {
                    0_u32
                }
            }
            _ => 0_u32,
        };
        let memory_bias = u32::from(match behavior {
            AntBehaviorState::Searching => {
                direction_bias(game.npcs[index].recent_food_dir, npc_pos, next)
            }
            AntBehaviorState::ReturningFood => {
                direction_bias(game.npcs[index].recent_home_dir, npc_pos, next)
            }
            AntBehaviorState::Defending | AntBehaviorState::Idle => 0,
        });
        let search_profile_bias = match (behavior, effective_search_profile, queen_pos) {
            (
                AntBehaviorState::Searching,
                SearchBehaviorProfile::OutwardBiasV1,
                Some(queen_pos),
            ) => {
                let current = (queen_pos.x - npc_pos.x).abs() + (queen_pos.y - npc_pos.y).abs();
                let next_dist = (queen_pos.x - next.x).abs() + (queen_pos.y - next.y).abs();
                if next_dist > current {
                    80_u32
                } else if next_dist == current {
                    8_u32
                } else {
                    0_u32
                }
            }
            _ => 0_u32,
        };
        let local_field_search = match (behavior, effective_search_profile, npc_hive) {
            (AntBehaviorState::Searching, SearchBehaviorProfile::LocalFieldV1, Some(hive_id))
            | (AntBehaviorState::Searching, SearchBehaviorProfile::LocalFieldV2, Some(hive_id)) => {
                Some(game.local_field_search_bias(next, hive_id))
            }
            _ => None,
        };
        let destination_bias = match (behavior, effective_search_profile, search_destination) {
            (AntBehaviorState::Searching, SearchBehaviorProfile::LocalFieldV2, Some(destination)) => {
                local_field_destination_bias(npc_pos, next, destination)
            }
            _ => 0_u32,
        };
        let carry_harvest_bias = if !carry_visible_food_targets.is_empty() {
            let current_best = carry_visible_food_targets
                .iter()
                .map(|target| manhattan_distance(npc_pos, *target))
                .min()
                .unwrap_or(0);
            let next_best = carry_visible_food_targets
                .iter()
                .map(|target| manhattan_distance(next, *target))
                .min()
                .unwrap_or(current_best);
            if next_best < current_best {
                160_u32
            } else if next_best == current_best {
                16_u32
            } else {
                0_u32
            }
        } else {
            0_u32
        };
        let recent_position_penalty = recent_position_penalty(&game.npcs[index].recent_positions, next);
        raw_candidates.push((
            next,
            tile,
            food_pheromone,
            home_pheromone,
            random_score,
            tile_bonus,
            queen_bias,
            memory_bias,
            search_profile_bias,
            local_field_search,
            destination_bias,
            carry_harvest_bias,
            recent_position_penalty,
        ));
    }

    let has_increasing_adjacent_food_signal = matches!(behavior, AntBehaviorState::Searching)
        && raw_candidates
            .iter()
            .any(|(_, _, food_pheromone, _, _, _, _, _, _, _, _, _, _)| {
                *food_pheromone > current_food_pheromone
            });

    let mut candidates = Vec::new();
    let mut candidate_logs = Vec::new();
    for (
        next,
        tile,
        food_pheromone,
        home_pheromone,
        random_score,
        tile_bonus,
        queen_bias,
        memory_bias,
        search_profile_bias,
        local_field_search,
        destination_bias,
        carry_harvest_bias,
        recent_position_penalty,
    ) in raw_candidates
    {
        let pheromone_score = match behavior {
            AntBehaviorState::Searching
                if effective_search_profile == SearchBehaviorProfile::LocalFieldV1 =>
            {
                local_field_search
                    .map(|score| score.visible_food_bonus + score.food_field_score)
                    .unwrap_or(0)
            }
            AntBehaviorState::Searching
                if effective_search_profile == SearchBehaviorProfile::LocalFieldV2 =>
            {
                destination_bias
            }
            AntBehaviorState::Searching if has_increasing_adjacent_food_signal => {
                food_pheromone.saturating_sub(current_food_pheromone)
            }
            AntBehaviorState::Searching => 255_u32.saturating_sub(home_pheromone),
            AntBehaviorState::ReturningFood => home_pheromone,
            AntBehaviorState::Defending | AntBehaviorState::Idle => 0,
        };
        let terrain_penalty = match (behavior, tile) {
            (AntBehaviorState::ReturningFood, Some(Tile::Stone)) => 220_u32,
            (AntBehaviorState::ReturningFood, Some(Tile::Bedrock)) => 260_u32,
            _ => 0_u32,
        };
        let local_field_penalty = local_field_search
            .map(|score| score.home_field_penalty + score.stone_penalty)
            .unwrap_or(0);
        let score = pheromone_score
            + random_score
            + tile_bonus
            + queen_bias
            + memory_bias
            + search_profile_bias
            + carry_harvest_bias;
        let score = score
            .saturating_sub(terrain_penalty + recent_position_penalty + local_field_penalty);
        candidates.push((score, next, tile));
        candidate_logs.push(json!({
            "next": { "x": next.x, "y": next.y },
            "tile": tile.map(tile_name),
            "food_pheromone": food_pheromone,
            "home_pheromone": home_pheromone,
            "pheromone_score": pheromone_score,
            "current_food_pheromone": current_food_pheromone,
            "has_increasing_adjacent_food_signal": has_increasing_adjacent_food_signal,
            "random_score": random_score,
            "tile_bonus": tile_bonus,
            "queen_bias": queen_bias,
            "memory_bias": memory_bias,
            "search_profile": search_behavior_profile_name(effective_search_profile),
            "search_profile_bias": search_profile_bias,
            "search_destination": search_destination.map(|pos| json!({ "x": pos.x, "y": pos.y })),
            "destination_bias": destination_bias,
            "carry_harvest_bias": carry_harvest_bias,
            "local_field_search": local_field_search.map(|score| json!({
                "visible_food_bonus": score.visible_food_bonus,
                "visible_food_distance": score.visible_food_distance,
                "food_field_score": score.food_field_score,
                "home_field_penalty": score.home_field_penalty,
                "stone_penalty": score.stone_penalty,
            })),
            "recent_position_penalty": recent_position_penalty,
            "terrain_penalty": terrain_penalty,
            "local_field_penalty": local_field_penalty,
            "score": score,
        }));
    }

    candidates.sort_by_key(|(score, _, _)| std::cmp::Reverse(*score));

    let mut outcome = "blocked".to_string();
    let mut chosen_next = None;
    for (_, next, tile) in candidates {
        match tile {
            Some(Tile::Empty) => {
                game.npcs[index].pos = next;
                if matches!(behavior, AntBehaviorState::Searching)
                    && let (Some(hive_id), Some(home_trail_steps)) =
                        (npc_hive, game.npcs[index].home_trail_steps)
                {
                    let deposit = home_deposit_for_trail_steps(home_trail_steps);
                    if deposit > 0 {
                        game.pheromones.deposit(next, hive_id, PheromoneChannel::Home, deposit);
                        game.npcs[index].home_trail_steps =
                            Some(home_trail_steps.saturating_add(1));
                    } else {
                        game.npcs[index].home_trail_steps = None;
                    }
                }
                if matches!(behavior, AntBehaviorState::ReturningFood) {
                    game.npcs[index].carrying_food_ticks =
                        game.npcs[index].carrying_food_ticks.saturating_add(1);
                }
                game.remember_recent_position(index, next);
                game.update_search_destination_progress(index, npc_pos, Some(next));
                game.mark_npcs_dirty();
                outcome = "moved".to_string();
                chosen_next = Some(next);
                break;
            }
            Some(Tile::Food) if !matches!(behavior, AntBehaviorState::ReturningFood) => {
                game.set_world_tile(next, Tile::Empty);
                game.npcs[index].pos = next;
                let lifespan_bonus =
                    worker_lifespan_bonus(game.npcs[index].age_ticks, game.worker_lifespan_ticks_for(index));
                game.npcs[index].age_ticks = game.npcs[index].age_ticks.saturating_sub(lifespan_bonus);
                let previous_carried = game.npcs[index].food;
                let carried_after_pickup = previous_carried.saturating_add(1).min(food_carry_max);
                game.npcs[index].carrying_food = true;
                game.npcs[index].food = carried_after_pickup;
                let keep_collecting =
                    carried_after_pickup < food_carry_max && game.adjacent_food_visible(next);
                game.npcs[index].behavior = if keep_collecting {
                    AntBehaviorState::Searching
                } else {
                    AntBehaviorState::ReturningFood
                };
                game.npcs[index].carrying_food_ticks = 0;
                game.npcs[index].home_trail_steps = None;
                game.npcs[index].recent_home_memory_ticks = 0;
                game.npcs[index].recent_positions.clear();
                game.npcs[index].search_destination = None;
                game.npcs[index].search_destination_stuck_ticks = 0;
                game.found_food_count = game.found_food_count.saturating_add(1);
                game.mark_npcs_dirty();
                events.push(format!("NPC ant {} found food", npc_id));
                game.push_npc_debug_event(NpcDebugEvent {
                    tick: game.tick,
                    npc_id,
                    hive_id: npc_hive,
                    event_type: "found_food".to_string(),
                    pos: next,
                    details: json!({
                        "behavior_before": behavior_name(behavior),
                        "behavior_after": behavior_name(game.npcs[index].behavior),
                        "carried_food_before": previous_carried,
                        "carried_food_after": carried_after_pickup,
                        "food_carry_max": food_carry_max,
                        "keep_collecting": keep_collecting,
                        "lifespan_bonus": lifespan_bonus,
                        "age_ticks": game.npcs[index].age_ticks,
                    }),
                });
                outcome = "picked_up_food".to_string();
                chosen_next = Some(next);
                break;
            }
            Some(Tile::Dirt) | Some(Tile::Resource) => {
                match tile {
                    Some(Tile::Dirt) => {
                        crate::inventory::add_inventory(&mut game.npcs[index].inventory, "dirt", 1)
                    }
                    Some(Tile::Resource) => {
                        crate::inventory::add_inventory(&mut game.npcs[index].inventory, "ore", 1)
                    }
                    _ => {}
                }
                game.set_world_tile(next, Tile::Empty);
                events.push(format!("NPC ant {} tunneled at {},{}", npc_id, next.x, next.y));
                game.update_search_destination_progress(index, npc_pos, None);
                outcome = "tunneled".to_string();
                chosen_next = Some(next);
                break;
            }
            Some(Tile::Food) | Some(Tile::Stone) | Some(Tile::Bedrock) | None => {}
        }
    }
    if chosen_next.is_none() {
        game.update_search_destination_progress(index, npc_pos, None);
    }

    game.push_npc_debug_event(NpcDebugEvent {
        tick: game.tick,
        npc_id,
        hive_id: npc_hive,
        event_type: "selected_move".to_string(),
        pos: npc_pos,
        details: json!({
            "behavior": behavior_name(behavior),
            "search_behavior_profile": search_behavior_profile_name(search_profile),
            "effective_search_behavior_profile": search_behavior_profile_name(effective_search_profile),
            "carrying_food": game.npcs[index].carrying_food,
            "carried_food_count": game.npcs[index].food,
            "food_carry_max": food_carry_max,
            "carrying_food_ticks": game.npcs[index].carrying_food_ticks,
            "home_trail_steps": game.npcs[index].home_trail_steps,
            "search_destination": game.npcs[index]
                .search_destination
                .map(|pos| json!({ "x": pos.x, "y": pos.y })),
            "search_destination_stuck_ticks": game.npcs[index].search_destination_stuck_ticks,
            "has_delivered_food": game.npcs[index].has_delivered_food,
            "memory": {
                "home_dir": game.npcs[index].recent_home_dir.map(dir_name),
                "home_ttl": game.npcs[index].recent_home_memory_ticks,
                "food_dir": game.npcs[index].recent_food_dir.map(dir_name),
                "food_ttl": game.npcs[index].recent_food_memory_ticks,
                "recent_positions": game.npcs[index]
                    .recent_positions
                    .iter()
                    .map(|pos| json!({ "x": pos.x, "y": pos.y }))
                    .collect::<Vec<_>>(),
            },
            "radius_sample": {
                "home": home_axes.map(axes_json),
                "food": food_axes.map(axes_json),
            },
            "queen_pos": queen_pos.map(|pos| json!({ "x": pos.x, "y": pos.y })),
            "neighborhood": npc_hive.map(|hive_id| game.local_neighborhood_snapshot(npc_pos, hive_id)),
            "candidates": candidate_logs,
            "chosen_next": chosen_next.map(|pos| json!({ "x": pos.x, "y": pos.y })),
            "outcome": outcome,
        }),
    });
}

pub(crate) fn on_hatch(game: &mut GameState, index: usize) {
    game.npcs[index].behavior = crate::AntBehaviorState::Searching;
    game.npcs[index].home_trail_steps = Some(0);
}

pub(crate) fn search_behavior_profile_name(profile: SearchBehaviorProfile) -> &'static str {
    match profile {
        SearchBehaviorProfile::Baseline => "baseline",
        SearchBehaviorProfile::OutwardBiasV1 => "outward_bias_v1",
        SearchBehaviorProfile::LocalFieldV1 => "local_field_v1",
        SearchBehaviorProfile::LocalFieldV2 => "local_field_v2",
        SearchBehaviorProfile::OutwardBiasWithLocalFieldV1 => "outward_bias_with_local_field_v1",
    }
}

pub(crate) fn manhattan_distance(a: Position, b: Position) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

pub(crate) fn local_field_destination_bias(
    current: Position,
    next: Position,
    destination: Position,
) -> u32 {
    let current_distance = manhattan_distance(current, destination);
    let next_distance = manhattan_distance(next, destination);
    if next == destination {
        280
    } else if next_distance < current_distance {
        u32::try_from(current_distance - next_distance).unwrap_or(0) * 140
    } else if next_distance == current_distance {
        12
    } else {
        0
    }
}

pub(crate) fn recent_position_penalty(recent_positions: &[Position], next: Position) -> u32 {
    recent_positions
        .iter()
        .rev()
        .enumerate()
        .find_map(|(index, pos)| {
            if *pos != next {
                return None;
            }
            Some(match index {
                0 => 160,
                1 => 120,
                2 => 80,
                3 => 48,
                _ => 24,
            })
        })
        .unwrap_or(0)
}

pub(crate) fn direction_bias(preferred: Option<MoveDir>, current: Position, next: Position) -> u8 {
    let Some(preferred) = preferred else {
        return 0;
    };
    let dir = match (next.x - current.x, next.y - current.y) {
        (-1, 0) => MoveDir::Left,
        (1, 0) => MoveDir::Right,
        (0, -1) => MoveDir::Up,
        (0, 1) => MoveDir::Down,
        _ => return 0,
    };
    if dir == preferred { 32 } else { 0 }
}

pub(crate) fn food_deposit_for_carry_ticks(carry_ticks: u16) -> u8 {
    let decay = (carry_ticks / crate::WORKER_FOOD_DEPOSIT_DECAY_STEPS) as u8;
    crate::WORKER_FOOD_DEPOSIT_PEAK
        .saturating_sub(decay)
        .max(crate::WORKER_FOOD_DEPOSIT_FLOOR)
}

pub(crate) fn home_deposit_for_trail_steps(trail_steps: u16) -> u8 {
    let decay = (trail_steps / crate::constants::WORKER_HOME_DEPOSIT_DECAY_STEPS) as u8;
    crate::WORKER_HOME_DEPOSIT.saturating_sub(decay)
}

pub(crate) fn worker_lifespan_bonus(age_ticks: u16, default_max_life_span: u16) -> u16 {
    if default_max_life_span == 0 {
        return 0;
    }
    let remaining = default_max_life_span.saturating_sub(age_ticks) as u32;
    ((remaining * 200) / u32::from(default_max_life_span)) as u16
}

pub(crate) fn axes_json((left, right, up, down): (u32, u32, u32, u32)) -> Value {
    json!({
        "left": left,
        "right": right,
        "up": up,
        "down": down,
    })
}

pub(crate) fn dir_name(dir: MoveDir) -> &'static str {
    match dir {
        MoveDir::Up => "up",
        MoveDir::Down => "down",
        MoveDir::Left => "left",
        MoveDir::Right => "right",
    }
}

pub(crate) fn tile_name(tile: Tile) -> &'static str {
    match tile {
        Tile::Empty => "empty",
        Tile::Dirt => "dirt",
        Tile::Stone => "stone",
        Tile::Resource => "resource",
        Tile::Food => "food",
        Tile::Bedrock => "bedrock",
    }
}

pub(crate) fn behavior_name(behavior: AntBehaviorState) -> &'static str {
    match behavior {
        AntBehaviorState::Searching => "searching",
        AntBehaviorState::ReturningFood => "returning_food",
        AntBehaviorState::Defending => "defending",
        AntBehaviorState::Idle => "idle",
    }
}
