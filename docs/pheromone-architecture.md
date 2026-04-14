# Pheromone Architecture

## Intent

This document captures the intended architecture for hive-specific pheromone behavior in Antfarm.

The goal is to support ant-like emergent movement where:

- ants search for food through local sensing rather than global pathfinding
- ants can find their way back to the queen using directional pheromone information
- hives react to their own pheromones differently than to other hives
- the system scales to additional pheromone types such as threat and defense
- the server remains authoritative over world state and NPC behavior
- clients can render and inspect pheromone state without needing to simulate it authoritatively

This is not an LLM-style "AI" system. It is a local-rule, stateful simulation system driven by pheromone fields and simple ant behavior rules.

## Rationale

### Why pheromones are server-authoritative

Pheromone behavior is part of the world simulation:

- ants read nearby pheromone state
- ants modify that state
- pheromones decay over time
- multiple ants interact indirectly through shared fields
- future systems such as threat response and hive defense will depend on the same fields

Because of that, pheromones should be treated as world state, not presentation state.

If clients simulated pheromones independently:

- each client could drift from the server over time
- multiple clients viewing the same hive could disagree
- debugging would become harder because simulation depends on which clients are present
- validation would become expensive enough that the server would effectively need to re-simulate everything anyway

So the design should keep pheromone generation, decay, sensing, and ant decisions on the server.

### Why the directional signal is a gradient, not a symbolic sequence

The original idea considered directional pheromones like `ABCABCABC`, where ants infer direction by reading an ordered repeating sequence.

That is a clever idea, but it is brittle in a live simulation:

- many ants will refresh trails asynchronously
- trails will branch and overlap
- fading will remove pieces unevenly
- crossings and partial renewals will scramble the sequence

A more robust directional mechanism is a **local gradient**:

- queens emit a strong `home` signal locally
- workers reinforce `home` as they move away from the queen
- food-carrying workers follow stronger `home` back toward the queen
- workers returning from food reinforce `food` trails
- searching ants follow stronger `food` trails probabilistically

This still provides direction, but it is resilient to overlap, branching, and decay.

## High-Level Design

### Core idea

Each active world cell can carry pheromone intensity for one or more hives and one or more channels.

Initial channels:

- `home`
- `food`

Planned future channels:

- `threat`
- `defense`

The queen acts as the anchor for a hive's `home` signal.

Workers use local sensing plus probabilistic movement to:

- search for food
- return food to the queen
- reinforce useful trails

## Shared Data Model

These types should live in shared code so both server and client agree on the schema.

### Pheromone channels

```rust
enum PheromoneChannel {
    Home,
    Food,
    Threat,
    Defense,
}
```

### Ant behavior state

```rust
enum AntBehaviorState {
    Searching,
    ReturningFood,
    Defending,
    Idle,
}
```

### Per-cell hive pheromone sample

```rust
struct HivePheromone {
    hive_id: u16,
    home: u8,
    food: u8,
    threat: u8,
    defense: u8,
}
```

### Pheromone cell

```rust
struct PheromoneCell {
    // sparse: only active hive signals are stored
    entries: SmallVec<[HivePheromone; 2]>,
}
```

### Chunk pheromone storage

```rust
struct ChunkPheromones {
    width: u16,
    height: u16,
    cells: Vec<PheromoneCell>,
}
```

Notes:

- `u8` intensities are preferred over floats for memory and transfer efficiency
- sparse storage avoids a full `hive -> channels` map on every tile
- keeping top 1-2 hive entries per cell is a reasonable initial constraint

## Server Responsibilities

The server owns all authoritative pheromone behavior:

- pheromone storage
- queen emission
- worker deposition
- local sensing
- ant movement decisions
- pheromone decay
- egg spawning and hatching
- food pickup and delivery

### Queen behavior

Queens should:

- remain stationary
- emit `home` pheromone in a local radius
- consume food to produce eggs

The queen's local `home` emission is what anchors the colony.

### Worker behavior

Workers should have behavior state such as:

- `Searching`
- `ReturningFood`

Searching workers:

- sense nearby `food` pheromone for their own hive
- wander probabilistically if no strong food signal exists
- deposit `home` pheromone as they move

Returning workers:

- sense nearby `home` pheromone for their own hive
- move toward stronger `home`
- deposit `food` pheromone on the return path

### Probabilistic following

Ants should not follow pheromones perfectly.

Each move should be influenced by:

- relevant pheromone intensity
- a small amount of randomness
- heading inertia / preference to continue current motion
- terrain constraints
- optional avoidance of danger channels later

This keeps movement organic and prevents over-optimized robotic trails.

### Decay

Pheromones should fade over time unless renewed by recent activity.

Recommended model:

- decay every tick or every few ticks
- use multiplicative decay or quantized decrement
- clamp tiny values to zero
- remove empty hive entries from sparse cells

## Client Responsibilities

Clients should not authoritatively simulate pheromones.

Client responsibilities:

- render the world
- render ants and queens
- optionally render/debug pheromone overlays
- optionally inspect pheromone channels for the visible area

The client may eventually submit high-level hive directives, but not low-level pheromone simulation.

## Client/Server Data Transfer

### Normal gameplay sync

Normal world updates should continue to send:

- world tile changes
- player changes
- NPC changes
- event log changes
- config changes

Normal gameplay sync should **not** include the full pheromone map.

### Pheromone overlay/debug sync

When a player wants to inspect pheromones, the client should request data for the visible area only.

Recommended messages:

```rust
ClientMessage::SetOverlay { overlay: Option<OverlayMode> }
ClientMessage::RequestPheromoneViewport {
    viewport,
    hive_id,
    channels,
}

ServerMessage::PheromoneViewport {
    hive_id,
    channels,
    cells,
}
```

Where:

- `viewport` is the current visible world area
- `hive_id` is usually the local player's hive
- `channels` is a selected subset such as `Home` or `Food`
- `cells` is sparse and only includes non-zero entries

This keeps bandwidth bounded and avoids syncing the entire pheromone field every tick.

## Suggested Movement Model

For each worker ant:

1. inspect nearby cells
2. score candidate moves based on:
   - target pheromone strength for current state
   - random noise
   - inertia / heading bias
   - terrain passability
   - future threat avoidance
3. choose the best move probabilistically
4. deposit pheromone associated with current role/state

Examples:

- searching worker:
  - prefer stronger `food`
  - deposit `home`
- returning worker:
  - prefer stronger `home`
  - deposit `food`
- defender:
  - prefer stronger `threat`
  - reinforce `defense`

## Scalability

This design is intended to scale across:

- larger worlds
- multiple hives
- additional pheromone channels
- future debugging overlays

The main scalability techniques are:

- chunk-level storage
- sparse per-cell hive data
- compact `u8` intensities
- viewport-only client sync
- generic pheromone channels rather than hardcoded special cases

## Recommended Implementation Order

1. Introduce chunked pheromone storage on the server
2. Add per-hive `home` and `food` channels
3. Add queen-local `home` emission
4. Add worker searching/returning behavior with probabilistic gradient following
5. Add decay
6. Add optional client overlay/debug requests for visible pheromone state
7. Extend the same channel system later with `threat` and `defense`

## Summary

The intended approach is:

- server-authoritative pheromone simulation
- per-hive pheromone channels
- queen-emitted `home` signal
- worker-laid `home` and `food` trails
- probabilistic local movement using gradients
- sparse chunk-based pheromone storage
- client-side visualization and inspection only

This gives a directional, emergent ant-colony system that is robust, tunable, and extensible to future hive-defense behavior.
