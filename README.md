# Morphos

Morphos is a Rust-native declarative 3D geometry workspace.

The repository is developed milestone by milestone. Each milestone file is durable project
state: it records scope, invariants, and completion criteria for a specific stage of work.
Agents should check boxes only after the implementation exists and the relevant verification
passes. When a blocker or follow-up is discovered during implementation, record it in the
relevant milestone instead of silently skipping it or pulling later-milestone scope forward.

Current implementation status: this repository contains bootstrap scaffolding only. It does
not yet implement workspace persistence, scene parsing, geometry evaluation, GUI behavior, or
AI editing workflows.

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
- `docs/architecture.md`: the bootstrap architecture contract and development rules.
- `milestones/`: canonical milestone files and milestone index.
- `.github/workflows/ci.yml`: baseline continuous integration workflow.
- `tmp/`: local reference material only; intentionally ignored and never canonical.

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
[milestones/M00-project-bootstrap.md](milestones/M00-project-bootstrap.md).
