use crate::{game_state::GameState, types::Position};

pub(crate) fn tick(
    game: &mut GameState,
    index: usize,
    queen_pos: Option<Position>,
    events: &mut Vec<String>,
) {
    game.tick_food_gatherer_worker(index, queen_pos, events);
}

pub(crate) fn on_hatch(game: &mut GameState, index: usize) {
    game.npcs[index].behavior = crate::AntBehaviorState::Searching;
    game.npcs[index].home_trail_steps = Some(0);
}
