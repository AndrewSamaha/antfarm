use anyhow::{Context, Result};
use antfarm_core::{DAY_TICKS, GameState, NpcKind, TICK_MILLIS};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ServerConfigFile {
    #[serde(default)]
    pub(crate) config: Value,
    #[serde(default)]
    pub(crate) startup: StartupConfig,
    #[serde(default)]
    pub(crate) experiment: ExperimentConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct StartupConfig {
    #[serde(default)]
    pub(crate) paused: bool,
    #[serde(default)]
    pub(crate) reset_world: bool,
    #[serde(default)]
    pub(crate) load_gamestate: Option<String>,
    #[serde(default)]
    pub(crate) sc_commands: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ExperimentConfig {
    #[serde(default)]
    pub(crate) debug_log: bool,
    #[serde(default)]
    pub(crate) terminate_server_on_completion: bool,
    #[serde(default)]
    pub(crate) randomize_seed_from_datetime: bool,
    #[serde(default)]
    pub(crate) tick_millis: Option<u64>,
    #[serde(default)]
    pub(crate) stop_conditions: StopConditionExpr,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum StopConditionExpr {
    And { and: Vec<StopConditionExpr> },
    Or { or: Vec<StopConditionExpr> },
    Pred(StopPredicateSpec),
}

impl Default for StopConditionExpr {
    fn default() -> Self {
        Self::Pred(StopPredicateSpec::default())
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct StopPredicateSpec {
    #[serde(default)]
    pub(crate) max_tick: Option<u64>,
    #[serde(default)]
    pub(crate) max_day: Option<u64>,
    #[serde(default)]
    pub(crate) all_workers_dead: bool,
    #[serde(default)]
    pub(crate) no_eggs: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedServerConfig {
    pub(crate) path: Option<PathBuf>,
    pub(crate) file: ServerConfigFile,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ExperimentRunContext {
    pub(crate) source_config_path: PathBuf,
    pub(crate) run_dir: PathBuf,
    pub(crate) debug_log: bool,
    pub(crate) terminate_server_on_completion: bool,
    pub(crate) randomized_seed: Option<u64>,
    pub(crate) tick_millis: u64,
    pub(crate) stop_conditions: StopConditionExpr,
    #[serde(skip_serializing)]
    pub(crate) finished: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) enum StopReason {
    MaxTick(u64),
    MaxDay(u64),
    AllWorkersDead,
    NoEggs,
    And(Vec<StopReason>),
}

pub(crate) fn load_server_config(path_arg: Option<&str>) -> Result<LoadedServerConfig> {
    let path = match path_arg {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            let default = PathBuf::from("server.yaml");
            default.exists().then_some(default)
        }
    };

    let file = if let Some(path) = &path {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("read server config at {}", path.display()))?;
        serde_yaml::from_str::<ServerConfigFile>(&raw)
            .with_context(|| format!("parse yaml server config at {}", path.display()))?
    } else {
        ServerConfigFile::default()
    };

    Ok(LoadedServerConfig { path, file })
}

pub(crate) fn maybe_create_run_context(
    config_path: Option<&Path>,
    experiment_config: &ExperimentConfig,
) -> Result<Option<ExperimentRunContext>> {
    let Some(config_path) = config_path else {
        return Ok(None);
    };
    let Some(experiment_dir) = config_path.parent() else {
        return Ok(None);
    };
    let Some(parent) = experiment_dir.parent() else {
        return Ok(None);
    };
    if parent.file_name().and_then(|name| name.to_str()) != Some("experiments") {
        return Ok(None);
    }

    let run_dir = experiment_dir.join("runs").join(run_name()?);
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("create experiment run dir {}", run_dir.display()))?;
    fs::copy(config_path, run_dir.join("server.yaml")).with_context(|| {
        format!(
            "copy server config {} into run dir {}",
            config_path.display(),
            run_dir.display()
        )
    })?;

    let context = ExperimentRunContext {
        source_config_path: config_path.to_path_buf(),
        run_dir: run_dir.clone(),
        debug_log: experiment_config.debug_log,
        terminate_server_on_completion: experiment_config.terminate_server_on_completion,
        randomized_seed: None,
        tick_millis: experiment_config.tick_millis.unwrap_or(TICK_MILLIS).max(1),
        stop_conditions: experiment_config.stop_conditions.clone(),
        finished: false,
    };
    write_run_manifest(&context)?;
    Ok(Some(context))
}

pub(crate) fn stop_reason(stop_conditions: &StopConditionExpr, game: &GameState) -> Option<StopReason> {
    match stop_conditions {
        StopConditionExpr::And { and } => {
            if and.is_empty() {
                return None;
            }
            let mut reasons = Vec::with_capacity(and.len());
            for condition in and {
                let reason = stop_reason(condition, game)?;
                reasons.push(reason);
            }
            Some(StopReason::And(reasons))
        }
        StopConditionExpr::Or { or } => {
            for condition in or {
                if let Some(reason) = stop_reason(condition, game) {
                    return Some(reason);
                }
            }
            None
        }
        StopConditionExpr::Pred(spec) => stop_reason_for_predicate(spec, game),
    }
}

pub(crate) fn write_run_result(
    context: &ExperimentRunContext,
    reason: &StopReason,
    game: &GameState,
) -> Result<()> {
    let worker_count = game
        .npcs
        .iter()
        .filter(|npc| npc.kind == NpcKind::Worker)
        .count();
    let queen_count = game
        .npcs
        .iter()
        .filter(|npc| npc.kind == NpcKind::Queen)
        .count();
    let egg_count = game
        .npcs
        .iter()
        .filter(|npc| npc.kind == NpcKind::Egg)
        .count();
    let summary = json!({
        "stop_reason": reason,
        "tick": game.tick,
        "day": game.tick / DAY_TICKS + 1,
        "simulation_paused": game.simulation_paused,
        "found_food": game.found_food_count,
        "delivered_food": game.delivered_food_count,
        "egg_laid": game.egg_laid_count,
        "egg_hatched": game.egg_hatched_count,
        "workers": worker_count,
        "queens": queen_count,
        "eggs": egg_count,
        "players": game.players.len(),
        "world": {
            "width": game.world.width(),
            "height": game.world.height(),
        }
    });
    fs::write(
        context.run_dir.join("result.json"),
        serde_json::to_vec_pretty(&summary)?,
    )
    .with_context(|| format!("write experiment result {}", context.run_dir.join("result.json").display()))?;
    write_experiment_results_html(context)?;
    Ok(())
}

pub(crate) fn persist_run_manifest(context: &ExperimentRunContext) -> Result<()> {
    write_run_manifest(context)
}

fn write_run_manifest(context: &ExperimentRunContext) -> Result<()> {
    let manifest = json!({
        "source_config_path": context.source_config_path.display().to_string(),
        "run_dir": context.run_dir.display().to_string(),
        "debug_log": context.debug_log,
        "terminate_server_on_completion": context.terminate_server_on_completion,
        "randomized_seed": context.randomized_seed,
        "tick_millis": context.tick_millis,
        "stop_conditions": context.stop_conditions,
    });
    fs::write(
        context.run_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )
    .with_context(|| format!("write run manifest {}", context.run_dir.join("manifest.json").display()))?;
    Ok(())
}

fn write_experiment_results_html(context: &ExperimentRunContext) -> Result<()> {
    let Some(experiment_dir) = context.source_config_path.parent() else {
        return Ok(());
    };
    let runs_dir = experiment_dir.join("runs");
    if !runs_dir.exists() {
        return Ok(());
    }

    let mut run_count = 0usize;
    let mut stop_reason_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut metrics: BTreeMap<String, MetricSamples> = BTreeMap::new();

    for entry in fs::read_dir(&runs_dir)
        .with_context(|| format!("read experiment runs dir {}", runs_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let result_path = entry.path().join("result.json");
        if !result_path.exists() {
            continue;
        }
        let raw = fs::read_to_string(&result_path)
            .with_context(|| format!("read experiment result {}", result_path.display()))?;
        let result: Value = serde_json::from_str(&raw)
            .with_context(|| format!("parse experiment result {}", result_path.display()))?;
        run_count = run_count.saturating_add(1);

        if let Some(stop_reason) = result.get("stop_reason") {
            let label = stop_reason_label(stop_reason);
            *stop_reason_counts.entry(label).or_default() += 1;
        }
        collect_numeric_metrics(None, &result, &mut metrics);
    }

    let html = render_experiment_results_html(
        experiment_dir,
        run_count,
        &stop_reason_counts,
        &metrics,
    );
    fs::write(experiment_dir.join("results.html"), html)
        .with_context(|| format!("write experiment summary {}", experiment_dir.join("results.html").display()))?;
    Ok(())
}

#[derive(Default)]
struct MetricSamples {
    values: Vec<f64>,
    labels: Vec<String>,
}

fn collect_numeric_metrics(
    prefix: Option<&str>,
    value: &Value,
    metrics: &mut BTreeMap<String, MetricSamples>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next = match prefix {
                    Some(prefix) => format!("{prefix}.{key}"),
                    None => key.clone(),
                };
                collect_numeric_metrics(Some(&next), child, metrics);
            }
        }
        Value::Number(number) => {
            let Some(metric_name) = prefix else {
                return;
            };
            if let Some(as_f64) = number.as_f64() {
                let entry = metrics.entry(metric_name.to_string()).or_default();
                entry.values.push(as_f64);
                entry.labels.push(number.to_string());
            }
        }
        Value::Array(_) | Value::String(_) | Value::Bool(_) | Value::Null => {}
    }
}

fn render_experiment_results_html(
    experiment_dir: &Path,
    run_count: usize,
    stop_reason_counts: &BTreeMap<String, usize>,
    metrics: &BTreeMap<String, MetricSamples>,
) -> String {
    let experiment_name = experiment_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("experiment");
    let mut metrics_rows = String::new();
    for (metric, samples) in metrics {
        if samples.values.is_empty() {
            continue;
        }
        let stats = compute_stats(samples);
        metrics_rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html_escape(metric),
            stats.sample_count,
            format_number(stats.min),
            format_number(stats.max),
            format_number(stats.mean),
            format_number(stats.median),
            html_escape(&stats.mode),
        ));
    }
    if metrics_rows.is_empty() {
        metrics_rows.push_str("<tr><td colspan=\"7\">No numeric metrics found.</td></tr>");
    }

    let mut stop_reason_rows = String::new();
    for (reason, count) in stop_reason_counts {
        stop_reason_rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td></tr>",
            html_escape(reason),
            count
        ));
    }
    if stop_reason_rows.is_empty() {
        stop_reason_rows.push_str("<tr><td colspan=\"2\">No completed runs yet.</td></tr>");
    }

    format!(
        "<!doctype html>\
<html lang=\"en\">\
<head>\
<meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title>\
<style>\
body {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; margin: 24px; color: #111; background: #faf8f2; }}\
h1, h2 {{ margin: 0 0 12px; }}\
p {{ margin: 0 0 16px; }}\
table {{ border-collapse: collapse; width: 100%; margin: 0 0 24px; background: white; }}\
th, td {{ border: 1px solid #d7d0c3; padding: 8px 10px; text-align: left; vertical-align: top; }}\
th {{ background: #efe7d8; }}\
code {{ background: #efe7d8; padding: 1px 4px; }}\
</style>\
</head>\
<body>\
<h1>{title}</h1>\
<p>Samples: <strong>{run_count}</strong></p>\
<h2>Stop Reasons</h2>\
<table>\
<thead><tr><th>Reason</th><th>Count</th></tr></thead>\
<tbody>{stop_reason_rows}</tbody>\
</table>\
<h2>Metric Statistics</h2>\
<table>\
<thead><tr><th>Metric</th><th>Samples</th><th>Min</th><th>Max</th><th>Average</th><th>Median</th><th>Mode</th></tr></thead>\
<tbody>{metrics_rows}</tbody>\
</table>\
</body>\
</html>",
        title = html_escape(&format!("Experiment Results: {experiment_name}")),
        run_count = run_count,
        stop_reason_rows = stop_reason_rows,
        metrics_rows = metrics_rows,
    )
}

