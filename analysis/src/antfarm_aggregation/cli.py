from __future__ import annotations

import argparse
import json
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from statistics import median
from typing import Any

from antfarm_metrics.steps import load_step_series_for_run


@dataclass
class MetricSamples:
    values: list[float]
    labels: list[str]


@dataclass
class RunSummary:
    run_name: str
    result: dict[str, Any]
    steps_to_nth_food: list[float]
    steps_to_nth_queen: list[float]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--experiment-dir", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    experiment_dir = Path(args.experiment_dir).resolve()
    write_results_html(experiment_dir)
    return 0


def write_results_html(experiment_dir: Path) -> None:
    if has_condition_dirs(experiment_dir):
        write_condition_comparison_html(experiment_dir)
        return

    runs_dir = experiment_dir / "runs"
    run_count = 0
    stop_reason_counts: Counter[str] = Counter()
    metrics: dict[str, MetricSamples] = {}
    run_summaries: list[RunSummary] = []

    if runs_dir.exists():
        for run_dir in sorted(runs_dir.glob("run-*")):
            result_path = run_dir / "result.json"
            if not result_path.exists():
                continue
            result = json.loads(result_path.read_text(encoding="utf-8"))
            run_count += 1
            stop_reason_counts[stop_reason_label(result.get("stop_reason"))] += 1
            collect_numeric_metrics(None, result, metrics)

            analysis_metrics_path = run_dir / "analysis" / "metrics.json"
            if analysis_metrics_path.exists():
                analysis = json.loads(analysis_metrics_path.read_text(encoding="utf-8"))
                metric_values = analysis.get("metrics")
                if isinstance(metric_values, dict):
                    collect_numeric_metrics("analysis", metric_values, metrics)

            steps_to_nth_food, steps_to_nth_queen = load_step_series_for_run(run_dir)
            run_summaries.append(
                RunSummary(
                    run_name=run_dir.name,
                    result=result,
                    steps_to_nth_food=steps_to_nth_food,
                    steps_to_nth_queen=steps_to_nth_queen,
                )
            )

    html = render_results_html(
        base_dir=experiment_dir,
        experiment_name=experiment_dir.name,
        run_count=run_count,
        stop_reason_counts=stop_reason_counts,
        metrics=metrics,
        run_summaries=run_summaries,
    )
    (experiment_dir / "results.html").write_text(html, encoding="utf-8")


def has_condition_dirs(experiment_dir: Path) -> bool:
    return any(
        child.is_dir() and child.name != "runs" and (child / "runs").exists()
        for child in experiment_dir.iterdir()
    ) if experiment_dir.exists() else False


def write_condition_comparison_html(experiment_dir: Path) -> None:
    condition_dirs = sorted(
        child for child in experiment_dir.iterdir()
        if child.is_dir() and child.name != "runs" and (child / "runs").exists()
    )

    condition_rows: list[tuple[str, int]] = []
    metrics: dict[str, MetricSamples] = {}

    for condition_dir in condition_dirs:
        run_count, condition_metrics = collect_condition_metric_stats(condition_dir)
        condition_rows.append((condition_dir.name, run_count))
        for metric_name, value in condition_metrics.items():
            sample = metrics.setdefault(metric_name, MetricSamples(values=[], labels=[]))
            sample.values.append(value)
            sample.labels.append(condition_dir.name)

    title = f"Experiment Results: {experiment_dir.name}"
    condition_table_rows = "".join(
        f"<tr><td><a href=\"{html_escape(name)}/results.html\">{html_escape(name)}</a></td><td>{run_count}</td></tr>"
        for name, run_count in condition_rows
    ) or '<tr><td colspan="2">No conditions found.</td></tr>'

    metric_rows = []
    for metric_name, samples in sorted(metrics.items()):
        sample_count = len(samples.values)
        if sample_count == 0:
            continue
        metric_rows.append(
            "<tr>"
            f"<td>{html_escape(metric_name)}</td>"
            f"<td>{sample_count}</td>"
            f"<td>{format_number(min(samples.values))}</td>"
            f"<td>{format_number(max(samples.values))}</td>"
            f"<td>{format_number(sum(samples.values) / sample_count)}</td>"
            f"<td>{format_number(float(median(samples.values)))}</td>"
            f"<td>{html_escape(compute_mode([format_number(v) for v in samples.values]))}</td>"
            "</tr>"
        )
    metric_rows_html = "".join(metric_rows) or '<tr><td colspan="7">No numeric metrics found.</td></tr>'
    visualizations_html = render_visualizations_section(experiment_dir)

    html = (
        "<!doctype html>"
        "<html lang=\"en\">"
        "<head>"
        "<meta charset=\"utf-8\">"
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
        f"<title>{html_escape(title)}</title>"
        "<style>"
        "body { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; margin: 24px; color: #111; background: #faf8f2; }"
        "h1, h2 { margin: 0 0 12px; }"
        "p { margin: 0 0 16px; }"
        "table { border-collapse: collapse; width: 100%; margin: 0 0 24px; background: white; }"
        "th, td { border: 1px solid #d7d0c3; padding: 8px 10px; text-align: left; vertical-align: top; }"
        "th { background: #efe7d8; }"
        "</style>"
        "</head>"
        "<body>"
        f"<h1>{html_escape(title)}</h1>"
        "<h2>Conditions</h2>"
        "<table><thead><tr><th>Condition</th><th>Runs</th></tr></thead>"
        f"<tbody>{condition_table_rows}</tbody></table>"
        "<h2>Condition Metric Statistics</h2>"
        "<table><thead><tr><th>Metric</th><th>Conditions</th><th>Min</th><th>Max</th><th>Average</th><th>Median</th><th>Mode</th></tr></thead>"
        f"<tbody>{metric_rows_html}</tbody></table>"
        f"{visualizations_html}"
        "</body></html>"
    )
    (experiment_dir / "results.html").write_text(html, encoding="utf-8")


