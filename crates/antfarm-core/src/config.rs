use serde_json::{Map, Value, json};

use crate::constants::{
    DEFAULT_SOIL_SETTLE_FREQUENCY, DEFAULT_WORLD_MAX_DEPTH,
    DEFAULT_WORLD_SNAPSHOT_INTERVAL_SECONDS, DEFAULT_WORLD_SEED,
};

pub fn default_server_config() -> Value {
    default_config()
}

pub fn merge_with_default_config(config: Value) -> Value {
    let mut merged = default_config();
    merge_config_value(&mut merged, migrate_legacy_config(config));
    merged
}

fn default_config() -> Value {
    json!({
        "soil": {
            "settle_frequency": DEFAULT_SOIL_SETTLE_FREQUENCY
        },
        "world": {
            "seed": DEFAULT_WORLD_SEED,
            "max_depth": DEFAULT_WORLD_MAX_DEPTH,
            "snapshot_interval": DEFAULT_WORLD_SNAPSHOT_INTERVAL_SECONDS,
            "gen_params": {
                "chunk_width": 16,
                "soil": {
                    "surface_variation": 4,
                    "dirt_depth": 150,
                    "dirt_variation": 3
                },
                "ore": {
                    "attempts_per_chunk": 2,
                    "cluster_min": 6,
                    "cluster_max": 18,
                    "min_depth": 20,
                    "max_depth": 220
                },
                "food": {
                    "attempts_per_chunk": 3,
                    "cluster_min": 6,
                    "cluster_max": 14,
                    "min_depth": 0,
                    "max_depth": 50
                },
                "stone_pockets": {
                    "attempts_per_chunk": 60.0,
                    "cluster_min": 1,
                    "cluster_max": 60,
                    "min_depth": 0,
                    "max_depth": 235,
                    "depth_gain": 0.00002
                }
            }
        }
    })
}

fn migrate_legacy_config(mut config: Value) -> Value {
    let Some(root) = config.as_object_mut() else {
        return config;
    };

    let terrain = root.remove("terrain");
    let ore = root.remove("ore");
    let food = root.remove("food");
    let chunk_width = root
        .get_mut("world")
        .and_then(Value::as_object_mut)
        .and_then(|world| world.remove("chunk_width"));

    let _ = root;

    if let Some(terrain) = terrain {
        let _ = set_config_path(&mut config, "world.gen_params.soil", terrain);
    }
    if let Some(ore) = ore {
        let _ = set_config_path(&mut config, "world.gen_params.ore", ore);
    }
    if let Some(food) = food {
        let _ = set_config_path(&mut config, "world.gen_params.food", food);
    }
    if let Some(chunk_width) = chunk_width {
        let _ = set_config_path(&mut config, "world.gen_params.chunk_width", chunk_width);
    }

    config
}

pub fn set_config_path(root: &mut Value, path: &str, value: Value) -> Result<(), String> {
    let segments: Vec<_> = path
        .split('.')
        .filter(|segment| !segment.trim().is_empty())
        .collect();
    if segments.is_empty() {
        return Err("config path cannot be empty".to_string());
    }

    if !root.is_object() {
        *root = Value::Object(Map::new());
    }

    let mut current = root;
    for segment in &segments[..segments.len() - 1] {
        let Some(object) = current.as_object_mut() else {
            return Err(format!("path segment {segment} is not an object"));
        };
        current = object
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !current.is_object() {
            *current = Value::Object(Map::new());
        }
    }

    let final_key = segments.last().expect("non-empty segments");
    let Some(object) = current.as_object_mut() else {
        return Err(format!("parent of {final_key} is not an object"));
    };
    object.insert((*final_key).to_string(), value);
    Ok(())
}

pub fn config_f64(root: &Value, path: &str, default: f64) -> f64 {
    let mut current = root;
    for segment in path.split('.').filter(|segment| !segment.trim().is_empty()) {
        let Some(next) = current.get(segment) else {
            return default;
        };
        current = next;
    }
    current.as_f64().unwrap_or(default)
}

pub fn config_i32(root: &Value, path: &str, default: i32) -> i32 {
    let mut current = root;
    for segment in path.split('.').filter(|segment| !segment.trim().is_empty()) {
        let Some(next) = current.get(segment) else {
            return default;
        };
        current = next;
    }
    current
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(default)
}

pub fn config_u64(root: &Value, path: &str, default: u64) -> u64 {
    let mut current = root;
    for segment in path.split('.').filter(|segment| !segment.trim().is_empty()) {
        let Some(next) = current.get(segment) else {
            return default;
        };
        current = next;
    }
    current.as_u64().unwrap_or(default)
}

fn merge_config_value(target: &mut Value, incoming: Value) {
    match (target, incoming) {
        (Value::Object(target_map), Value::Object(incoming_map)) => {
            for (key, value) in incoming_map {
                match target_map.get_mut(&key) {
                    Some(existing) => merge_config_value(existing, value),
                    None => {
                        target_map.insert(key, value);
                    }
                }
            }
        }
        (target, incoming) => {
            *target = incoming;
        }
    }
}