struct MetricStats {
    sample_count: usize,
    min: f64,
    max: f64,
    mean: f64,
    median: f64,
    mode: String,
}

fn compute_stats(samples: &MetricSamples) -> MetricStats {
    let mut sorted = samples.values.clone();
    sorted.sort_by(f64::total_cmp);
    let sample_count = sorted.len();
    let min = *sorted.first().unwrap_or(&0.0);
    let max = *sorted.last().unwrap_or(&0.0);
    let sum: f64 = sorted.iter().sum();
    let mean = if sample_count == 0 {
        0.0
    } else {
        sum / sample_count as f64
    };
    let median = if sample_count == 0 {
        0.0
    } else if sample_count % 2 == 1 {
        sorted[sample_count / 2]
    } else {
        (sorted[sample_count / 2 - 1] + sorted[sample_count / 2]) / 2.0
    };

    let mut frequencies: BTreeMap<&str, usize> = BTreeMap::new();
    for label in &samples.labels {
        *frequencies.entry(label.as_str()).or_default() += 1;
    }
    let max_frequency = frequencies.values().copied().max().unwrap_or(0);
    let mode = if max_frequency <= 1 {
        "none".to_string()
    } else {
        frequencies
            .into_iter()
            .filter_map(|(label, count)| (count == max_frequency).then_some(label.to_string()))
            .collect::<Vec<_>>()
            .join(", ")
    };

    MetricStats {
        sample_count,
        min,
        max,
        mean,
        median,
        mode,
    }
}