def collect_condition_metric_stats(condition_dir: Path) -> tuple[int, dict[str, float]]:
    metrics: dict[str, list[float]] = {}
    run_count = 0
    for run_dir in sorted((condition_dir / "runs").glob("run-*")):
        result_path = run_dir / "result.json"
        if not result_path.exists():
            continue
        run_count += 1
        result = json.loads(result_path.read_text(encoding="utf-8"))
        flat_metrics: dict[str, MetricSamples] = {}
        collect_numeric_metrics(None, result, flat_metrics)
        analysis_metrics_path = run_dir / "analysis" / "metrics.json"
        if analysis_metrics_path.exists():
            analysis = json.loads(analysis_metrics_path.read_text(encoding="utf-8"))
            metric_values = analysis.get("metrics")
            if isinstance(metric_values, dict):
                collect_numeric_metrics("analysis", metric_values, flat_metrics)
        for metric_name, samples in flat_metrics.items():
            metrics.setdefault(metric_name, []).extend(samples.values)

    return run_count, {
        metric_name: (sum(values) / len(values))
        for metric_name, values in metrics.items()
        if values
    }


def collect_numeric_metrics(prefix: str | None, value: Any, metrics: dict[str, MetricSamples]) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            next_prefix = f"{prefix}.{key}" if prefix else str(key)
            collect_numeric_metrics(next_prefix, child, metrics)
        return
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return
    if prefix is None:
        return
    sample = metrics.setdefault(prefix, MetricSamples(values=[], labels=[]))
    sample.values.append(float(value))
    sample.labels.append(str(value))


def render_results_html(
    *,
    base_dir: Path,
    experiment_name: str,
    run_count: int,
    stop_reason_counts: Counter[str],
    metrics: dict[str, MetricSamples],
    run_summaries: list[RunSummary],
) -> str:
    stop_reason_rows = "".join(
        f"<tr><td>{html_escape(reason)}</td><td>{count}</td></tr>"
        for reason, count in sorted(stop_reason_counts.items())
    ) or '<tr><td colspan="2">No completed runs yet.</td></tr>'

    metric_rows = []
    for metric_name, samples in sorted(metrics.items()):
        if not samples.values:
            continue
        sample_count = len(samples.values)
        minimum = min(samples.values)
        maximum = max(samples.values)
        average = sum(samples.values) / sample_count
        med = median(samples.values)
        mode = compute_mode(samples.labels)
        metric_rows.append(
            "<tr>"
            f"<td>{html_escape(metric_name)}</td>"
            f"<td>{sample_count}</td>"
            f"<td>{format_number(minimum)}</td>"
            f"<td>{format_number(maximum)}</td>"
            f"<td>{format_number(average)}</td>"
            f"<td>{format_number(float(med))}</td>"
            f"<td>{html_escape(mode)}</td>"
            "</tr>"
        )
    metric_rows_html = "".join(metric_rows) or '<tr><td colspan="7">No numeric metrics found.</td></tr>'
    food_steps_table = render_nth_steps_table(
        title="Steps To Nth Food",
        run_summaries=run_summaries,
        attr_name="steps_to_nth_food",
    )
    queen_steps_table = render_nth_steps_table(
        title="Steps To Nth Queen",
        run_summaries=run_summaries,
        attr_name="steps_to_nth_queen",
    )
    visualizations_html = render_visualizations_section(base_dir)

    title = f"Experiment Results: {experiment_name}"
    return (
        "<!doctype html>"
        "<html lang=\"en\">"
        "<head>"
        "<meta charset=\"utf-8\">"
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
        f"<title>{html_escape(title)}</title>"
        "<style>"
        "body { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; margin: 24px; color: #111; background: #faf8f2; }"
        "h1, h2 { margin: 0 0 12px; }"
        "p { margin: 0 0 16px; }"
        "table { border-collapse: collapse; width: 100%; margin: 0 0 24px; background: white; }"
        "th, td { border: 1px solid #d7d0c3; padding: 8px 10px; text-align: left; vertical-align: top; }"
        "th { background: #efe7d8; }"
        "</style>"
        "</head>"
        "<body>"
        f"<h1>{html_escape(title)}</h1>"
        f"<p>Samples: <strong>{run_count}</strong></p>"
        "<h2>Stop Reasons</h2>"
        "<table><thead><tr><th>Reason</th><th>Count</th></tr></thead>"
        f"<tbody>{stop_reason_rows}</tbody></table>"
        "<h2>Metric Statistics</h2>"
        "<table><thead><tr><th>Metric</th><th>Samples</th><th>Min</th><th>Max</th><th>Average</th><th>Median</th><th>Mode</th></tr></thead>"
        f"<tbody>{metric_rows_html}</tbody></table>"
        f"{food_steps_table}"
        f"{queen_steps_table}"
        f"{visualizations_html}"
        "</body></html>"
    )


