from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


def render_cumulative_record(*, experiment_dir: Path, spec: dict[str, Any]) -> None:
    event_type = str(spec.get("event") or "")
    mode = str(spec.get("mode") or "staircase")
    x_axis = str(spec.get("x_axis") or "tick")
    group_by = str(spec.get("group_by") or "condition")
    if event_type not in {"found_food", "worker_count"}:
        raise ValueError(f"unsupported cumulative_record event: {event_type}")
    if mode not in {"smooth", "staircase"}:
        raise ValueError(f"unsupported cumulative_record mode: {mode}")
    if x_axis != "tick":
        raise ValueError(f"unsupported cumulative_record x_axis: {x_axis}")
    if group_by != "condition":
        raise ValueError(f"unsupported cumulative_record group_by: {group_by}")

    output_name = str(spec.get("output") or f"{spec.get('name', 'cumulative-record')}.png")
    title = str(spec.get("title") or "Cumulative Record")
    x_label = str(spec.get("x_label") or "Tick")
    y_label = str(spec.get("y_label") or "Cumulative Count")

    series_by_condition = collect_condition_series(experiment_dir, event_type)
    if not series_by_condition:
        return

    output_dir = experiment_dir / "visualizations"
    output_dir.mkdir(parents=True, exist_ok=True)

    fig, ax = plt.subplots(figsize=(11, 6))
    color_map = plt.get_cmap("tab10")

    for color_index, condition_name in enumerate(sorted(series_by_condition)):
        color = color_map(color_index % 10)
        runs = series_by_condition[condition_name]
        for run_index, ticks in enumerate(runs):
            if event_type == "worker_count":
                xs, ys = worker_count_series(ticks)
            else:
                xs, ys = cumulative_series(ticks)
            label = condition_name if run_index == 0 else None
            if mode == "staircase":
                ax.step(
                    xs,
                    ys,
                    where="post",
                    color=color,
                    alpha=0.35,
                    linewidth=1.5,
                    label=label,
                )
            else:
                ax.plot(xs, ys, color=color, alpha=0.35, linewidth=1.5, label=label)

    ax.set_title(title)
    ax.set_xlabel(x_label)
    ax.set_ylabel(y_label)
    ax.grid(True, alpha=0.25)
    ax.legend()
    fig.tight_layout()
    fig.savefig(output_dir / output_name, dpi=180)
    plt.close(fig)


def collect_condition_series(
    experiment_dir: Path, event_type: str
) -> dict[str, list[list[int] | list[tuple[int, int]]]]:
    condition_dirs = [
        child
        for child in experiment_dir.iterdir()
        if child.is_dir() and child.name != "runs" and (child / "runs").exists()
    ]
    if not condition_dirs:
        runs_dir = experiment_dir / "runs"
        if not runs_dir.exists():
            return {}
        return {"default": collect_run_series(runs_dir, event_type)}

    return {
        condition_dir.name: collect_run_series(condition_dir / "runs", event_type)
        for condition_dir in sorted(condition_dirs)
    }


def collect_run_series(runs_dir: Path, event_type: str) -> list[list[int] | list[tuple[int, int]]]:
    series: list[list[int] | list[tuple[int, int]]] = []
    for run_dir in sorted(runs_dir.glob("run-*")):
        debuglog_path = run_dir / "data" / "debuglog.sqlite"
        if not debuglog_path.exists():
            continue
        with sqlite3.connect(debuglog_path) as con:
            if event_type == "worker_count":
                rows = con.execute(
                    """
                    SELECT tick, event_type
                    FROM npc_debug_events
                    WHERE event_type IN ('egg_hatched', 'died_of_old_age')
                    ORDER BY tick ASC, id ASC
                    """
                ).fetchall()
                series.append([(int(row[0]), worker_count_delta(str(row[1]))) for row in rows])
            else:
                rows = con.execute(
                    "SELECT tick FROM npc_debug_events WHERE event_type = ? ORDER BY tick ASC, id ASC",
                    (event_type,),
                ).fetchall()
                series.append([int(row[0]) for row in rows])
    return series


def cumulative_series(ticks: list[int]) -> tuple[list[int], list[int]]:
    xs = [0]
    ys = [0]
    count = 0
    for tick in ticks:
        count += 1
        xs.append(tick)
        ys.append(count)
    return xs, ys


def worker_count_series(changes: list[tuple[int, int]]) -> tuple[list[int], list[int]]:
    xs = [0]
    ys = [0]
    count = 0
    for tick, delta in changes:
        count = max(0, count + delta)
        xs.append(tick)
        ys.append(count)
    return xs, ys


def worker_count_delta(event_type: str) -> int:
    if event_type == "egg_hatched":
        return 1
    if event_type == "died_of_old_age":
        return -1
    raise ValueError(f"unsupported worker_count event type: {event_type}")
