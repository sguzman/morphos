# M14 — Integration, Packaging & Release

## Goal

Turn the accumulated subsystems into a coherent tool that can be cloned, built, tested, launched, and used repeatedly without tribal knowledge.

A completed milestone means a clean machine can build Morphos, run its test suite, open bundled example workspaces, use the CLI, and exercise the core AI-edit interfaces.

## Scope

Owns repository ergonomics, integration tests, packaging, examples, compatibility policy, and initial release discipline.

## Subgoals

### Repository and build discipline

- [ ] Ensure crate boundaries match the architectural responsibilities established by prior milestones.
- [ ] Add top-level build/test/lint commands.
- [ ] Add formatting/lint configuration.
- [ ] Add CI for supported platforms/toolchains.
- [ ] Pin or document important native geometry dependencies.
- [ ] Document development feature flags.

### End-to-end tests

- [ ] Test TOML edit → workspace revision → geometry rebuild.
- [ ] Test GUI-style structured edit → persisted TOML.
- [ ] Test CLI export from bundled workspace.
- [ ] Test invalid edit → diagnostic → last-good geometry preservation.
- [ ] Test AI proposal → approval → apply → history → revert.
- [ ] Test workspace reopen after AI/user history exists.
- [ ] Test schema/workspace version rejection/migration path.

### Example workspaces

- [ ] Ship a primitive starter workspace.
- [ ] Ship a boolean hard-surface workspace.
- [ ] Ship a parameterized expressive character/creature workspace.
- [ ] Ship an architecture/prop workspace.
- [ ] Ship an AI-edit demo workspace with documented example operations.
- [ ] Ensure every example validates and exports headlessly in CI.

### Packaging

- [ ] Produce a desktop binary artifact.
- [ ] Produce a CLI binary artifact.
- [ ] Define default workspace creation/template behavior.
- [ ] Ensure app can locate bundled assets/examples without source-tree assumptions.
- [ ] Add version display to GUI and CLI.

### Compatibility and release policy

- [ ] Define semantic/version policy for workspace format.
- [ ] Define semantic/version policy for AI/tool protocol.
- [ ] Document migration expectations.
- [ ] Add changelog/release notes structure.
- [ ] Create an initial tagged release once completion criteria are satisfied.

### User-facing project documentation

- [ ] Document "create/open/edit/export" basic workflow.
- [ ] Document TOML-first workflow.
- [ ] Document GUI-first workflow.
- [ ] Document headless/automation workflow.
- [ ] Document AI proposal/live-edit workflow and recovery model.
- [ ] Document known geometry/backend limitations.

## Completion criteria

- The project is usable without the original author manually explaining setup.
- Core workflows are protected by integration tests.
- Example workspaces demonstrate the intended declarative, live, headless, and AI-first experience.

