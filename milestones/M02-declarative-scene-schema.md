# M02 — Declarative Scene Schema

## Goal

Define the human-facing declarative language for scenes and make TOML a pleasant, durable authoring format.

A completed milestone means a user can author a meaningful scene in TOML, parse it into typed project-owned data, modify it, and write it back without losing the structure that matters to a human editor.

## Scope

Owns syntax, identifiers, serialization, source locations, and schema versioning.

Does **not** own mesh generation.

## Subgoals

### Scene schema

- [x] Define stable `NodeId`, `ParamId`, and resource-reference types.
- [x] Define a scene document with named nodes and a clear root/output concept.
- [x] Define primitive declarations for at least box, sphere, cylinder, capsule, and plane/profile placeholders.
- [x] Define transform declarations: translation, rotation, scale.
- [x] Define composition declarations: union, difference, intersection.
- [x] Define reusable named parameters with typed values.
- [x] Define references so nodes can depend on other named nodes without textual duplication.
- [x] Decide and document units, axis convention, handedness, and rotation representation.

### TOML representation

- [x] Implement parsing with useful source spans/locations for errors.
- [x] Implement serialization back to TOML.
- [x] Preserve comments/order/formatting where practical using a TOML editing representation rather than naive full rewrites.
- [x] Ensure GUI/AI edits can update individual values without gratuitously rewriting unrelated source text.
- [x] Add canonical formatting for newly generated sections/nodes.
- [x] Define behavior for temporarily invalid TOML while the user is typing.

### Schema evolution

- [x] Add an explicit scene/schema version.
- [x] Create a migration interface even if only one version exists initially.
- [x] Reject unknown required fields with actionable diagnostics.
- [x] Preserve unknown optional/extensible metadata where practical.

### Examples

- [x] Add a minimal primitive example.
- [x] Add a boolean CSG example.
- [x] Add a parameterized object example.
- [x] Add a small hierarchical character/prop example.
- [x] Document the exact relationship between TOML source and internal typed scene data.

### Tests

- [x] Add parse/serialize/parse equivalence tests.
- [x] Add comment/order preservation tests for targeted value edits.
- [x] Add duplicate-ID and broken-reference tests.
- [x] Add invalid-number/transform tests with source locations.
- [x] Add schema-version tests.

## Completion criteria

- A human can create useful geometry descriptions without touching Rust.
- A tool can make a targeted scene edit and persist it without destroying unrelated human formatting.
- Scene parsing yields project-owned typed data that geometry/UI/AI code can consume without knowing TOML internals.

