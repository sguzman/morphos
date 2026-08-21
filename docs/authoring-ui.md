# Morphos Authoring UI (M06)

Launch command:

```powershell
cargo run -p geom_app -- examples/workspaces/authoring-smoke
```

## Layout

- top status window for workspace/build/revision state and viewport controls
- left scene-tree window for graph navigation and node creation
- right inspector window for node properties, structural edits, and parameter controls

The current `bevy_egui` integration exposes anchored windows rather than dock/panel containers in this app build. The authoring layout still maps to the intended left/tree, center/viewport, right/inspector workflow.

## Scene-tree projection

- The tree derives from the current accepted `SceneDocument`, not from a GUI-owned copy.
- Composition dependencies are shown as explicit ordered child references.
- Shared/reused nodes are marked as shared when multiple compositions reference them.
- Unreferenced nodes are listed separately rather than being forced into a fake hierarchy.
- Search matches `NodeId` and label using case-insensitive substring matching.

## Selection semantics

- Selection identity is the stable `NodeId`.
- Tree selection drives the inspector and `Frame Selected`.
- Unrelated successful revisions preserve selection when the selected `NodeId` still exists.
- Deleted or missing selections fall back to the current scene root as a safe default.
- Invalid current source keeps the last-good scene readable but disables canonical edits.

## Inspector source of truth

- Inspector values are read directly from the current accepted `SceneDocument`.
- The UI does not keep a second mutable semantic scene copy.
- Accepted edits dispatch typed app commands into the existing M05 path:

  `egui -> AppCommand -> AppModel -> SceneSource -> validate -> workspace save -> reactive rebuild`

## Transform and primitive editing

- Literal transform components use numeric drag controls for translate, rotate degrees, and scale.
- Literal primitive dimensions use numeric drag controls for supported M02 primitives.
- Parameter-driven fields are shown as parameter-driven and are not silently converted to literals.
- Scale and primitive validation still flows through `SceneSource`/schema validation.

## Parameters and dependencies

- Scalar parameters render with editable numeric controls.
- Parameter extension metadata such as `units` is shown when present.
- The current M02 schema only exposes scalar parameters, so boolean and enum parameter widgets are
  not applicable yet.
- Direct and transitive dependent nodes are derived from semantic scene references, not raw TOML search.
- Updating a parameter preserves parameter-backed primitive references.

## Structural editing

- Add unreferenced primitive nodes with deterministic defaults.
- Rename updates node table keys, root references, and composition child references.
- Duplicate performs a shallow declarative copy and preserves child/parameter references.
- Delete is conservative:
  - root nodes are blocked
  - referenced nodes are blocked with dependent warnings
  - unreferenced nodes can be deleted
- Composition editing supports ordered child replacement, append, removal when still valid, and reordering.

## Source location / reveal hook

- Node and parameter entries show `source/scene.toml` line/column information from `SceneSource`.
- The inspector exposes a generic copy-location action rather than coupling to a specific editor.

## Invalid-source and conflict behavior

- When the current source is invalid, Morphos keeps the last-good scene readable.
- Canonical GUI writes are rejected while the source is invalid.
- Before saving a GUI edit, Morphos still performs the M05 source-fingerprint conflict check against disk.
- External accepted source updates refresh the displayed scene through the normal watcher/rebuild path.

## M06 / M07 boundary

M06 adds direct GUI authoring over TOML-backed scene state, but it does not add:

- `WorkspaceOp`
- transactions
- undo/redo
- provenance/history
- structured transaction diffs

Those remain M07 work.
