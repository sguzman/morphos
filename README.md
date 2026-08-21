# Morphos

Morphos is a Rust-native declarative 3D geometry workspace.

The repository is developed milestone by milestone. Each milestone file is durable project
state: it records scope, invariants, and completion criteria for a specific stage of work.
Agents should check boxes only after the implementation exists and the relevant verification
passes. When a blocker or follow-up is discovered during implementation, record it in the
relevant milestone instead of silently skipping it or pulling later-milestone scope forward.

Current implementation status: the repository now includes the durable `geom_workspace`
foundation from M01 and the declarative `geom_scene` schema layer from M02. Morphos can create,
open, inspect, mutate, save, and reopen a workspace, and it can parse, validate, canonically
serialize, and source-edit versioned TOML scene documents without geometry evaluation, GUI,
CLI product, or AI integration. Geometry evaluation, viewport behavior, and higher-level
editing systems remain future work.

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
- `docs/architecture.md`: the bootstrap architecture contract and development rules.
- `docs/scene-schema.md`: the M02 scene grammar, conventions, edit model, and workspace
  relationship.
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

The baseline lint policy for M00 is that workspace Clippy warnings are treated as errors.

If `just` is installed, the same workflows are available through `just build`, `just test`,
`just fmt`, and `just lint`. The Cargo commands above remain the canonical source of truth.

## Milestones

Start at [milestones/README.md](milestones/README.md). M00 is tracked in
[milestones/M00-project-bootstrap.md](milestones/M00-project-bootstrap.md). M02 scene examples
live under `examples/scenes/`.
