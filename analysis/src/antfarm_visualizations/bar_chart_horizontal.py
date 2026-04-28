from __future__ import annotations

import json
import statistics
from pathlib import Path
from typing import Any

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


def render_bar_chart_horizontal(*, experiment_dir: Path, spec: dict[str, Any]) -> None:
    metric = str(spec.get("metric") or "")
    aggregate = str(spec.get("aggregate") or "median")
    group_by = str(spec.get("group_by") or "condition")
    if not metric:
        raise ValueError("bar_chart_horizontal requires metric")
    if aggregate != "median":
        raise ValueError(f"unsupported bar_chart_horizontal aggregate: {aggregate}")
    if group_by != "condition":
        raise ValueError(f"unsupported bar_chart_horizontal group_by: {group_by}")

    output_name = str(spec.get("output") or f"{spec.get('name', 'bar-chart-horizontal')}.png")
    title = str(spec.get("title") or "Horizontal Bar Chart")
    x_label = str(spec.get("x_label") or metric)
    y_label = str(spec.get("y_label") or "Condition")

    values_by_condition = collect_condition_metric_values(experiment_dir, metric)
    aggregated: list[tuple[str, float]] = []
    for condition_name, values in values_by_condition.items():
        if not values:
            continue
        aggregated.append((condition_name, float(statistics.median(values))))
    if not aggregated:
        return

    aggregated.sort(key=lambda item: item[1])

    output_dir = experiment_dir / "visualizations"
    output_dir.mkdir(parents=True, exist_ok=True)

    fig_height = max(4.5, 1.0 + (0.7 * len(aggregated)))
    fig, ax = plt.subplots(figsize=(9, fig_height))
    color_map = plt.get_cmap("tab10")

    labels = [condition_name for condition_name, _ in aggregated]
    values = [value for _, value in aggregated]
    colors = [color_map(index % 10) for index in range(len(aggregated))]

    bars = ax.barh(labels, values, color=colors, alpha=0.85)
    for bar, value in zip(bars, values, strict=True):
        ax.text(
            bar.get_width(),
            bar.get_y() + (bar.get_height() / 2),
            f" {value:.1f}",
            va="center",
            ha="left",
        )

    ax.set_title(title)
    ax.set_xlabel(x_label)
    ax.set_ylabel(y_label)
    ax.grid(True, axis="x", alpha=0.25)
    fig.tight_layout()
    fig.savefig(output_dir / output_name, dpi=180)
    plt.close(fig)


def collect_condition_metric_values(
    experiment_dir: Path, metric: str
) -> dict[str, list[float]]:
    condition_dirs = [
        child
        for child in experiment_dir.iterdir()
        if child.is_dir() and child.name != "runs" and (child / "runs").exists()
    ]
    if not condition_dirs:
        runs_dir = experiment_dir / "runs"
        if not runs_dir.exists():
            return {}
        return {"default": collect_run_metric_values(runs_dir, metric)}

    return {
        condition_dir.name: collect_run_metric_values(condition_dir / "runs", metric)
        for condition_dir in sorted(condition_dirs)
    }


def collect_run_metric_values(runs_dir: Path, metric: str) -> list[float]:
    values: list[float] = []
    for run_dir in sorted(runs_dir.glob("run-*")):
        analysis_path = run_dir / "analysis" / "metrics.json"
        result_path = run_dir / "result.json"
        analysis = (
            json.loads(analysis_path.read_text(encoding="utf-8"))
            if analysis_path.exists()
            else {}
        )
        result = (
            json.loads(result_path.read_text(encoding="utf-8"))
            if result_path.exists()
            else {}
        )
        value = resolve_metric_value(analysis, result, metric)
        if value is not None:
            values.append(value)
    return values


def resolve_metric_value(
    analysis: dict[str, Any], result: dict[str, Any], metric_path: str
) -> float | None:
    analysis_metrics = analysis.get("metrics")
    if isinstance(analysis_metrics, dict):
        value = resolve_path(analysis_metrics, metric_path)
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return float(value)
    value = resolve_path(result, metric_path)
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return float(value)
    return None


def resolve_path(value: Any, path: str) -> Any:
    current = value
    for segment in path.split("."):
        if not segment:
            continue
        if not isinstance(current, dict):
            return None
        current = current.get(segment)
    return current
