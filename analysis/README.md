# Antfarm Metrics

Shared Python analysis code for experiment post-run hooks.

The server invokes this package with:

```bash
uv run --project analysis python -m antfarm_metrics.cli ...
uv run --project analysis python -m antfarm_aggregation.cli ...
```

Per-run outputs are written under:

- `experiments/<name>/runs/run-<timestamp>/analysis/metrics.json`
- `experiments/<name>/runs/run-<timestamp>/analysis/stdout.log`
- `experiments/<name>/runs/run-<timestamp>/analysis/stderr.log`

Experiment-level aggregation is written to:

- `experiments/<name>/results.html`
