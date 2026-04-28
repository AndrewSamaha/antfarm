from __future__ import annotations

import argparse
import json
from pathlib import Path

from .registry import run_visualizations


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--experiment-dir", required=True)
    parser.add_argument("--run-dir")
    parser.add_argument("--visualizations-json")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    experiment_dir = Path(args.experiment_dir).resolve()
    if bool(args.run_dir) == bool(args.visualizations_json):
        raise SystemExit("pass exactly one of --run-dir or --visualizations-json")
    if args.run_dir:
        run_dir = Path(args.run_dir).resolve()
        manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
        visualizations = manifest.get("visualizations", [])
    else:
        visualizations = json.loads(
            Path(args.visualizations_json).resolve().read_text(encoding="utf-8")
        )
    run_visualizations(experiment_dir=experiment_dir, visualizations=visualizations)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
