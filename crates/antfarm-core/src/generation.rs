use rand::{Rng, SeedableRng, rngs::StdRng};
use std::collections::HashSet;

use crate::{
    config::{config_f64, config_i32},
    constants::{DEFAULT_WORLD_MAX_DEPTH, SURFACE_Y},
    types::{Position, Tile},
    world::World,
};

pub fn generate_world(seed: u64, width: i32, config: &serde_json::Value) -> World {
    let max_depth = config_i32(config, "world.max_depth", DEFAULT_WORLD_MAX_DEPTH).min(-1);
    let height = SURFACE_Y + max_depth.abs() + 1;
    let mut world = World::empty(width, height);

    let terrain_variation = config_i32(config, "world.gen_params.soil.surface_variation", 4).max(0);
    let dirt_depth = config_i32(config, "world.gen_params.soil.dirt_depth", 150).max(1);
    let dirt_variation = config_i32(config, "world.gen_params.soil.dirt_variation", 3).max(0);
    let chunk_width = config_i32(config, "world.gen_params.chunk_width", 16).clamp(4, 64);

    let surface_heights: Vec<i32> = (0..width)
        .map(|x| {
            let noise = fbm_1d(seed ^ 0x51_7A_2D, f64::from(x) * 0.045, 3);
            let offset = (noise * f64::from(terrain_variation)).round() as i32;
            (SURFACE_Y + offset).clamp(4, height - 3)
        })
        .collect();

    for x in 0..width {
        let surface_y = surface_heights[x as usize];
        let local_dirt_depth = (dirt_depth
            + (fbm_1d(seed ^ 0x92_11_4F, f64::from(x) * 0.09, 2) * f64::from(dirt_variation))
                .round() as i32)
            .max(1);

        for y in 0..height {
            let pos = Position { x, y };
            let tile = if y < surface_y {
                Tile::Empty
            } else if y == height - 1 {
                Tile::Bedrock
            } else {
                let depth = y - surface_y;
                if depth <= local_dirt_depth {
                    Tile::Dirt
                } else {
                    Tile::Stone
                }
            };
            world.set_tile(pos, tile);
        }
    }

    apply_cluster_pass(
        &mut world,
        seed ^ 0xA5_0E,
        chunk_width,
        Tile::Resource,
        &DepositConfig::from_config(
            config,
            "world.gen_params.ore",
            2,
            6,
            18,
            20,
            max_depth.abs() - 8,
        ),
        &[Tile::Stone],
        &surface_heights,
    );

    apply_depth_scaled_cluster_pass(
        &mut world,
        seed ^ 0x57_0A_E0,
        chunk_width,
        Tile::Stone,
        &DepthScaledDepositConfig::from_config(
            config,
            "world.gen_params.stone_pockets",
            1.0,
            4,
            12,
            6,
            max_depth.abs() - 20,
            1.8,
        ),
        &[Tile::Dirt],
        &surface_heights,
    );

    apply_cluster_pass(
        &mut world,
        seed ^ 0xF0_0D,
        chunk_width,
        Tile::Food,
        &DepositConfig::from_config(config, "world.gen_params.food", 3, 6, 14, 0, 50),
        &[Tile::Dirt, Tile::Stone],
        &surface_heights,
    );

    world
}

#[derive(Debug, Clone)]
struct DepositConfig {
    attempts_per_chunk: i32,
    cluster_min: i32,
    cluster_max: i32,
    min_depth: i32,
    max_depth: i32,
}

impl DepositConfig {
    fn from_config(
        config: &serde_json::Value,
        path: &str,
        attempts_per_chunk: i32,
        cluster_min: i32,
        cluster_max: i32,
        min_depth: i32,
        max_depth: i32,
    ) -> Self {
        Self {
            attempts_per_chunk: config_i32(
                config,
                &format!("{path}.attempts_per_chunk"),
                attempts_per_chunk,
            )
            .max(0),
            cluster_min: config_i32(config, &format!("{path}.cluster_min"), cluster_min).max(1),
            cluster_max: config_i32(config, &format!("{path}.cluster_max"), cluster_max).max(1),
            min_depth: config_i32(config, &format!("{path}.min_depth"), min_depth).max(0),
            max_depth: config_i32(config, &format!("{path}.max_depth"), max_depth).max(0),
        }
    }
}

#[derive(Debug, Clone)]
struct DepthScaledDepositConfig {
    attempts_per_chunk: f64,
    cluster_min: i32,
    cluster_max: i32,
    min_depth: i32,
    max_depth: i32,
    depth_gain: f64,
}

impl DepthScaledDepositConfig {
    fn from_config(
        config: &serde_json::Value,
        path: &str,
        attempts_per_chunk: f64,
        cluster_min: i32,
        cluster_max: i32,
        min_depth: i32,
        max_depth: i32,
        depth_gain: f64,
    ) -> Self {
        Self {
            attempts_per_chunk: config_f64(
                config,
                &format!("{path}.attempts_per_chunk"),
                attempts_per_chunk,
            )
            .max(0.0),
            cluster_min: config_i32(config, &format!("{path}.cluster_min"), cluster_min).max(1),
            cluster_max: config_i32(config, &format!("{path}.cluster_max"), cluster_max).max(1),
            min_depth: config_i32(config, &format!("{path}.min_depth"), min_depth).max(0),
            max_depth: config_i32(config, &format!("{path}.max_depth"), max_depth).max(0),
            depth_gain: config_f64(config, &format!("{path}.depth_gain"), depth_gain).max(0.1),
        }
    }
}

