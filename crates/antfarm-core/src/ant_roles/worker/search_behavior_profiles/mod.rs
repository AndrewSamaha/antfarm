mod local_field_v1;
mod local_field_v2;
mod outward_bias_v1;
mod outward_bias_v2;

use crate::{
    game_state::GameState,
    pheromones::AntBehaviorState,
    types::Position,
};

use super::LocalFieldSearchScore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchBehaviorProfile {
    Baseline,
    OutwardBiasV1,
    OutwardBiasV2,
    LocalFieldV1,
    LocalFieldV2,
    OutwardBiasWithLocalFieldV1,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SearchTickContext {
    pub(crate) behavior: AntBehaviorState,
    pub(crate) npc_hive: Option<u16>,
    pub(crate) npc_pos: Position,
    pub(crate) queen_pos: Option<Position>,
    pub(crate) sensory_radius: i32,
    pub(crate) current_food_pheromone: u32,
    pub(crate) search_destination: Option<Position>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PreparedSearchProfile {
    pub(crate) visible_food_target: Option<Position>,
    pub(crate) visible_food_pheromone_target: Option<Position>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SearchCandidateContext {
    pub(crate) next: Position,
    pub(crate) food_pheromone: u32,
    pub(crate) home_pheromone: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SearchCandidateScoring {
    pub(crate) search_profile_bias: u32,
    pub(crate) visible_food_target_bias: u32,
    pub(crate) food_pheromone_target_bias: u32,
    pub(crate) local_field_search: Option<LocalFieldSearchScore>,
    pub(crate) destination_bias: u32,
}

pub(crate) fn search_behavior_profile_name(profile: SearchBehaviorProfile) -> &'static str {
    match profile {
        SearchBehaviorProfile::Baseline => "baseline",
        SearchBehaviorProfile::OutwardBiasV1 => "outward_bias_v1",
        SearchBehaviorProfile::OutwardBiasV2 => "outward_bias_v2",
        SearchBehaviorProfile::LocalFieldV1 => "local_field_v1",
        SearchBehaviorProfile::LocalFieldV2 => "local_field_v2",
        SearchBehaviorProfile::OutwardBiasWithLocalFieldV1 => "outward_bias_with_local_field_v1",
    }
}

pub(crate) fn prepare(
    game: &GameState,
    effective_profile: SearchBehaviorProfile,
    ctx: &SearchTickContext,
) -> PreparedSearchProfile {
    match effective_profile {
        SearchBehaviorProfile::OutwardBiasV2 => outward_bias_v2::prepare(game, ctx),
        _ => PreparedSearchProfile::default(),
    }
}

pub(crate) fn score_candidate(
    game: &GameState,
    effective_profile: SearchBehaviorProfile,
    ctx: &SearchTickContext,
    prepared: &PreparedSearchProfile,
    candidate: SearchCandidateContext,
) -> SearchCandidateScoring {
    match effective_profile {
        SearchBehaviorProfile::OutwardBiasV1 => outward_bias_v1::score_candidate(ctx, candidate),
        SearchBehaviorProfile::OutwardBiasV2 => {
            outward_bias_v2::score_candidate(ctx, prepared, candidate)
        }
        SearchBehaviorProfile::LocalFieldV1 => local_field_v1::score_candidate(game, ctx, candidate),
        SearchBehaviorProfile::LocalFieldV2 => local_field_v2::score_candidate(game, ctx, candidate),
        SearchBehaviorProfile::Baseline | SearchBehaviorProfile::OutwardBiasWithLocalFieldV1 => {
            SearchCandidateScoring::default()
        }
    }
}

pub(crate) fn pheromone_score(
    effective_profile: SearchBehaviorProfile,
    behavior: AntBehaviorState,
    current_food_pheromone: u32,
    has_increasing_adjacent_food_signal: bool,
    candidate: SearchCandidateContext,
    scoring: &SearchCandidateScoring,
) -> u32 {
    match behavior {
        AntBehaviorState::Searching if effective_profile == SearchBehaviorProfile::LocalFieldV1 => {
            scoring
                .local_field_search
                .map(|score| score.visible_food_bonus + score.food_field_score)
                .unwrap_or(0)
        }
        AntBehaviorState::Searching if effective_profile == SearchBehaviorProfile::LocalFieldV2 => {
            scoring.destination_bias
        }
        AntBehaviorState::Searching if has_increasing_adjacent_food_signal => {
            candidate
                .food_pheromone
                .saturating_sub(current_food_pheromone)
        }
        AntBehaviorState::Searching => 255_u32.saturating_sub(candidate.home_pheromone),
        AntBehaviorState::ReturningFood => candidate.home_pheromone,
        AntBehaviorState::Defending | AntBehaviorState::Idle => 0,
    }
}
