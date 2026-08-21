# M07 — Workspace Operations & History

## Goal

Create the structured mutation system that makes user edits, AI edits, undo/redo, history, and provenance reliable.

A completed milestone means meaningful workspace changes are represented as typed operations, can be grouped into transactions, can be reversed, and can be attributed to an actor.

## Scope

Owns `WorkspaceOp`, transactions, inverse operations, history, snapshots, and authorship metadata.

## Subgoals

### Operation model

- [ ] Define project-owned `WorkspaceOp` variants for core scene mutations.
- [ ] Include operations for add/delete/replace node.
- [ ] Include operations for rename/reparent/reference changes.
- [ ] Include operations for parameter and transform changes.
- [ ] Include operations for workspace metadata changes where appropriate.
- [ ] Give every operation a stable operation ID.

### Transactions

- [ ] Allow multiple operations to be grouped into one transaction.
- [ ] Validate a transaction before mutating canonical workspace state.
- [ ] Apply transactions atomically from the perspective of observers.
- [ ] Emit one coherent revision/event record per committed transaction.
- [ ] Record transaction actor: user, AI, CLI/automation, or system/migration.
- [ ] Add optional human-readable transaction intent/summary.

### Undo/redo

- [ ] Generate or record inverse operations.
- [ ] Implement undo of a committed transaction.
- [ ] Implement redo.
- [ ] Handle new edits after undo by clearing or branching redo state explicitly.
- [ ] Confirm undo/redo persists through the normal scene serialization path.

### History and snapshots

- [ ] Persist a lightweight history log.
- [ ] Add named snapshots/checkpoints.
- [ ] Restore a snapshot as a new transaction rather than silently replacing history.
- [ ] Allow history queries by revision, actor, node, and time.
- [ ] Keep history format versioned.

### Diff model

- [ ] Produce a structured before/after diff for a transaction.
- [ ] Produce concise human-readable summaries from structured diffs.
- [ ] Expose affected node IDs and parameter paths.
- [ ] Support comparing current state to a snapshot/revision.

### Tests

- [ ] Add operation application tests.
- [ ] Add transaction atomicity tests.
- [ ] Add undo/redo round-trip tests.
- [ ] Add actor/provenance persistence tests.
- [ ] Add snapshot restore tests.
- [ ] Add structured diff tests.

## Completion criteria

- GUI and future AI code can mutate the workspace without directly rewriting arbitrary source text.
- Every committed mutation is attributable and reversible at transaction granularity.
- History can explain what changed without reconstructing meaning from raw file diffs.

