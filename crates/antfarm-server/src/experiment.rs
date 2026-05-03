use antfarm_core::{
    DAY_TICKS, GameState, NpcKind, ReplayArtifact, Snapshot, TICK_MILLIS, merge_config,
    set_config_path,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use crate::logging::emit_log;

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
pub(crate) struct StartupOverride {
    pub(crate) paused: Option<bool>,
    pub(crate) reset_world: Option<bool>,
    pub(crate) load_gamestate: Option<String>,
    pub(crate) sc_commands: Option<Vec<String>>,
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
    pub(crate) analysis: ExperimentAnalysisConfig,
    #[serde(default)]
    pub(crate) visualizations: Vec<ExperimentVisualizationSpec>,
    #[serde(default)]
    pub(crate) conditions: Vec<ExperimentCondition>,
    #[serde(default)]
    pub(crate) stop_conditions: StopConditionExpr,
    #[serde(default)]
    pub(crate) replay: ExperimentReplayConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct ExperimentReplayConfig {
    #[serde(default)]
    pub(crate) save: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ExperimentCondition {
    pub(crate) name: String,
    #[serde(default = "default_condition_runs")]
    pub(crate) runs: u32,
    #[serde(default)]
    pub(crate) parameter: Option<ExperimentConditionParameterSweep>,
    #[serde(default)]
    pub(crate) config: Value,
    #[serde(default)]
    pub(crate) startup: StartupOverride,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ExperimentConditionParameterSweep {
    pub(crate) path: String,
    pub(crate) values: Vec<Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct ExperimentAnalysisConfig {
    #[serde(default)]
    pub(crate) metrics: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct ExperimentVisualizationSpec {
    #[serde(rename = "type")]
    pub(crate) type_name: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) x_metric: String,
    #[serde(default)]
    pub(crate) y_metric: String,
    #[serde(default)]
    pub(crate) x_aggregate: String,
    #[serde(default)]
    pub(crate) y_aggregate: String,
    #[serde(default)]
    pub(crate) metric: String,
    #[serde(default)]
    pub(crate) aggregate: String,
    #[serde(default)]
    pub(crate) event: String,
    #[serde(default)]
    pub(crate) x_axis: String,
    #[serde(default)]
    pub(crate) group_by: String,
    #[serde(default)]
    pub(crate) output: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) x_label: String,
    #[serde(default)]
    pub(crate) y_label: String,
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

#[derive(Debug, Clone)]
pub(crate) struct ResolvedServerConfig {
    pub(crate) config: Value,
    pub(crate) startup: StartupConfig,
    pub(crate) experiment: ExperimentConfig,
    pub(crate) condition_name: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ConditionPlanEntry {
    pub(crate) name: Option<String>,
    pub(crate) runs: u32,
}

#[derive(Debug, Clone)]
struct ExpandedCondition {
    name: String,
    runs: u32,
    config: Value,
    startup: StartupOverride,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ExperimentRunContext {
    pub(crate) source_config_path: PathBuf,
    pub(crate) experiment_dir: PathBuf,
    pub(crate) condition_name: Option<String>,
    pub(crate) run_dir: PathBuf,
    pub(crate) start_tick: u64,
    pub(crate) debug_log: bool,
    pub(crate) terminate_server_on_completion: bool,
    pub(crate) randomized_seed: Option<u64>,
    pub(crate) tick_millis: u64,
    pub(crate) analysis_metrics: Vec<String>,
    pub(crate) visualizations: Vec<ExperimentVisualizationSpec>,
    pub(crate) stop_conditions: StopConditionExpr,
    pub(crate) replay_save: bool,
    #[serde(skip_serializing)]
    pub(crate) finished: bool,
    #[serde(skip_serializing)]
    pub(crate) initial_snapshot: Option<Snapshot>,
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
    condition_name: Option<&str>,
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

    let run_dir = if let Some(condition_name) = condition_name {
        experiment_dir
            .join(condition_name)
            .join("runs")
            .join(run_name()?)
    } else {
        experiment_dir.join("runs").join(run_name()?)
    };
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
        experiment_dir: experiment_dir.to_path_buf(),
        condition_name: condition_name.map(ToOwned::to_owned),
        run_dir: run_dir.clone(),
        start_tick: 0,
        debug_log: experiment_config.debug_log,
        terminate_server_on_completion: experiment_config.terminate_server_on_completion,
        randomized_seed: None,
        tick_millis: experiment_config.tick_millis.unwrap_or(TICK_MILLIS).max(1),
        analysis_metrics: experiment_config.analysis.metrics.clone(),
        visualizations: experiment_config.visualizations.clone(),
        stop_conditions: experiment_config.stop_conditions.clone(),
        replay_save: experiment_config.replay.save,
        finished: false,
        initial_snapshot: None,
    };
    write_run_manifest(&context)?;
    Ok(Some(context))
}

pub(crate) fn resolve_server_config(
    file: &ServerConfigFile,
    condition_selector: Option<&str>,
) -> Result<ResolvedServerConfig> {
    if file.experiment.conditions.is_empty() {
        let mut experiment = file.experiment.clone();
        experiment.conditions.clear();
        return Ok(ResolvedServerConfig {
            config: file.config.clone(),
            startup: file.startup.clone(),
            experiment,
            condition_name: None,
        });
    }

    let selector = condition_selector.ok_or_else(|| {
        anyhow::anyhow!(
            "server config defines experiment.conditions; use --condition <name> or ./antfarm experiment"
        )
    })?;
    let expanded_conditions = expand_conditions(file)?;
    let condition = expanded_conditions
        .iter()
        .find(|condition| condition.name == selector)
        .ok_or_else(|| anyhow::anyhow!("unknown condition: {selector}"))?;

    let mut experiment = file.experiment.clone();
    experiment.conditions.clear();

    Ok(ResolvedServerConfig {
        config: merge_config(file.config.clone(), condition.config.clone()),
        startup: merge_startup(&file.startup, &condition.startup),
        experiment,
        condition_name: Some(condition.name.clone()),
    })
}

pub(crate) fn condition_plan(file: &ServerConfigFile) -> Result<Vec<ConditionPlanEntry>> {
    if file.experiment.conditions.is_empty() {
        return Ok(vec![ConditionPlanEntry {
            name: None,
            runs: 1,
        }]);
    }

    Ok(expand_conditions(file)?
        .iter()
        .map(|condition| ConditionPlanEntry {
            name: Some(condition.name.clone()),
            runs: condition.runs.max(1),
        })
        .collect())
}

fn expand_conditions(file: &ServerConfigFile) -> Result<Vec<ExpandedCondition>> {
    let mut expanded = Vec::new();
    for condition in &file.experiment.conditions {
        match &condition.parameter {
            Some(parameter) => {
                if parameter.path.trim().is_empty() {
                    anyhow::bail!("condition {} has an empty parameter path", condition.name);
                }
                if parameter.values.is_empty() {
                    anyhow::bail!("condition {} has no parameter values", condition.name);
                }
                let parameter_key = parameter_key_segment(&parameter.path);
                for value in &parameter.values {
                    let mut condition_config = condition.config.clone();
                    set_config_path(&mut condition_config, &parameter.path, value.clone())
                        .map_err(anyhow::Error::msg)?;
                    expanded.push(ExpandedCondition {
                        name: format!(
                            "{}__{}={}",
                            condition.name,
                            parameter_key,
                            parameter_value_label(value)
                        ),
                        runs: condition.runs.max(1),
                        config: condition_config,
                        startup: condition.startup.clone(),
                    });
                }
            }
            None => expanded.push(ExpandedCondition {
                name: condition.name.clone(),
                runs: condition.runs.max(1),
                config: condition.config.clone(),
                startup: condition.startup.clone(),
            }),
        }
    }
    Ok(expanded)
}

fn parameter_key_segment(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or(path)
}

fn parameter_value_label(value: &Value) -> String {
    let raw = match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "value".to_string()),
    };
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn stop_reason(
    stop_conditions: &StopConditionExpr,
    game: &GameState,
) -> Option<StopReason> {
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
    let final_snapshot_hash = game
        .final_snapshot_hash_hex()
        .context("compute final snapshot hash")?;
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
        "randomized_seed": context.randomized_seed,
        "final_snapshot_hash": final_snapshot_hash,
        "start_tick": context.start_tick,
        "tick": game.tick,
        "simulation_length": game.tick.saturating_sub(context.start_tick),
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
        },
        "replay_artifact_path": context
            .replay_save
            .then(|| replay_artifact_path(context).display().to_string()),
    });
    fs::write(
        context.run_dir.join("result.json"),
        serde_json::to_vec_pretty(&summary)?,
    )
    .with_context(|| {
        format!(
            "write experiment result {}",
            context.run_dir.join("result.json").display()
        )
    })?;
    emit_log(
        "experiment_run_finished",
        json!({
            "run_dir": context.run_dir.display().to_string(),
            "experiment_dir": context.experiment_dir.display().to_string(),
            "condition": context.condition_name,
            "stop_reason": reason,
            "tick": game.tick,
            "simulation_length": game.tick.saturating_sub(context.start_tick),
        }),
    );
    if context.replay_save {
        write_replay_artifact(context, game, &final_snapshot_hash)
            .context("write replay artifact")?;
    }
    if let Err(error) = run_post_run_analysis(context) {
        write_analysis_error(context, "post_run_error.txt", &error.to_string())?;
    }
    if let Err(error) = run_experiment_aggregation(context) {
        write_analysis_error(context, "aggregation_error.txt", &error.to_string())?;
    }
    if let Err(error) = run_experiment_visualizations(context) {
        write_analysis_error(context, "visualization_error.txt", &error.to_string())?;
    }
    Ok(())
}

pub(crate) fn replay_artifact_path(context: &ExperimentRunContext) -> PathBuf {
    context.run_dir.join("replay").join("replay.json")
}

fn write_replay_artifact(
    context: &ExperimentRunContext,
    game: &GameState,
    final_snapshot_hash: &str,
) -> Result<()> {
    let initial_snapshot = context
        .initial_snapshot
        .clone()
        .context("missing initial snapshot for replay artifact")?;
    let artifact = ReplayArtifact::new(
        initial_snapshot,
        game.tick.saturating_sub(context.start_tick),
        final_snapshot_hash.to_string(),
        json!({
            "source": "experiment",
            "experiment_dir": context.experiment_dir.display().to_string(),
            "run_dir": context.run_dir.display().to_string(),
            "condition_name": context.condition_name,
            "randomized_seed": context.randomized_seed,
        }),
    )
    .context("build replay artifact")?;
    let path = replay_artifact_path(context);
    let Some(parent) = path.parent() else {
        anyhow::bail!("invalid replay artifact path: {}", path.display());
    };
    fs::create_dir_all(parent)
        .with_context(|| format!("create replay artifact dir {}", parent.display()))?;
    fs::write(&path, serde_json::to_vec_pretty(&artifact)?)
        .with_context(|| format!("write replay artifact {}", path.display()))?;
    Ok(())
}

pub(crate) fn persist_run_manifest(context: &ExperimentRunContext) -> Result<()> {
    write_run_manifest(context)
}

fn write_run_manifest(context: &ExperimentRunContext) -> Result<()> {
    let manifest = json!({
        "source_config_path": context.source_config_path.display().to_string(),
        "experiment_dir": context.experiment_dir.display().to_string(),
        "condition_name": context.condition_name,
        "run_dir": context.run_dir.display().to_string(),
        "start_tick": context.start_tick,
        "debug_log": context.debug_log,
        "terminate_server_on_completion": context.terminate_server_on_completion,
        "randomized_seed": context.randomized_seed,
        "tick_millis": context.tick_millis,
        "analysis_metrics": context.analysis_metrics,
        "visualizations": context.visualizations,
        "stop_conditions": context.stop_conditions,
        "replay": {
            "save": context.replay_save,
        },
    });
    fs::write(
        context.run_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )
    .with_context(|| {
        format!(
            "write run manifest {}",
            context.run_dir.join("manifest.json").display()
        )
    })?;
    Ok(())
}

fn merge_startup(base: &StartupConfig, override_config: &StartupOverride) -> StartupConfig {
    StartupConfig {
        paused: override_config.paused.unwrap_or(base.paused),
        reset_world: override_config.reset_world.unwrap_or(base.reset_world),
        load_gamestate: override_config
            .load_gamestate
            .clone()
            .or_else(|| base.load_gamestate.clone()),
        sc_commands: override_config
            .sc_commands
            .clone()
            .unwrap_or_else(|| base.sc_commands.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::ServerConfigFile;

    #[test]
    fn server_yaml_supports_nested_colony_roles() {
        let raw = r#"
config:
  colony:
    roles:
      food_gatherer:
        weight: 5
      hive_maintenance:
        queen_chamber:
          weight: 2
"#;

        let parsed: ServerConfigFile = serde_yaml::from_str(raw).expect("parse yaml server config");

        assert_eq!(
            parsed
                .config
                .pointer("/colony/roles/food_gatherer/weight")
                .and_then(serde_json::Value::as_u64),
            Some(5)
        );
        assert_eq!(
            parsed
                .config
                .pointer("/colony/roles/hive_maintenance/queen_chamber/weight")
                .and_then(serde_json::Value::as_u64),
            Some(2)
        );
    }
}

fn default_condition_runs() -> u32 {
    1
}

fn run_post_run_analysis(context: &ExperimentRunContext) -> Result<()> {
    if context.analysis_metrics.is_empty() {
        return Ok(());
    }
    let Some(experiment_dir) = context.source_config_path.parent() else {
        return Ok(());
    };
    let Some(experiments_dir) = experiment_dir.parent() else {
        return Ok(());
    };
    let Some(repo_root) = experiments_dir.parent() else {
        return Ok(());
    };

    let analysis_dir = context.run_dir.join("analysis");
    fs::create_dir_all(&analysis_dir)
        .with_context(|| format!("create run analysis dir {}", analysis_dir.display()))?;

    let mut command = Command::new("uv");
    command
        .current_dir(repo_root)
        .arg("run")
        .arg("--project")
        .arg("analysis")
        .arg("python")
        .arg("-m")
        .arg("antfarm_metrics.cli")
        .arg("--experiment-dir")
        .arg(experiment_dir)
        .arg("--run-dir")
        .arg(&context.run_dir);
    for metric in &context.analysis_metrics {
        command.arg("--metric").arg(metric);
    }

    emit_log(
        "experiment_analysis_started",
        json!({
            "run_dir": context.run_dir.display().to_string(),
            "experiment_dir": experiment_dir.display().to_string(),
            "condition": context.condition_name,
            "metrics": context.analysis_metrics,
        }),
    );
    let started = Instant::now();
    let output = command.output().with_context(|| {
        format!(
            "run experiment analysis hook for {}",
            context.run_dir.display()
        )
    })?;

    fs::write(analysis_dir.join("stdout.log"), &output.stdout).with_context(|| {
        format!(
            "write analysis stdout {}",
            analysis_dir.join("stdout.log").display()
        )
    })?;
    fs::write(analysis_dir.join("stderr.log"), &output.stderr).with_context(|| {
        format!(
            "write analysis stderr {}",
            analysis_dir.join("stderr.log").display()
        )
    })?;

    if !output.status.success() {
        anyhow::bail!("analysis hook exited with status {}", output.status);
    }
    emit_log(
        "experiment_analysis_finished",
        json!({
            "run_dir": context.run_dir.display().to_string(),
            "experiment_dir": experiment_dir.display().to_string(),
            "condition": context.condition_name,
            "duration_ms": started.elapsed().as_millis() as u64,
            "status": output.status.code(),
        }),
    );
    Ok(())
}

fn run_experiment_aggregation(context: &ExperimentRunContext) -> Result<()> {
    let experiment_dir = &context.experiment_dir;
    let Some(experiments_dir) = experiment_dir.parent() else {
        return Ok(());
    };
    let Some(repo_root) = experiments_dir.parent() else {
        return Ok(());
    };

    let analysis_dir = context.run_dir.join("analysis");
    fs::create_dir_all(&analysis_dir)
        .with_context(|| format!("create run analysis dir {}", analysis_dir.display()))?;

    let mut targets = Vec::new();
    if let Some(condition_name) = &context.condition_name {
        targets.push(experiment_dir.join(condition_name));
    }
    targets.push(experiment_dir.clone());

    for (index, target_dir) in targets.into_iter().enumerate() {
        let scope = if index == 0 && context.condition_name.is_some() {
            "condition"
        } else {
            "experiment"
        };
        emit_log(
            "experiment_aggregation_started",
            json!({
                "run_dir": context.run_dir.display().to_string(),
                "experiment_dir": experiment_dir.display().to_string(),
                "condition": context.condition_name,
                "target_dir": target_dir.display().to_string(),
                "scope": scope,
            }),
        );
        let started = Instant::now();
        let output = Command::new("uv")
            .current_dir(repo_root)
            .arg("run")
            .arg("--project")
            .arg("analysis")
            .arg("python")
            .arg("-m")
            .arg("antfarm_aggregation.cli")
            .arg("--experiment-dir")
            .arg(&target_dir)
            .output()
            .with_context(|| {
                format!(
                    "run experiment aggregation hook for {}",
                    target_dir.display()
                )
            })?;

        let suffix = scope;
        fs::write(
            analysis_dir.join(format!("aggregation_{suffix}_stdout.log")),
            &output.stdout,
        )
        .with_context(|| format!("write aggregation stdout for {}", target_dir.display()))?;
        fs::write(
            analysis_dir.join(format!("aggregation_{suffix}_stderr.log")),
            &output.stderr,
        )
        .with_context(|| format!("write aggregation stderr for {}", target_dir.display()))?;

        if !output.status.success() {
            anyhow::bail!("aggregation hook exited with status {}", output.status);
        }
        emit_log(
            "experiment_aggregation_finished",
            json!({
                "run_dir": context.run_dir.display().to_string(),
                "experiment_dir": experiment_dir.display().to_string(),
                "condition": context.condition_name,
                "target_dir": target_dir.display().to_string(),
                "scope": scope,
                "duration_ms": started.elapsed().as_millis() as u64,
                "status": output.status.code(),
            }),
        );
    }
    Ok(())
}

fn run_experiment_visualizations(context: &ExperimentRunContext) -> Result<()> {
    if context.visualizations.is_empty() {
        return Ok(());
    }
    let experiment_dir = &context.experiment_dir;
    let Some(experiments_dir) = experiment_dir.parent() else {
        return Ok(());
    };
    let Some(repo_root) = experiments_dir.parent() else {
        return Ok(());
    };

    let analysis_dir = context.run_dir.join("analysis");
    fs::create_dir_all(&analysis_dir)
        .with_context(|| format!("create run analysis dir {}", analysis_dir.display()))?;

    emit_log(
        "experiment_visualizations_started",
        json!({
            "run_dir": context.run_dir.display().to_string(),
            "experiment_dir": experiment_dir.display().to_string(),
            "condition": context.condition_name,
            "visualization_count": context.visualizations.len(),
        }),
    );
    let started = Instant::now();
    let output = Command::new("uv")
        .current_dir(repo_root)
        .arg("run")
        .arg("--project")
        .arg("analysis")
        .arg("python")
        .arg("-m")
        .arg("antfarm_visualizations.cli")
        .arg("--experiment-dir")
        .arg(experiment_dir)
        .arg("--run-dir")
        .arg(&context.run_dir)
        .output()
        .with_context(|| {
            format!(
                "run experiment visualization hook for {}",
                experiment_dir.display()
            )
        })?;

    fs::write(
        analysis_dir.join("visualization_stdout.log"),
        &output.stdout,
    )
    .with_context(|| {
        format!(
            "write visualization stdout for {}",
            experiment_dir.display()
        )
    })?;
    fs::write(
        analysis_dir.join("visualization_stderr.log"),
        &output.stderr,
    )
    .with_context(|| {
        format!(
            "write visualization stderr for {}",
            experiment_dir.display()
        )
    })?;

    if !output.status.success() {
        anyhow::bail!("visualization hook exited with status {}", output.status);
    }
    emit_log(
        "experiment_visualizations_finished",
        json!({
            "run_dir": context.run_dir.display().to_string(),
            "experiment_dir": experiment_dir.display().to_string(),
            "condition": context.condition_name,
            "visualization_count": context.visualizations.len(),
            "duration_ms": started.elapsed().as_millis() as u64,
            "status": output.status.code(),
        }),
    );
    Ok(())
}

fn write_analysis_error(
    context: &ExperimentRunContext,
    filename: &str,
    message: &str,
) -> Result<()> {
    let analysis_dir = context.run_dir.join("analysis");
    fs::create_dir_all(&analysis_dir)
        .with_context(|| format!("create run analysis dir {}", analysis_dir.display()))?;
    fs::write(analysis_dir.join(filename), message).with_context(|| {
        format!(
            "write analysis error {}",
            analysis_dir.join(filename).display()
        )
    })?;
    Ok(())
}

pub(crate) fn debug_log_path(context: &ExperimentRunContext) -> PathBuf {
    context.run_dir.join("data").join("debuglog.sqlite")
}

pub(crate) fn datetime_seed() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before unix epoch")?
        .as_millis() as u64)
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
    if spec.all_workers_dead && !game.npcs.iter().any(|npc| npc.kind == NpcKind::Worker) {
        return Some(StopReason::AllWorkersDead);
    }
    if spec.no_eggs && !game.npcs.iter().any(|npc| npc.kind == NpcKind::Egg) {
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
