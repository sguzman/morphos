# M01 — Workspace Core

## Goal

Create the durable project/workspace layer that every other subsystem can depend on.

A completed milestone means Morphos can create, open, inspect, save, and safely manage a workspace directory without requiring geometry, GUI, or AI code.

## Scope

Owns workspace identity, paths, metadata, persistence boundaries, and change events.

Does **not** own the scene language itself.

## Subgoals

### Workspace crate and model

- [ ] Create a dedicated `geom_workspace` crate with no GUI dependency.
- [ ] Define a `Workspace` type representing an opened project.
- [ ] Define stable workspace-relative path helpers for source, exports, cache, history, and AI/session data.
- [ ] Define workspace metadata including format version, workspace name, and optional description.
- [ ] Make workspace IDs stable across reopen operations.
- [ ] Add a lightweight `WorkspaceSummary` suitable for CLI/UI/AI consumers.

### Workspace lifecycle

- [ ] Implement `create_workspace(path, options)` with a minimal valid directory layout.
- [ ] Implement `open_workspace(path)` with useful typed errors.
- [ ] Implement explicit save/flush behavior.
- [ ] Detect unsupported workspace format versions and return a clear compatibility error.
- [ ] Ensure workspace creation never overwrites an existing non-empty directory without an explicit option.

### Change model

- [ ] Define project-owned workspace change events independent of Bevy/UI event types.
- [ ] Emit changes for source reload, metadata change, scene replacement, and dirty/clean transitions.
- [ ] Track whether in-memory state differs from persisted state.
- [ ] Expose a monotonically increasing workspace revision number.
- [ ] Add a way for consumers to query which logical workspace resources changed since a prior revision.

### Filesystem safety

- [ ] Use atomic/temp-file replacement for canonical workspace writes where practical.
- [ ] Add recovery behavior for interrupted writes.
- [ ] Keep generated cache data clearly disposable and separate from source files.
- [ ] Normalize workspace-relative paths and reject traversal outside the workspace root.

### Tests

- [ ] Add create/open/save/reopen round-trip tests.
- [ ] Add tests for format-version rejection.
- [ ] Add tests proving cache deletion does not invalidate source state.
- [ ] Add tests for dirty/clean transitions and revision increments.
- [ ] Add tests for path traversal rejection.

## Completion criteria

- A test can create a workspace, mutate metadata/state, save it, reopen it, and obtain equivalent durable state.
- No Bevy, egui, geometry backend, or AI dependency is required.
- All other crates can treat `geom_workspace` as the canonical owner of project state.

