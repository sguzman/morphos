# M06 — Scene Tree & Inspector

## Goal

Make the declarative scene directly manipulable from the GUI without abandoning the underlying TOML model.

A completed milestone means a user can browse scene structure, select nodes, edit common properties, and have those edits persist to source and appear immediately in the viewport.

## Scope

Owns scene navigation, selection, property editing, and generated parameter controls.

## Subgoals

### Scene tree

- [x] Display named nodes in a hierarchical/dependency-aware tree.
- [x] Support node selection.
- [x] Preserve selection when unrelated scene revisions occur.
- [x] Add visibility/preview toggles without changing canonical geometry semantics.
- [x] Add search/filter by node name or ID.
- [x] Add frame-selected integration with the viewport.

### Inspector

- [x] Display selected node identity/type.
- [x] Edit translation, rotation, and scale.
- [x] Edit primitive dimensions.
- [x] Edit boolean/composition-specific properties.
- [x] Edit references through safe selectors rather than raw IDs where practical.
- [x] Validate inspector edits before committing them to workspace state.
- [x] Display source location and a "reveal in source" hook/API.

### Parameter UI

- [x] Generate controls for declared scalar parameters.
- [x] Support min/max/step metadata where provided.
- [x] Add boolean and enum-style parameter controls where schema supports them.
- [x] Make parameter changes flow through the same mutation/revision system as other edits.
- [x] Display which nodes depend on a selected parameter.

### Creation/deletion basics

- [x] Add a primitive node from the GUI.
- [x] Rename a node with reference-safe behavior.
- [x] Duplicate a node.
- [x] Delete a node with dependency warnings.
- [x] Reparent/restructure where the scene model permits it.

### Tests

- [x] Add selection persistence tests.
- [x] Add inspector → workspace → TOML persistence tests.
- [x] Add rename/reference integrity tests.
- [x] Add delete-with-dependents behavior tests.

## Completion criteria

- Common geometry authoring can happen either in TOML or through the GUI.
- Both routes produce equivalent workspace revisions and persisted source.
- The GUI never becomes a second hidden scene model.

## Notes

- M06 consumes existing scalar-parameter extension metadata where present. The current schema does
  not define richer boolean or enum parameter types yet, so that conditional checkbox is satisfied
  vacuously rather than by inventing new schema.
- "Reparent/restructure" is implemented as composition/dependency graph editing on ordered child
  references. M06 does not introduce a transform-parent hierarchy.

