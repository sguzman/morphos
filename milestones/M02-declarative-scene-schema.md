# M02 — Declarative Scene Schema

## Goal

Define the human-facing declarative language for scenes and make TOML a pleasant, durable authoring format.

A completed milestone means a user can author a meaningful scene in TOML, parse it into typed project-owned data, modify it, and write it back without losing the structure that matters to a human editor.

## Scope

Owns syntax, identifiers, serialization, source locations, and schema versioning.

Does **not** own mesh generation.

## Subgoals

### Scene schema

- [ ] Define stable `NodeId`, `ParamId`, and resource-reference types.
- [ ] Define a scene document with named nodes and a clear root/output concept.
- [ ] Define primitive declarations for at least box, sphere, cylinder, capsule, and plane/profile placeholders.
- [ ] Define transform declarations: translation, rotation, scale.
- [ ] Define composition declarations: union, difference, intersection.
- [ ] Define reusable named parameters with typed values.
- [ ] Define references so nodes can depend on other named nodes without textual duplication.
- [ ] Decide and document units, axis convention, handedness, and rotation representation.

### TOML representation

- [ ] Implement parsing with useful source spans/locations for errors.
- [ ] Implement serialization back to TOML.
- [ ] Preserve comments/order/formatting where practical using a TOML editing representation rather than naive full rewrites.
- [ ] Ensure GUI/AI edits can update individual values without gratuitously rewriting unrelated source text.
- [ ] Add canonical formatting for newly generated sections/nodes.
- [ ] Define behavior for temporarily invalid TOML while the user is typing.

### Schema evolution

- [ ] Add an explicit scene/schema version.
- [ ] Create a migration interface even if only one version exists initially.
- [ ] Reject unknown required fields with actionable diagnostics.
- [ ] Preserve unknown optional/extensible metadata where practical.

### Examples

- [ ] Add a minimal primitive example.
- [ ] Add a boolean CSG example.
- [ ] Add a parameterized object example.
- [ ] Add a small hierarchical character/prop example.
- [ ] Document the exact relationship between TOML source and internal typed scene data.

### Tests

- [ ] Add parse/serialize/parse equivalence tests.
- [ ] Add comment/order preservation tests for targeted value edits.
- [ ] Add duplicate-ID and broken-reference tests.
- [ ] Add invalid-number/transform tests with source locations.
- [ ] Add schema-version tests.

## Completion criteria

- A human can create useful geometry descriptions without touching Rust.
- A tool can make a targeted scene edit and persist it without destroying unrelated human formatting.
- Scene parsing yields project-owned typed data that geometry/UI/AI code can consume without knowing TOML internals.

