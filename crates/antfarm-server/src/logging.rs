use antfarm_core::GameState;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn emit_log(event: &str, fields: Value) {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let mut object = serde_json::Map::new();
    object.insert("ts_ms".to_string(), Value::from(ts_ms));
    object.insert("event".to_string(), Value::from(event));
    if let Value::Object(extra) = fields {
        for (key, value) in extra {
            object.insert(key, value);
        }
    }
    println!("{}", Value::Object(object));
}

pub(crate) fn world_log_fields(game: &GameState) -> Value {
    json!({
        "tick": game.tick,
        "width": game.world.width(),
        "height": game.world.height(),
        "seed": game.config.pointer("/world/seed").and_then(Value::as_u64),
        "max_depth": game.config.pointer("/world/max_depth").and_then(Value::as_i64),
        "gen_params": game.config.pointer("/world/gen_params").cloned(),
    })
}
