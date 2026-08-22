# M07 — Workspace Operations & History

## Goal

Create the structured mutation system that makes user edits, AI edits, undo/redo, history, and provenance reliable.

A completed milestone means meaningful workspace changes are represented as typed operations, can be grouped into transactions, can be reversed, and can be attributed to an actor.

## Scope

Owns `WorkspaceOp`, transactions, inverse operations, history, snapshots, and authorship metadata.

## Subgoals

### Operation model

- [x] Define project-owned `WorkspaceOp` variants for core scene mutations.
- [x] Include operations for add/delete/replace node.
- [x] Include operations for rename/reparent/reference changes.
- [x] Include operations for parameter and transform changes.
- [x] Re-evaluate workspace metadata changes and keep them outside scene transactions when they do not belong.
- [x] Give every operation a stable operation ID.

### Transactions

- [x] Allow multiple operations to be grouped into one transaction.
- [x] Validate a transaction before mutating canonical workspace state.
- [x] Apply transactions atomically from the perspective of observers.
- [x] Emit one coherent revision/event record per committed transaction.
- [x] Record transaction actor: user, AI, CLI/automation, or system/migration.
- [x] Add optional human-readable transaction intent/summary.

### Undo/redo

- [x] Generate or record inverse operations.
- [x] Implement undo of a committed transaction.
- [x] Implement redo.
- [x] Handle new edits after undo by clearing or branching redo state explicitly.
- [x] Confirm undo/redo persists through the normal scene serialization path.

### History and snapshots

- [x] Persist a lightweight history log.
- [x] Add named snapshots/checkpoints.
- [x] Restore a snapshot as a new transaction rather than silently replacing history.
- [x] Allow history queries by revision, actor, node, and time.
- [x] Keep history format versioned.

### Diff model

- [x] Produce a structured before/after diff for a transaction.
- [x] Produce concise human-readable summaries from structured diffs.
- [x] Expose affected node IDs and parameter paths.
- [x] Support comparing current state to a snapshot/revision.

### Tests

- [x] Add operation application tests.
- [x] Add transaction atomicity tests.
- [x] Add undo/redo round-trip tests.
- [x] Add actor/provenance persistence tests.
- [x] Add snapshot restore tests.
- [x] Add structured diff tests.

## Notes

- M07 now covers the transaction foundation, in-memory transaction-level undo/redo, durable
  per-transaction history, named snapshots, simple history queries, semantic diff/comparison APIs,
  and restore-via-transaction.
- The current operation surface matches the M06 authoring mutations and now includes explicit
  full-node and full-scene replacement operations. Workspace metadata remains outside
  `WorkspaceOp`: metadata edits already have a coherent workspace-level API, are not part of the
  scene semantic mutation surface, and do not need to be forced into scene-history transactions to
  satisfy M07.
- Undo/redo still operates on in-memory structured transaction records and clears redo on any new
  post-undo commit rather than branching redo state, but undo/redo commits are also captured in
  durable history through the normal transaction path.

## Status

- M07 is complete. Remaining future work belongs to later milestones rather than this one.

## Completion criteria

- GUI and future AI code can mutate the workspace without directly rewriting arbitrary source text.
- Every committed mutation is attributable and reversible at transaction granularity.
- History can explain what changed without reconstructing meaning from raw file diffs.

