use super::{SearchCandidateContext, SearchCandidateScoring, SearchTickContext};

pub(super) fn score_candidate(
    ctx: &SearchTickContext,
    candidate: SearchCandidateContext,
) -> SearchCandidateScoring {
    let search_profile_bias = match ctx.queen_pos {
        Some(queen_pos) => {
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
        None => 0_u32,
    };
    SearchCandidateScoring {
        search_profile_bias,
        ..SearchCandidateScoring::default()
    }
}
