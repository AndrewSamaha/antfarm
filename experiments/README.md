# Experiments

Each experiment lives in its own folder:

- `experiments/<experiment-name>/server.yaml`
- `experiments/<experiment-name>/runs/run-<timestamp>/...`

Start an experiment with:

```bash
cargo run -p antfarm-server -- --server-config experiments/experiment-1/server.yaml
```

The server will:

- load the YAML settings
- create a new run folder under `runs/`
- copy the experiment `server.yaml` into that run folder
- write `manifest.json`
- pause the simulation when any configured stop condition is met
- write `result.json` for the run
- regenerate `results.html` at the experiment folder level using all completed `runs/*/result.json` files

Useful fields in an experiment `server.yaml`:

- `config`
  Merged into the server's existing config tree.
- `startup.paused`
- `startup.reset_world`
- `startup.load_gamestate`
- `startup.sc_commands`
- `experiment.debug_log`
  If `true`, NPC debug starts automatically for the run and writes to `runs/run-<timestamp>/data/debuglog.sqlite`.
- `experiment.terminate_server_on_completion`
  If `true`, the server exits when the experiment stop condition is met.
- `experiment.randomize_seed_from_datetime`
  If `true`, the run overrides `config.world.seed` with the current wall-clock timestamp in milliseconds. The chosen seed is written into the run manifest.
- `experiment.tick_millis`
  Overrides the server tick interval for that experiment run. Lower values run the simulation faster in wall-clock time.
- `experiment.stop_conditions`
  Supports either the legacy flat form:

```yaml
experiment:
  stop_conditions:
    max_day: 10
    all_workers_dead: true
    no_eggs: true
```

or boolean composition:

```yaml
experiment:
  stop_conditions:
    or:
      - max_day: 10
      - all_workers_dead: true
      - no_eggs: true
```

and nested combinations:

```yaml
experiment:
  stop_conditions:
    and:
      - max_tick: 2000
      - or:
          - all_workers_dead: true
          - no_eggs: true
          - max_day: 20
```
