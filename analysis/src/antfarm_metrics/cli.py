from __future__ import annotations

import argparse
import json
from pathlib import Path

from .metrics import compute_metrics


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--experiment-dir", required=True)
    parser.add_argument("--run-dir", required=True)
    parser.add_argument("--metric", action="append", default=[])
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    experiment_dir = Path(args.experiment_dir).resolve()
    run_dir = Path(args.run_dir).resolve()
    metrics = compute_metrics(
        experiment_dir=experiment_dir,
        run_dir=run_dir,
        metric_names=list(args.metric),
    )

    output_dir = run_dir / "analysis"
    output_dir.mkdir(parents=True, exist_ok=True)
    payload = {
        "experiment_dir": str(experiment_dir),
        "run_dir": str(run_dir),
        "metrics": metrics,
    }
    (output_dir / "metrics.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