fn apply_cluster_pass(
    world: &mut World,
    seed: u64,
    chunk_width: i32,
    tile: Tile,
    deposit: &DepositConfig,
    replaceable: &[Tile],
    surface_heights: &[i32],
) {
    if deposit.attempts_per_chunk == 0 || deposit.max_depth < deposit.min_depth {
        return;
    }

    let chunks = (world.width() + chunk_width - 1) / chunk_width;
    for chunk_x in 0..chunks {
        let chunk_seed = seed ^ mix_u64(chunk_x as u64);
        let mut rng = StdRng::seed_from_u64(chunk_seed);
        let chunk_start = chunk_x * chunk_width;
        let chunk_end = (chunk_start + chunk_width).min(world.width());

        for _ in 0..deposit.attempts_per_chunk {
            let center_x = rng.random_range(chunk_start..chunk_end);
            let surface_y = surface_heights[center_x as usize];
            let min_y = (surface_y + deposit.min_depth).clamp(0, world.height() - 2);
            let max_y = (surface_y + deposit.max_depth).clamp(0, world.height() - 2);
            if min_y > max_y {
                continue;
            }

            let center = Position {
                x: center_x,
                y: rng.random_range(min_y..=max_y),
            };
            let cluster_max = deposit.cluster_max.max(deposit.cluster_min);
            let target_size = rng.random_range(deposit.cluster_min..=cluster_max);
            grow_cluster(
                world,
                &mut rng,
                center,
                target_size,
                tile,
                replaceable,
                min_y,
                max_y,
            );
        }
    }
}

fn apply_depth_scaled_cluster_pass(
    world: &mut World,
    seed: u64,
    chunk_width: i32,
    tile: Tile,
    deposit: &DepthScaledDepositConfig,
    replaceable: &[Tile],
    surface_heights: &[i32],
) {
    if deposit.attempts_per_chunk <= 0.0 || deposit.max_depth < deposit.min_depth {
        return;
    }

    let chunks = (world.width() + chunk_width - 1) / chunk_width;
    for chunk_x in 0..chunks {
        let chunk_seed = seed ^ mix_u64(chunk_x as u64);
        let mut rng = StdRng::seed_from_u64(chunk_seed);
        let chunk_start = chunk_x * chunk_width;
        let chunk_end = (chunk_start + chunk_width).min(world.width());

        let mut attempts = deposit.attempts_per_chunk.floor() as i32;
        let fractional = deposit.attempts_per_chunk.fract();
        if rng.random::<f64>() < fractional {
            attempts += 1;
        }

        for _ in 0..attempts.max(1) {
            let center_x = rng.random_range(chunk_start..chunk_end);
            let surface_y = surface_heights[center_x as usize];
            let min_y = (surface_y + deposit.min_depth).clamp(0, world.height() - 2);
            let max_y = (surface_y + deposit.max_depth).clamp(0, world.height() - 2);
            if min_y > max_y {
                continue;
            }

            let center_y = rng.random_range(min_y..=max_y);
            let depth = center_y - surface_y;
            let depth_span = (deposit.max_depth - deposit.min_depth).max(1);
            let depth_factor = ((depth - deposit.min_depth).max(0) as f64 / f64::from(depth_span))
                .clamp(0.0, 1.0)
                .powf(deposit.depth_gain);

            if rng.random::<f64>() > depth_factor {
                continue;
            }

            let cluster_max = deposit.cluster_max.max(deposit.cluster_min);
            let scaled_max = deposit.cluster_min
                + ((cluster_max - deposit.cluster_min) as f64 * depth_factor).round() as i32;
            let target_size =
                rng.random_range(deposit.cluster_min..=scaled_max.max(deposit.cluster_min));
            grow_cluster(
                world,
                &mut rng,
                Position {
                    x: center_x,
                    y: center_y,
                },
                target_size,
                tile,
                replaceable,
                min_y,
                max_y,
            );
        }
    }
}

fn grow_cluster(
    world: &mut World,
    rng: &mut StdRng,
    center: Position,
    target_size: i32,
    tile: Tile,
    replaceable: &[Tile],
    min_y: i32,
    max_y: i32,
) {
    let mut frontier = vec![center];
    let mut visited = HashSet::new();
    let mut placed = 0;

    while placed < target_size && !frontier.is_empty() {
        let index = rng.random_range(0..frontier.len());
        let pos = frontier.swap_remove(index);
        if !visited.insert(pos) || !world.in_bounds(pos) || pos.y < min_y || pos.y > max_y {
            continue;
        }

        if let Some(existing) = world.tile(pos) {
            if replaceable.contains(&existing) {
                world.set_tile(pos, tile);
                placed += 1;
            }
        }

        for next in [
            pos.offset(1, 0),
            pos.offset(-1, 0),
            pos.offset(0, 1),
            pos.offset(0, -1),
        ] {
            if rng.random::<f64>() < 0.78 {
                frontier.push(next);
            }
        }
    }
}

fn fbm_1d(seed: u64, x: f64, octaves: u32) -> f64 {
    let mut total = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut norm = 0.0;

    for octave in 0..octaves {
        total += value_noise_1d(seed ^ mix_u64(octave as u64), x * frequency) * amplitude;
        norm += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }

    if norm == 0.0 { 0.0 } else { total / norm }
}

fn value_noise_1d(seed: u64, x: f64) -> f64 {
    let x0 = x.floor() as i64;
    let x1 = x0 + 1;
    let t = x - x0 as f64;
    let v0 = random_unit(seed, x0 as u64);
    let v1 = random_unit(seed, x1 as u64);
    lerp(v0, v1, smoothstep(t))
}

fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn random_unit(seed: u64, value: u64) -> f64 {
    let mixed = mix_u64(seed ^ value);
    (mixed as f64 / u64::MAX as f64) * 2.0 - 1.0
}

fn mix_u64(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
