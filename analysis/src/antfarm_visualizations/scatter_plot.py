from __future__ import annotations

import json
import statistics
from pathlib import Path
from typing import Any

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


def render_scatter_plot(*, experiment_dir: Path, spec: dict[str, Any]) -> None:
    x_metric = str(spec.get("x_metric") or "")
    y_metric = str(spec.get("y_metric") or "")
    x_aggregate = str(spec.get("x_aggregate") or "")
    y_aggregate = str(spec.get("y_aggregate") or "")
    group_by = str(spec.get("group_by") or "condition")
    if not x_metric or not y_metric:
        raise ValueError("scatter_plot requires x_metric and y_metric")
    if group_by != "condition":
        raise ValueError(f"unsupported scatter_plot group_by: {group_by}")
    if x_aggregate not in {"", "median"}:
        raise ValueError(f"unsupported scatter_plot x_aggregate: {x_aggregate}")
    if y_aggregate not in {"", "median"}:
        raise ValueError(f"unsupported scatter_plot y_aggregate: {y_aggregate}")
    if bool(x_aggregate) != bool(y_aggregate):
        raise ValueError("scatter_plot x_aggregate and y_aggregate must either both be set or both be empty")

    output_name = str(spec.get("output") or f"{spec.get('name', 'scatter-plot')}.png")
    title = str(spec.get("title") or "Scatter Plot")
    x_label = str(spec.get("x_label") or x_metric)
    y_label = str(spec.get("y_label") or y_metric)

    points_by_condition = collect_condition_points(
        experiment_dir, x_metric, y_metric, x_aggregate, y_aggregate
    )
    if not points_by_condition:
        return

    output_dir = experiment_dir / "visualizations"
    output_dir.mkdir(parents=True, exist_ok=True)

    fig, ax = plt.subplots(figsize=(8, 8))
    color_map = plt.get_cmap("tab10")

    for color_index, condition_name in enumerate(sorted(points_by_condition)):
        color = color_map(color_index % 10)
        points = points_by_condition[condition_name]
        if not points:
            continue
        xs = [point[0] for point in points]
        ys = [point[1] for point in points]
        ax.scatter(
            xs,
            ys,
            color=color,
            alpha=0.8,
            s=42,
            label=condition_name,
        )
        if x_aggregate == "median" and y_aggregate == "median" and len(points) == 1:
            ax.annotate(
                condition_name,
                (xs[0], ys[0]),
                xytext=(6, 4),
                textcoords="offset points",
                fontsize=9,
            )

    ax.set_title(title)
    ax.set_xlabel(x_label)
    ax.set_ylabel(y_label)
    ax.set_xlim(left=0)
    ax.set_ylim(bottom=0)
    ax.grid(True, alpha=0.25)
    ax.legend()
    fig.tight_layout()
    fig.savefig(output_dir / output_name, dpi=180)
    plt.close(fig)


def collect_condition_points(
    experiment_dir: Path, x_metric: str, y_metric: str, x_aggregate: str = "", y_aggregate: str = ""
) -> dict[str, list[tuple[float, float]]]:
    condition_dirs = [
        child
        for child in experiment_dir.iterdir()
        if child.is_dir() and child.name != "runs" and (child / "runs").exists()
    ]
    if not condition_dirs:
        runs_dir = experiment_dir / "runs"
        if not runs_dir.exists():
            return {}
        points = collect_run_points(runs_dir, x_metric, y_metric)
        return {"default": aggregate_points(points, x_aggregate, y_aggregate)}

    return {
        condition_dir.name: aggregate_points(
            collect_run_points(condition_dir / "runs", x_metric, y_metric),
            x_aggregate,
            y_aggregate,
        )
        for condition_dir in sorted(condition_dirs)
    }


def aggregate_points(
    points: list[tuple[float, float]], x_aggregate: str, y_aggregate: str
) -> list[tuple[float, float]]:
    if not points:
        return []
    if not x_aggregate and not y_aggregate:
        return points
    if x_aggregate == "median" and y_aggregate == "median":
        xs = [point[0] for point in points]
        ys = [point[1] for point in points]
        return [(float(statistics.median(xs)), float(statistics.median(ys)))]
    raise ValueError(
        f"unsupported scatter_plot aggregation combination: x={x_aggregate}, y={y_aggregate}"
    )


def collect_run_points(
    runs_dir: Path, x_metric: str, y_metric: str
) -> list[tuple[float, float]]:
    points: list[tuple[float, float]] = []
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
        x_value = resolve_metric_value(analysis, result, x_metric)
        y_value = resolve_metric_value(analysis, result, y_metric)
        if x_value is None or y_value is None:
            continue
        points.append((x_value, y_value))
    return points


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
