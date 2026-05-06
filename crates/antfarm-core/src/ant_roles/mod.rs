mod food_gatherer;
pub(crate) mod queen_chamber;

use serde_json::Value;

use crate::{
    game_state::GameState,
    types::Position,
};

pub(crate) use queen_chamber::queen_chamber_initial_radii_for_mode;

pub(crate) const FOOD_GATHERER_ROLE_PATH: &str = crate::DEFAULT_WORKER_ROLE_PATH;
pub(crate) const QUEEN_CHAMBER_ROLE_PATH: &str = "hive_maintenance.queen_chamber";

#[derive(Debug, Clone)]
pub(crate) struct WorkerRoleDefinition {
    pub(crate) path: String,
    pub(crate) lifespan_ticks: u16,
    pub(crate) weight: u16,
}

pub(crate) fn tick_worker(
    game: &mut GameState,
    index: usize,
    queen_pos: Option<Position>,
    events: &mut Vec<String>,
) {
    match game.worker_role_path(index) {
        FOOD_GATHERER_ROLE_PATH => food_gatherer::tick(game, index, queen_pos, events),
        QUEEN_CHAMBER_ROLE_PATH => queen_chamber::tick(game, index, queen_pos, events),
        _ => set_idle(game, index),
    }
}

pub(crate) fn initialize_worker_role(game: &mut GameState, index: usize) {
    match game.worker_role_path(index) {
        FOOD_GATHERER_ROLE_PATH => food_gatherer::on_hatch(game, index),
        QUEEN_CHAMBER_ROLE_PATH => queen_chamber::on_hatch(game, index),
        _ => set_idle(game, index),
    }
}

pub(crate) fn configured_worker_roles(config: &Value) -> Vec<WorkerRoleDefinition> {
    let mut roles = Vec::new();
    if let Some(root) = config.pointer("/colony/roles") {
        collect_worker_roles(root, &mut Vec::new(), &mut roles);
    }
    if roles.is_empty() {
        roles.push(WorkerRoleDefinition {
            path: crate::DEFAULT_WORKER_ROLE_PATH.to_string(),
            lifespan_ticks: crate::NPC_WORKER_LIFESPAN_TICKS,
            weight: 1,
        });
    }
    roles.sort_by(|left, right| left.path.cmp(&right.path));
    roles
}

fn set_idle(game: &mut GameState, index: usize) {
    game.set_worker_idle(index);
}

fn collect_worker_roles(
    value: &Value,
    path: &mut Vec<String>,
    roles: &mut Vec<WorkerRoleDefinition>,
) {
    let Some(object) = value.as_object() else {
        return;
    };

    if !path.is_empty()
        && let Some(weight) = object
            .get("weight")
            .and_then(Value::as_u64)
            .and_then(|weight| u16::try_from(weight).ok())
        && weight > 0
    {
        let lifespan_ticks = object
            .get("lifespan")
            .and_then(Value::as_u64)
            .and_then(|lifespan| u16::try_from(lifespan).ok())
            .unwrap_or(crate::NPC_WORKER_LIFESPAN_TICKS);
        roles.push(WorkerRoleDefinition {
            path: path.join("."),
            lifespan_ticks,
            weight,
        });
    }

    let mut child_keys: Vec<_> = object
        .keys()
        .filter(|key| !matches!(key.as_str(), "weight" | "lifespan"))
        .collect();
    child_keys.sort();
    for key in child_keys {
        let Some(child) = object.get(key) else {
            continue;
        };
        if !child.is_object() {
            continue;
        }
        path.push(key.clone());
        collect_worker_roles(child, path, roles);
        path.pop();
    }
}
