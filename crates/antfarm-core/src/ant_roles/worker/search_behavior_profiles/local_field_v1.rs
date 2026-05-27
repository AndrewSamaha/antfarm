use crate::game_state::GameState;

use super::{SearchCandidateContext, SearchCandidateScoring, SearchTickContext};

pub(super) fn score_candidate(
    game: &GameState,
    ctx: &SearchTickContext,
    candidate: SearchCandidateContext,
) -> SearchCandidateScoring {
    let local_field_search = ctx.npc_hive.map(|hive_id| {
        game.local_field_search_bias(candidate.next, hive_id, ctx.sensory_radius)
    });
    SearchCandidateScoring {
        local_field_search,
        ..SearchCandidateScoring::default()
    }
}
