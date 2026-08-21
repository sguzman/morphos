# M00 — Project Bootstrap

## Goal

Establish a healthy, minimal Rust repository that is ready for milestone-driven development.

A completed milestone means Morphos can be cloned, built, tested, linted, and extended by a coding agent without requiring product functionality or undocumented setup knowledge.

## Scope

Owns initial repository constitution, Cargo workspace setup, baseline tooling, milestone placement, and minimal development documentation.

Does **not** implement workspace behavior, geometry, GUI, CLI product features, or AI integration from later milestones.

## Subgoals

### Repository initialization

- [x] Initialize the Git repository if one does not already exist.
- [x] Create the root Cargo workspace.
- [x] Add a root `Cargo.toml` with workspace-level package/dependency configuration.
- [x] Add a `.gitignore` appropriate for Rust, Bevy, generated exports, caches, and local editor files.
- [x] Add or document the supported Rust toolchain using `rust-toolchain.toml` or equivalent.
- [x] Confirm a clean checkout can resolve dependencies without relying on machine-specific paths.

### Initial crate structure

- [x] Create only the crates immediately justified by M00/M01 rather than scaffolding every future subsystem.
- [x] Create the initial `geom_workspace` library crate expected by M01.
- [x] Create a minimal application or placeholder binary crate only if needed to verify workspace integration.
- [x] Keep crate names and paths consistent with the milestone architecture.
- [x] Avoid introducing GUI, geometry-backend, or AI dependencies before a milestone requires them.
- [x] Ensure all initial crates participate in the root Cargo workspace.

### Baseline code quality

- [x] Add project formatting configuration if defaults are insufficient.
- [x] Establish a baseline Clippy policy.
- [x] Ensure `cargo fmt --check` passes.
- [x] Ensure `cargo clippy --workspace --all-targets` passes under the chosen warning policy.
- [x] Ensure `cargo test --workspace` passes.
- [x] Ensure `cargo build --workspace` passes.

### Development commands

- [x] Document the canonical build command.
- [x] Document the canonical test command.
- [x] Document the canonical formatting command.
- [x] Document the canonical lint command.
- [x] Add a small task runner/justfile/makefile only if it meaningfully reduces command friction.
- [x] Keep all canonical workflows usable without the task runner.

### Milestone system

- [x] Add the milestone files to the repository under a stable path such as `milestones/`.
- [x] Add `M00-project-bootstrap.md` to the milestone index.
- [x] Document that milestone files are durable project state rather than disposable planning notes.
- [x] Document that agents may check boxes only after implementation and verification.
- [x] Document that new blockers/follow-ups discovered during implementation should be recorded in the relevant milestone rather than silently ignored.
- [x] Document that later-milestone scope should not be pulled forward without a concrete dependency reason.

### Architecture invariants

- [x] Add a short architecture document or root README section recording the project's core invariants.
- [x] Record that a workspace is canonical structured state.
- [x] Record that TOML is the human-facing source representation rather than the sole internal model.
- [x] Record that GUI, CLI, and AI should share the same underlying project APIs.
- [x] Record that geometry backend types must remain encapsulated behind project-owned interfaces.
- [x] Record that headless operation is a first-class requirement.
- [x] Record that AI-authored mutations must eventually be structured, attributable, inspectable, and reversible.
- [x] Record that the GUI must not become a second hidden scene model.

### Minimal documentation

- [x] Add a root README describing Morphos in one concise project statement.
- [x] Explain the milestone-driven development workflow.
- [x] Explain the current repository/crate layout.
- [x] Explain how to run the baseline build/test/lint commands.
- [x] Link to the milestone index.
- [x] Clearly distinguish implemented functionality from planned functionality.

### Continuous integration

- [x] Add a minimal CI workflow for the primary supported development platform/toolchain.
- [x] Run formatting checks in CI.
- [x] Run Clippy/lint checks in CI.
- [x] Run workspace tests in CI.
- [x] Run a workspace build in CI if not already covered sufficiently by the previous steps.
- [x] Keep CI intentionally small; defer release packaging and platform-matrix hardening to M14.

### Bootstrap tests and verification

- [x] Verify the repository builds from a clean working tree.
- [x] Verify the test suite passes from the repository root.
- [x] Verify formatting and lint commands pass from the repository root.
- [x] Verify no generated build/cache artifacts are accidentally tracked.
- [x] Verify milestone/document links in the root README resolve correctly.
- [x] Review the dependency tree and remove dependencies that are not yet justified by M00/M01.

## Completion criteria

- A fresh clone can run the documented build, test, format, and lint workflows successfully.
- The repository contains the milestone system and enough architecture documentation for a coding agent to understand the development rules.
- The Cargo workspace contains only immediately justified scaffolding.
- No geometry, viewport, AI, or other later-milestone product functionality is required.
- The repository is ready for an agent to begin **M01 — Workspace Core** without first needing to invent project structure or setup conventions.

## Stop condition for implementation agents

When every required M00 checkbox is complete and the completion criteria are satisfied, stop.

Do **not** begin implementing M01 functionality merely because time or context remains. M01 is a separate sovereign goal target.

