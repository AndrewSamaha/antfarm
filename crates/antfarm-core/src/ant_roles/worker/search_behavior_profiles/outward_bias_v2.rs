use crate::game_state::GameState;

use super::{
    PreparedSearchProfile, SearchCandidateContext, SearchCandidateScoring, SearchTickContext,
};
use crate::ant_roles::worker::{
    find_visible_food_target, find_visible_higher_food_pheromone_target, target_approach_bias,
};

pub(super) fn prepare(game: &GameState, ctx: &SearchTickContext) -> PreparedSearchProfile {
    let visible_food_target = if ctx.behavior == crate::pheromones::AntBehaviorState::Searching {
        find_visible_food_target(game, ctx.npc_pos, ctx.sensory_radius)
    } else {
        None
    };
    let visible_food_pheromone_target = if ctx.behavior
        == crate::pheromones::AntBehaviorState::Searching
        && visible_food_target.is_none()
    {
        ctx.npc_hive.and_then(|hive_id| {
            find_visible_higher_food_pheromone_target(
                game,
                ctx.npc_pos,
                hive_id,
                ctx.sensory_radius,
                ctx.current_food_pheromone,
            )
        })
    } else {
        None
    };
    PreparedSearchProfile {
        visible_food_target,
        visible_food_pheromone_target,
    }
}

pub(super) fn score_candidate(
    ctx: &SearchTickContext,
    prepared: &PreparedSearchProfile,
    candidate: SearchCandidateContext,
) -> SearchCandidateScoring {
    let search_profile_bias = match ctx.queen_pos {
        Some(queen_pos)
            if prepared.visible_food_target.is_none()
                && prepared.visible_food_pheromone_target.is_none() =>
        {
            let current =
                (queen_pos.x - ctx.npc_pos.x).abs() + (queen_pos.y - ctx.npc_pos.y).abs();
            let next_dist = (queen_pos.x - candidate.next.x).abs()
                + (queen_pos.y - candidate.next.y).abs();
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
    let visible_food_target_bias = prepared
        .visible_food_target
        .map(|target| target_approach_bias(ctx.npc_pos, candidate.next, target, 320, 180, 16))
        .unwrap_or(0);
    let food_pheromone_target_bias = prepared
        .visible_food_pheromone_target
        .map(|target| target_approach_bias(ctx.npc_pos, candidate.next, target, 260, 140, 12))
        .unwrap_or(0);
    SearchCandidateScoring {
        search_profile_bias,
        visible_food_target_bias,
        food_pheromone_target_bias,
        ..SearchCandidateScoring::default()
    }
}
