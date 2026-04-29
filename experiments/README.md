# Experiments

Each experiment lives in its own folder:

- `experiments/<experiment-name>/server.yaml`
- `experiments/<experiment-name>/runs/run-<timestamp>/...`

Multi-condition experiments use:

- `experiments/<experiment-name>/server.yaml`
- `experiments/<experiment-name>/<condition-name>/runs/run-<timestamp>/...`
- `experiments/<experiment-name>/<condition-name>/results.html`
- `experiments/<experiment-name>/results.html`

Start an experiment with:

```bash
cargo run -p antfarm-server -- --server-config experiments/experiment-1/server.yaml
```

If `server.yaml` defines `experiment.conditions`, then:

- `./antfarm experiment ...` runs all conditions
- `./antfarm server ... --condition <name>` runs one condition

The server will:

- load the YAML settings
- create a new run folder under `runs/`
- copy the experiment `server.yaml` into that run folder
- write `manifest.json`
- pause the simulation when any configured stop condition is met
- write `result.json` for the run
- optionally run the shared Python analysis hook for any configured `experiment.analysis.metrics`
- run the shared Python aggregation hook to regenerate `results.html` at the experiment folder level using all completed `runs/*/result.json` files

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
- `experiment.replay.save`
  If `true`, the run writes a deterministic replay artifact to `runs/run-<timestamp>/replay/replay.json`.
- `experiment.analysis.metrics`
  Explicit metric ids to compute after run completion. The hook writes `runs/run-<timestamp>/analysis/metrics.json`, and any numeric values there are folded into `results.html`.
- `experiment.visualizations`
  Experiment-level visualization specs. The server runs the shared Python visualization hook after aggregation and writes outputs under `experiments/<name>/visualizations/`.
- `experiment.conditions`
  Optional named condition blocks. Each condition can override `config` and `startup`, can declare its own `runs` count, and may expand one config parameter over an array of values.
- `experiment.stop_conditions`
  Supports either the legacy flat form:

```yaml
experiment:
  analysis:
    metrics:
      - simulation_length
      - final_day
      - found_food
      - delivered_food_per_day
      - selected_move_count
  conditions:
    - name: baseline
      runs: 10
    - name: outward-bias-v2
      runs: 10
      config:
        colony:
          search_behavior_profile: outward-bias-v2
      startup:
        load_gamestate: "experiment 1"
        sc_commands:
          - /sc feed_queen 10
          - /sc kill @e[type=worker,hive=none]
          - /sc game unpause
    - name: plant-growth-sweep
      runs: 5
      parameter:
        path: soil.plant_growth_frequency
        values: [0.001, 0.0025, 0.005, 0.01]
      config:
        colony:
          search_behavior_profile: outward_bias_v1
  visualizations:
    - type: cumulative_record
      name: food-pickups-by-condition
      mode: staircase
      event: found_food
      x_axis: tick
      group_by: condition
      output: food-pickups-by-condition.png
      title: "Cumulative Food Pickups by Condition"
      x_label: "Tick"
      y_label: "Cumulative Food Pickups"
    - type: scatter_plot
      name: steps-to-first-food-vs-found-food
      x_metric: steps_to_nth_food.1
      y_metric: found_food
      group_by: condition
      output: steps-to-first-food-vs-found-food.png
      title: "Steps to 1st Food vs Found Food"
      x_label: "Steps to 1st Food"
      y_label: "Found Food"
```

Parametric conditions expand into concrete condition names automatically. The example above produces condition names like:

- `plant-growth-sweep__plant_growth_frequency=0.001`
- `plant-growth-sweep__plant_growth_frequency=0.0025`

`runs` still means runs per expanded value.

For `cumulative_record`, `mode` may be:

- `staircase`
  Flat between events, with vertical jumps at event ticks.
- `smooth`
  Standard line segments between event points.

For `scatter_plot`:

- `x_metric`
  Metric path for the x-axis. Nested metrics use dot notation, for example `steps_to_nth_food.1`.
- `y_metric`
  Metric path for the y-axis.
- `group_by`
  Currently supports `condition`, which colors points by experiment condition.

Built-in metric ids currently include:

- `simulation_length`
- `final_tick`
- `final_day`
- `found_food`
- `delivered_food`
- `egg_laid`
- `egg_hatched`
- `end_workers`
- `end_eggs`
- `end_queens`
- `found_food_per_day`
- `delivered_food_per_day`
- `delivery_per_found_food_ratio`
- `egg_hatch_per_laid_ratio`
- `selected_move_count`
- `placed_dirt_count`
- `memory_refresh_home_count`
- `memory_refresh_food_count`
- `debug_event_count:<event_type>`

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
