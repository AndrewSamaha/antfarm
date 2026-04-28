from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any


def load_step_series_for_run(run_dir: Path) -> tuple[list[float], list[float]]:
    debuglog_path = run_dir / "data" / "debuglog.sqlite"
    if not debuglog_path.exists():
        return [], []

    with sqlite3.connect(debuglog_path) as con:
        rows = con.execute(
            """
            SELECT id, tick, npc_id, event_type
            FROM npc_debug_events
            WHERE event_type IN ('selected_move', 'found_food', 'delivered_food', 'egg_hatched')
            ORDER BY tick ASC, id ASC
            """
        ).fetchall()

    states: dict[int, dict[str, Any]] = {}
    food_series_by_n: list[list[int]] = []
    queen_series_by_n: list[list[int]] = []

    for _, _, npc_id, event_type in rows:
        state = states.setdefault(
            int(npc_id),
            {
                "search_steps": 0,
                "return_steps": None,
                "food_index": 0,
                "queen_index": 0,
            },
        )

        if event_type == "egg_hatched":
            state["search_steps"] = 0
            state["return_steps"] = None
            state["food_index"] = 0
            state["queen_index"] = 0
            continue

        if event_type == "selected_move":
            if state["return_steps"] is None:
                state["search_steps"] += 1
            else:
                state["return_steps"] += 1
            continue

        if event_type == "found_food":
            append_series_value(food_series_by_n, int(state["food_index"]), int(state["search_steps"]))
            state["food_index"] += 1
            state["search_steps"] = 0
            state["return_steps"] = 0
            continue

        if event_type == "delivered_food":
            if state["return_steps"] is not None:
                append_series_value(queen_series_by_n, int(state["queen_index"]), int(state["return_steps"]))
                state["queen_index"] += 1
            state["search_steps"] = 0
            state["return_steps"] = None

    return average_series(food_series_by_n), average_series(queen_series_by_n)


def append_series_value(series_by_n: list[list[int]], index: int, value: int) -> None:
    while len(series_by_n) <= index:
        series_by_n.append([])
    series_by_n[index].append(value)


def average_series(series_by_n: list[list[int]]) -> list[float]:
    averages: list[float] = []
    for values in series_by_n:
        if not values:
            averages.append(0.0)
        else:
            averages.append(sum(values) / len(values))
    return averages
