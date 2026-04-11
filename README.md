# Antfarm

Cross-platform terminal ant colony game prototype in Rust.

## Workspace

- `crates/antfarm-core`: shared world simulation, tile rules, protocol messages
- `crates/antfarm-server`: authoritative TCP server with a ticking world state
- `crates/antfarm-tui`: terminal client built with `ratatui` and `crossterm`

## Current vertical slice

- scrollable world larger than the viewport
- one authoritative shared world process
- up to five simultaneous players
- player movement above ground and underground
- digging dirt and resources
- placing dirt back into the world
- placing stone back into the world
- stone obstacles
- deterministic layered world generation with configurable seed and max depth
- bedrock at configured max depth
- shallow food veins and deeper ore veins
- config-driven soil settling
- SQLite snapshot persistence with startup restore
- NPC ants that tunnel toward players and disturb them
- modal help overlay

## Run

In one terminal:

```bash
cargo run -p antfarm-server
```

In one or more additional terminals:

```bash
cargo run -p antfarm-tui -- scout
```

Use `h j k l` to move; filled tiles auto-dig. Use `Space d h/j/k/l` to place dirt and `Space s h/j/k/l` to place stone. Press `/` to enter a slash command like `/sc set soil.settle_frequency 0.01`, `/sc set world.max_depth -255`, `/sc show_params`, or `/sc world_reset`, `?` to toggle the help modal, and `q` to quit.

The server saves the latest world snapshots to `data/antfarm.sqlite3`, restores the newest one on startup, snapshots every `world.snapshot_interval` seconds by default, and prunes history down to the newest 10 snapshots after each save.
