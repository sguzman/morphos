# Morphos

Morphos is a Rust-native declarative 3D geometry workspace.

The repository is developed milestone by milestone. Each milestone file is durable project
state: it records scope, invariants, and completion criteria for a specific stage of work.
Agents should check boxes only after the implementation exists and the relevant verification
passes. When a blocker or follow-up is discovered during implementation, record it in the
relevant milestone instead of silently skipping it or pulling later-milestone scope forward.

Current implementation status: the repository now includes the durable `geom_workspace`
foundation from M01, the declarative `geom_scene` schema layer from M02, and the headless
`geom_geometry` evaluation layer from M03. Morphos can create, open, inspect, mutate, save, and
reopen a workspace; parse, validate, canonically serialize, and source-edit versioned TOML scene
documents; deterministically evaluate a validated `SceneDocument` into Morphos-owned mesh,
bounds, and statistics results through a backend-neutral geometry API with selective evaluation
and dependency-aware caching; launch the first desktop viewport shell through `geom_app`; and
reactively rebuild the viewport from external or in-app source edits through a watched
`source/scene.toml` loop with last-good preview preservation, stale-result suppression, and
timing instrumentation; directly author the current scene schema through a scene tree, inspector,
parameter controls, and basic structural editing while TOML remains canonical; and route current
scene mutations through a structured workspace-transaction foundation with typed operations,
atomic validation/apply behavior, actor metadata, and affected-target reporting. Undo/history,
CLI product commands, and AI integration remain future work.

## Architecture invariants

- A workspace is canonical structured state.
- TOML is the human-facing source representation, not the sole internal model.
- GUI, CLI, and AI interfaces must share the same underlying project APIs.
- Geometry backend details must remain encapsulated behind Morphos-owned abstractions.
- Headless operation is a first-class requirement.
- AI-authored mutations must eventually be structured, attributable, inspectable, and
  reversible.
- The GUI must not become a second hidden scene model.

## Repository layout

- `crates/geom_workspace`: the dedicated workspace library crate reserved for M01 ownership of
  durable project state.
- `crates/geom_scene`: the declarative scene schema crate for TOML parsing, validation,
  canonical serialization, and source-preserving edits.
- `crates/geom_geometry`: the backend-neutral geometry IR, evaluation graph, cache, and headless
  mesh-generation crate introduced in M03.
- `crates/geom_app`: the M05 desktop viewport shell built with Bevy 0.19.1 and bevy_egui 0.41.1,
  including reactive file watching and build orchestration.
- `docs/architecture.md`: the bootstrap architecture contract and development rules.
- `docs/scene-schema.md`: the M02 scene grammar, conventions, edit model, and workspace
  relationship.
- `docs/geometry-evaluation.md`: the M03 geometry IR, backend boundary, caching, and transform
  contract.
- `docs/viewport.md`: viewport launch, camera mapping, and manual viewport verification.
- `docs/reactive-editing.md`: the M05 watcher architecture, generation semantics, own-write
  suppression, timing harness, and reactive verification procedure.
- `docs/authoring-ui.md`: the M06 scene-tree, inspector, parameter, structural-editing, and
  invalid-source authoring behavior.
- `docs/workspace-transactions.md`: the M07 transaction foundation, actor model, atomic apply
  behavior, and GUI integration boundary.
- `milestones/`: canonical milestone files and milestone index.
- `.github/workflows/ci.yml`: baseline continuous integration workflow.
- `tmp/`: local reference material only; intentionally ignored and never canonical.

## Current workspace layout

`geom_workspace` currently manages this on-disk structure:

- `<workspace>/source/scene.toml`: the canonical opaque source document reserved for M02.
- `<workspace>/exports/`: reserved export/output directory.
- `<workspace>/.morphos/workspace.toml`: versioned workspace metadata including stable ID.
- `<workspace>/.morphos/cache/`: disposable generated cache data.
- `<workspace>/.morphos/history/`: reserved history path for later milestones.
- `<workspace>/.morphos/ai/`: reserved AI/session-data path for later milestones.

## Canonical commands

- Build: `cargo build --workspace`
- Test: `cargo test --workspace`
- Format: `cargo fmt --check`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Launch the viewport shell: `cargo run -p geom_app -- examples/workspaces/viewport-smoke`
- Launch the authoring smoke workspace: `cargo run -p geom_app -- examples/workspaces/authoring-smoke`
- Measure reactive timings: `cargo test -p geom_app reactive_timing_harness -- --ignored --nocapture`

The baseline lint policy for M00 is that workspace Clippy warnings are treated as errors.

If `just` is installed, the same workflows are available through `just build`, `just test`,
`just fmt`, and `just lint`. The Cargo commands above remain the canonical source of truth.

## Milestones

Start at [milestones/README.md](milestones/README.md). M00 is tracked in
[milestones/M00-project-bootstrap.md](milestones/M00-project-bootstrap.md). M02 scene examples
live under `examples/scenes/`, including the M03 benchmark/cache fixture.
