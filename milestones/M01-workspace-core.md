# M01 — Workspace Core

## Goal

Create the durable project/workspace layer that every other subsystem can depend on.

A completed milestone means Morphos can create, open, inspect, save, and safely manage a workspace directory without requiring geometry, GUI, or AI code.

## Scope

Owns workspace identity, paths, metadata, persistence boundaries, and change events.

Does **not** own the scene language itself.

## Subgoals

### Workspace crate and model

- [x] Create a dedicated `geom_workspace` crate with no GUI dependency.
- [x] Define a `Workspace` type representing an opened project.
- [x] Define stable workspace-relative path helpers for source, exports, cache, history, and AI/session data.
- [x] Define workspace metadata including format version, workspace name, and optional description.
- [x] Make workspace IDs stable across reopen operations.
- [x] Add a lightweight `WorkspaceSummary` suitable for CLI/UI/AI consumers.

### Workspace lifecycle

- [x] Implement `create_workspace(path, options)` with a minimal valid directory layout.
- [x] Implement `open_workspace(path)` with useful typed errors.
- [x] Implement explicit save/flush behavior.
- [x] Detect unsupported workspace format versions and return a clear compatibility error.
- [x] Ensure workspace creation never overwrites an existing non-empty directory without an explicit option.

### Change model

- [x] Define project-owned workspace change events independent of Bevy/UI event types.
- [x] Emit changes for source reload, metadata change, scene replacement, and dirty/clean transitions.
- [x] Track whether in-memory state differs from persisted state.
- [x] Expose a monotonically increasing workspace revision number.
- [x] Add a way for consumers to query which logical workspace resources changed since a prior revision.

### Filesystem safety

- [x] Use atomic/temp-file replacement for canonical workspace writes where practical.
- [x] Add recovery behavior for interrupted writes.
- [x] Keep generated cache data clearly disposable and separate from source files.
- [x] Normalize workspace-relative paths and reject traversal outside the workspace root.

### Tests

- [x] Add create/open/save/reopen round-trip tests.
- [x] Add tests for format-version rejection.
- [x] Add tests proving cache deletion does not invalidate source state.
- [x] Add tests for dirty/clean transitions and revision increments.
- [x] Add tests for path traversal rejection.

## Completion criteria

- A test can create a workspace, mutate metadata/state, save it, reopen it, and obtain equivalent durable state.
- No Bevy, egui, geometry backend, or AI dependency is required.
- All other crates can treat `geom_workspace` as the canonical owner of project state.

