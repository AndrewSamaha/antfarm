from __future__ import annotations

from pathlib import Path
from typing import Any

from .bar_chart_horizontal import render_bar_chart_horizontal
from .cumulative_record import render_cumulative_record
from .scatter_plot import render_scatter_plot


def run_visualizations(*, experiment_dir: Path, visualizations: list[dict[str, Any]]) -> None:
    for spec in visualizations:
        type_name = str(spec.get("type_name") or spec.get("type") or "")
        if type_name == "cumulative_record":
            render_cumulative_record(experiment_dir=experiment_dir, spec=spec)
            continue
        if type_name == "scatter_plot":
            render_scatter_plot(experiment_dir=experiment_dir, spec=spec)
            continue
        if type_name == "bar_chart_horizontal":
            render_bar_chart_horizontal(experiment_dir=experiment_dir, spec=spec)
            continue
        raise ValueError(f"unknown visualization type: {type_name}")
