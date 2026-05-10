# Reef Mode Plan

Implement the current `reef` mode described by `config.kdl`, supporting only the
config shape that exists now:

- `mode reef`
- infinite horizontal size
- terminal-dynamic vertical size
- horizontal scroll enabled with `offscreen-pages`
- floor and surface chunk art
- creature `exit-world` plus delayed respawn
- deferred tank mode improvements

## Goals

- Load and validate `config.kdl`.
- Add a module split for config parsing, world/layer loading, app state,
  rendering, and creature behavior.
- Implement borderless reef rendering.
- Always render surface and floor.
- Preserve `b` as the water-background toggle only.
- Support horizontal scrolling by mouse horizontal scroll plus left/right arrows.
- Keep creatures inside the playable water band: below surface and above floor.
- On terminal resize, move surface/floor according to the new height and shift
  overlapping creatures back in-bounds by a random amount.
- Warn when terminal rows are less than the minimum height required by the
  tallest loaded creature plus surface/floor constraints.

## Non-Goals

- Do not support config shapes beyond the current `config.kdl`.
- Do not add tank mode improvements yet.
- Do not add CLI config overrides.
- Do not add advanced procedural terrain beyond random chunk selection from KDL.
- Do not add save state or deterministic seeds.

## Proposed Module Split

- `src/main.rs`: startup, terminal lifecycle, and top-level app wiring.
- `src/config.rs`: parse current `config.kdl` shape into typed config structs.
- `src/creature.rs`: creature definitions, variants, entity movement, and
  spawn/respawn helpers.
- `src/world.rs`: world layer chunk loading, generated horizontal span, scroll
  offset, and offscreen page math.
- `src/app.rs`: app state, input handling, and tick/update loop.
- `src/render.rs`: reef/tank rendering and size warning.

## World Semantics

- A "page" is the number of terminal columns at launch time.
- `offscreen-pages=0.5` means maintain and simulate an offscreen world extending
  50% of the launch page width to the left and 50% to the right of the visible
  viewport.
- Creatures do not exit when they leave the visible screen.
- Creatures exit only when they cross the edge of the simulated offscreen world.
- Horizontal scroll changes the viewport offset inside/generated against that
  simulated world.
- World chunks for floor/surface are generated from KDL chunk lists using the
  configured `chunkgen=random`.

## Creature Semantics

- In reef mode, creatures use world coordinates.
- Creatures cannot overlap the surface or floor.
- Spawn and respawn choose:
  - a random edge: left or right
  - a random valid height within the water band
  - a suitable direction into or across the world
- After exit, respawn after configured `delay-ms`.
- Existing tank bouncing remains deferred/unchanged except where module
  extraction requires moving code.

## Resize Behavior

- Recompute reef viewport height from terminal rows.
- Reposition surface and floor to fit the new terminal height.
- If any creature overlaps or exceeds the valid water band, shift it in-bounds by
  a random valid amount.
- If the terminal is too short for the tallest creature, render a size warning
  instead of the reef.

## Input

- `q` / `Esc`: quit.
- `b`: toggle water background.
- left arrow: scroll viewport left.
- right arrow: scroll viewport right.
- mouse horizontal scroll: scroll viewport horizontally using crossterm mouse
  capture.

Arrow keys should move by a small fixed number of columns. Start with 4 columns
unless implementation/testing shows that another value feels better. Mouse
horizontal scroll should use the event amount if crossterm exposes one cleanly;
otherwise use the same 4-column step.

## Implementation Outline

1. Extract the existing creature loading, variant selection, entity movement, and
   tests into `src/creature.rs` without changing behavior.
2. Add `src/config.rs` and parse the current `config.kdl` into typed structs,
   including mode, reef horizontal/vertical settings, floor/surface layer files,
   creature edge behavior, and respawn delay.
3. Add `src/world.rs` to load floor/surface chunk files and maintain launch-page
   width, offscreen extent, viewport offset, and generated chunks.
4. Add `src/app.rs` for the main update loop and input handling, including
   crossterm mouse capture.
5. Add `src/render.rs` for reef rendering, water background rendering, floor and
   surface rendering, creature projection from world coordinates to viewport
   coordinates, and size warning rendering.
6. Wire `main.rs` to load config, load creatures, construct app state for the
   selected mode, run the terminal app, and restore terminal/mouse state on exit.
7. Keep tank mode available with current behavior, but avoid improving or
   reshaping it beyond what the module split requires.

## Validation Plan

- Add tests for `config.kdl` parsing.
- Add tests for world chunk loading.
- Add tests for minimum-height calculation from the tallest creature.
- Add tests for reef bounds and spawn placement.
- Add tests for exit-at-offscreen-edge behavior.
- Add tests for resize rebinding where creatures are shifted in-bounds.
- Run `cargo test`.

## Risks and Mitigations

- KDL schema ambiguity: keep the parser intentionally scoped to the existing
  config shape and fail with clear errors for missing required reef fields.
- Infinite scrolling complexity: keep world coordinates separate from viewport
  coordinates and centralize scroll/offscreen math in `world.rs`.
- Resize edge cases: make water-band calculations testable without terminal I/O.
- Mouse capture cleanup: ensure terminal restore disables mouse capture even when
  the app returns an error.
