use crate::game_state::GameState;

use super::{SearchCandidateContext, SearchCandidateScoring, SearchTickContext};
use crate::ant_roles::worker::local_field_destination_bias;

pub(super) fn score_candidate(
    game: &GameState,
    ctx: &SearchTickContext,
    candidate: SearchCandidateContext,
) -> SearchCandidateScoring {
    let local_field_search = ctx.npc_hive.map(|hive_id| {
        game.local_field_search_bias(candidate.next, hive_id, ctx.sensory_radius)
    });
    let destination_bias = ctx
        .search_destination
        .map(|destination| local_field_destination_bias(ctx.npc_pos, candidate.next, destination))
        .unwrap_or(0);
    SearchCandidateScoring {
        local_field_search,
        destination_bias,
        ..SearchCandidateScoring::default()
    }
}
