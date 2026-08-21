# M06 — Scene Tree & Inspector

## Goal

Make the declarative scene directly manipulable from the GUI without abandoning the underlying TOML model.

A completed milestone means a user can browse scene structure, select nodes, edit common properties, and have those edits persist to source and appear immediately in the viewport.

## Scope

Owns scene navigation, selection, property editing, and generated parameter controls.

## Subgoals

### Scene tree

- [ ] Display named nodes in a hierarchical/dependency-aware tree.
- [ ] Support node selection.
- [ ] Preserve selection when unrelated scene revisions occur.
- [ ] Add visibility/preview toggles without changing canonical geometry semantics.
- [ ] Add search/filter by node name or ID.
- [ ] Add frame-selected integration with the viewport.

### Inspector

- [ ] Display selected node identity/type.
- [ ] Edit translation, rotation, and scale.
- [ ] Edit primitive dimensions.
- [ ] Edit boolean/composition-specific properties.
- [ ] Edit references through safe selectors rather than raw IDs where practical.
- [ ] Validate inspector edits before committing them to workspace state.
- [ ] Display source location and a "reveal in source" hook/API.

### Parameter UI

- [ ] Generate controls for declared scalar parameters.
- [ ] Support min/max/step metadata where provided.
- [ ] Add boolean and enum-style parameter controls where schema supports them.
- [ ] Make parameter changes flow through the same mutation/revision system as other edits.
- [ ] Display which nodes depend on a selected parameter.

### Creation/deletion basics

- [ ] Add a primitive node from the GUI.
- [ ] Rename a node with reference-safe behavior.
- [ ] Duplicate a node.
- [ ] Delete a node with dependency warnings.
- [ ] Reparent/restructure where the scene model permits it.

### Tests

- [ ] Add selection persistence tests.
- [ ] Add inspector → workspace → TOML persistence tests.
- [ ] Add rename/reference integrity tests.
- [ ] Add delete-with-dependents behavior tests.

## Completion criteria

- Common geometry authoring can happen either in TOML or through the GUI.
- Both routes produce equivalent workspace revisions and persisted source.
- The GUI never becomes a second hidden scene model.