fn stop_reason_label(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "unknown".to_string()),
    }
}

fn format_number(value: f64) -> String {
    if (value.fract()).abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.3}")
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&#39;")
}

pub(crate) fn debug_log_path(context: &ExperimentRunContext) -> PathBuf {
    context.run_dir.join("data").join("debuglog.sqlite")
}

pub(crate) fn datetime_seed() -> Result<u64> {
    Ok(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before unix epoch")?
            .as_millis() as u64,
    )
}

fn stop_reason_for_predicate(spec: &StopPredicateSpec, game: &GameState) -> Option<StopReason> {
    if let Some(max_tick) = spec.max_tick
        && game.tick >= max_tick
    {
        return Some(StopReason::MaxTick(max_tick));
    }
    if let Some(max_day) = spec.max_day {
        let day = game.tick / DAY_TICKS + 1;
        if day >= max_day {
            return Some(StopReason::MaxDay(max_day));
        }
    }
    if spec.all_workers_dead
        && !game.npcs.iter().any(|npc| npc.kind == NpcKind::Worker)
    {
        return Some(StopReason::AllWorkersDead);
    }
    if spec.no_eggs
        && !game.npcs.iter().any(|npc| npc.kind == NpcKind::Egg)
    {
        return Some(StopReason::NoEggs);
    }
    None
}

fn run_name() -> Result<String> {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before unix epoch")?
        .as_millis();
    Ok(format!("run-{ts_ms}"))
}