def render_visualizations_section(base_dir: Path) -> str:
    visualizations_dir = base_dir / "visualizations"
    if not visualizations_dir.exists():
        return ""

    image_paths = sorted(
        path for path in visualizations_dir.iterdir()
        if path.is_file() and path.suffix.lower() in {".png", ".jpg", ".jpeg", ".svg"}
    )
    if not image_paths:
        return ""

    cards = []
    for image_path in image_paths:
        relative_path = image_path.relative_to(base_dir).as_posix()
        cards.append(
            "<figure style=\"margin: 0 0 24px;\">"
            f"<figcaption style=\"margin: 0 0 8px; font-weight: 600;\">{html_escape(image_path.name)}</figcaption>"
            f"<img src=\"{html_escape(relative_path)}\" alt=\"{html_escape(image_path.name)}\" "
            "style=\"max-width: 100%; border: 1px solid #d7d0c3; background: white; padding: 8px;\" />"
            "</figure>"
        )
    return f"<h2>Visualizations</h2>{''.join(cards)}"


def render_nth_steps_table(*, title: str, run_summaries: list[RunSummary], attr_name: str) -> str:
    sorted_runs = sorted(
        run_summaries,
        key=lambda run: (
            -int(run.result.get("delivered_food", 0) or 0),
            run.run_name,
        ),
    )
    max_n = 0
    for run in sorted_runs:
        max_n = max(max_n, len(getattr(run, attr_name)))

    if max_n == 0:
        return (
            f"<h2>{html_escape(title)}</h2>"
            "<table><thead><tr><th>Run</th></tr></thead>"
            "<tbody><tr><td>No debug data available for this table.</td></tr></tbody></table>"
        )

    header_cells = "".join(f"<th>{index}</th>" for index in range(1, max_n + 1))
    body_rows = []
    for run in sorted_runs:
        values: list[float] = getattr(run, attr_name)
        row_cells = []
        for index in range(max_n):
            if index < len(values):
                row_cells.append(f"<td>{format_number(values[index])}</td>")
            else:
                row_cells.append("<td></td>")
        body_rows.append(
            "<tr>"
            f"<td>{html_escape(run.run_name)}</td>"
            + "".join(row_cells)
            + "</tr>"
        )

    return (
        f"<h2>{html_escape(title)}</h2>"
        "<table><thead><tr><th>Run</th>"
        f"{header_cells}</tr></thead>"
        f"<tbody>{''.join(body_rows)}</tbody></table>"
    )

def compute_mode(labels: list[str]) -> str:
    counts = Counter(labels)
    if not counts:
        return "none"
    max_count = max(counts.values())
    if max_count <= 1:
        return "none"
    return ", ".join(sorted(label for label, count in counts.items() if count == max_count))


def stop_reason_label(value: Any) -> str:
    if isinstance(value, str):
        return value
    return json.dumps(value, sort_keys=True)


def format_number(value: float) -> str:
    if float(value).is_integer():
        return f"{value:.0f}"
    return f"{value:.3f}"


def html_escape(value: str) -> str:
    return (
        value.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
        .replace("'", "&#39;")
    )


if __name__ == "__main__":
    raise SystemExit(main())
