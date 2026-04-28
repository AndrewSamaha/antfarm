from __future__ import annotations

import json
import sqlite3
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from antfarm_metrics.steps import load_step_series_for_run


@dataclass(frozen=True)
class RunArtifacts:
    experiment_dir: Path
    run_dir: Path
    result: dict
    debuglog_path: Path


MetricValue = float | dict[str, float]
MetricFn = Callable[[RunArtifacts], MetricValue]


def compute_metrics(*, experiment_dir: Path, run_dir: Path, metric_names: list[str]) -> dict[str, MetricValue]:
    artifacts = load_artifacts(experiment_dir=experiment_dir, run_dir=run_dir)
    results: dict[str, MetricValue] = {}
    for metric_name in metric_names:
        results[metric_name] = resolve_metric(metric_name)(artifacts)
    return results


def load_artifacts(*, experiment_dir: Path, run_dir: Path) -> RunArtifacts:
    result_path = run_dir / "result.json"
    result = json.loads(result_path.read_text(encoding="utf-8"))
    return RunArtifacts(
        experiment_dir=experiment_dir,
        run_dir=run_dir,
        result=result,
        debuglog_path=run_dir / "data" / "debuglog.sqlite",
    )


def resolve_metric(name: str) -> MetricFn:
    registry: dict[str, MetricFn] = {
        "simulation_length": lambda a: simulation_length(a),
        "steps_to_nth_food": lambda a: nth_series_dict(load_step_series_for_run(a.run_dir)[0]),
        "steps_to_nth_queen": lambda a: nth_series_dict(load_step_series_for_run(a.run_dir)[1]),
        "final_tick": lambda a: metric_from_result(a, "tick"),
        "final_day": lambda a: metric_from_result(a, "day"),
        "found_food": lambda a: metric_from_result(a, "found_food"),
        "delivered_food": lambda a: metric_from_result(a, "delivered_food"),
        "egg_laid": lambda a: metric_from_result(a, "egg_laid"),
        "egg_hatched": lambda a: metric_from_result(a, "egg_hatched"),
        "end_workers": lambda a: metric_from_result(a, "workers"),
        "end_eggs": lambda a: metric_from_result(a, "eggs"),
        "end_queens": lambda a: metric_from_result(a, "queens"),
        "found_food_per_day": lambda a: safe_ratio(metric_from_result(a, "found_food"), metric_from_result(a, "day")),
        "delivered_food_per_day": lambda a: safe_ratio(metric_from_result(a, "delivered_food"), metric_from_result(a, "day")),
        "delivery_per_found_food_ratio": lambda a: safe_ratio(metric_from_result(a, "delivered_food"), metric_from_result(a, "found_food")),
        "egg_hatch_per_laid_ratio": lambda a: safe_ratio(metric_from_result(a, "egg_hatched"), metric_from_result(a, "egg_laid")),
        "selected_move_count": lambda a: debug_event_count(a, "selected_move"),
        "placed_dirt_count": lambda a: debug_event_count(a, "placed_dirt"),
        "memory_refresh_home_count": lambda a: debug_event_count(a, "memory_refresh_home"),
        "memory_refresh_food_count": lambda a: debug_event_count(a, "memory_refresh_food"),
    }
    if name.startswith("debug_event_count:"):
        event_type = name.split(":", 1)[1]
        return lambda a: debug_event_count(a, event_type)
    if name not in registry:
        available = ", ".join(sorted(registry.keys()) + ["debug_event_count:<event_type>"])
        raise ValueError(f"unknown metric '{name}'. available metrics: {available}")
    return registry[name]


def metric_from_result(artifacts: RunArtifacts, key: str) -> float:
    value = artifacts.result.get(key)
    if not isinstance(value, (int, float)):
        raise ValueError(f"result field '{key}' is not numeric")
    return float(value)


def debug_event_count(artifacts: RunArtifacts, event_type: str) -> float:
    if not artifacts.debuglog_path.exists():
        return 0.0
    with sqlite3.connect(artifacts.debuglog_path) as con:
        row = con.execute(
            "SELECT COUNT(*) FROM npc_debug_events WHERE event_type = ?",
            (event_type,),
        ).fetchone()
    return float(row[0] if row else 0)


def simulation_length(artifacts: RunArtifacts) -> float:
    start_tick = artifacts.result.get("start_tick", 0)
    end_tick = artifacts.result.get("tick")
    if not isinstance(start_tick, (int, float)):
        raise ValueError("result field 'start_tick' is not numeric")
    if not isinstance(end_tick, (int, float)):
        raise ValueError("result field 'tick' is not numeric")
    return float(end_tick) - float(start_tick)


def safe_ratio(numerator: float, denominator: float) -> float:
    if denominator == 0:
        return 0.0
    return numerator / denominator


def nth_series_dict(values: list[float]) -> dict[str, float]:
    return {str(index): value for index, value in enumerate(values, start=1)}
