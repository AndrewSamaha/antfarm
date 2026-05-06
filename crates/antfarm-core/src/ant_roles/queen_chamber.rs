use rand::Rng;

use crate::{
    game_state::GameState,
    types::{Position, QueenChamberGrowthMode},
};

const INITIAL_QUEEN_CHAMBER_RADIUS: i32 = 2;

pub(crate) fn tick(
    game: &mut GameState,
    index: usize,
    queen_pos: Option<Position>,
    events: &mut Vec<String>,
) {
    game.tick_queen_chamber_worker(index, queen_pos, events);
}

pub(crate) fn on_hatch(game: &mut GameState, index: usize) {
    let (max_x, max_y) = game.queen_chamber_max_radii();
    let growth_mode = game.next_queen_chamber_growth_mode();
    let (radius_x, radius_y) = queen_chamber_initial_radii_for_mode(growth_mode, max_x, max_y);
    let worker = &mut game.npcs[index];
    worker.behavior = crate::AntBehaviorState::Idle;
    worker.home_trail_steps = None;
    worker.chamber_growth_mode = growth_mode;
    worker.chamber_radius_x = Some(radius_x);
    worker.chamber_radius_y = Some(radius_y);
    worker.chamber_anchor = None;
    worker.chamber_has_left_anchor = false;
}

pub(crate) fn random_queen_chamber_growth_mode<R: Rng + ?Sized>(
    rng: &mut R,
) -> QueenChamberGrowthMode {
    if rng.random::<bool>() {
        QueenChamberGrowthMode::Outward
    } else {
        QueenChamberGrowthMode::Inward
    }
}

pub(crate) fn queen_chamber_initial_radii_for_mode(
    mode: QueenChamberGrowthMode,
    max_x: i32,
    max_y: i32,
) -> (i32, i32) {
    let initial_radius_x = INITIAL_QUEEN_CHAMBER_RADIUS.min(max_x);
    let initial_radius_y = INITIAL_QUEEN_CHAMBER_RADIUS.min(max_y);
    match mode {
        QueenChamberGrowthMode::Outward => (initial_radius_x, initial_radius_y),
        QueenChamberGrowthMode::Inward => (max_x, max_y),
    }
}
