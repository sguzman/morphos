# Morphos Workspace Transactions (M07 Tranche 1)

## Goal

M07 introduces a project-owned structured mutation boundary between GUI/automation callers and the
canonical TOML-backed workspace state.

This tranche adds:

- typed `WorkspaceOp` scene mutations
- atomic `WorkspaceTransaction` grouping
- transaction actor and optional human-readable intent
- affected-target reporting for `NodeId` and `ParamId`
- one coherent commit record returned for each accepted transaction

This tranche does not yet add undo, redo, persistent history, snapshots, or full before/after
diffs.

## Operation model

`geom_workspace` now owns the structured mutation entry point through:

- `WorkspaceOp`
- `WorkspaceTransaction`
- `WorkspaceTransactionCommit`
- `TransactionActor`
- stable `OperationId` and `TransactionId`

Current scene-oriented operations cover the M06 authoring surface:

- add node
- delete node
- rename node
- duplicate node
- set node label
- set composition children
- set parameter scalar
- set transform component
- set primitive scalar
- set root node

These operations are semantic changes, not arbitrary text replacement requests.

## Transaction model

A `WorkspaceTransaction` contains:

- one stable transaction ID
- one actor
- an optional intent/summary string
- an ordered list of typed operations

Transactions are rejected if they contain no operations.

## Validation and atomicity

Transactions are applied through `Workspace::apply_transaction`.

The workflow is:

1. parse the current canonical source into `SceneSource`
2. apply every `WorkspaceOp` to that temporary source
3. stop immediately if any operation fails validation
4. persist the final source once only after every operation succeeds
5. return one `WorkspaceTransactionCommit` summary

Because canonical workspace state is not mutated until the full transaction validates, one bad
operation prevents partial mutation from reaching observers or disk.

## SceneSource integration

`geom_workspace` does not reimplement TOML editing itself.

Instead, each `WorkspaceOp` delegates to the source-preserving mutation APIs already owned by
`geom_scene::SceneSource`. That preserves the M02/M06 source-authority boundary while moving the
structured mutation boundary up to the workspace layer.

Conceptually:

`caller -> WorkspaceTransaction -> WorkspaceOp[] -> SceneSource -> Workspace save`

## Actor and intent

Current transaction actors are:

- `User`
- `Ai`
- `CliAutomation`
- `SystemMigration`

Intent is an optional trimmed human-readable summary. It is returned in the commit record for UI,
CLI, or future history consumers, but this tranche does not yet persist a durable history log.

## Affected targets

Each operation reports its affected mutation targets, and the transaction commit returns their
union:

- affected `NodeId`s
- affected `ParamId`s

This is the minimal structured change summary needed by the transaction foundation. Rich structured
diffs remain later M07 work.

## M06 GUI integration

`geom_app::AppModel` now routes canonical GUI scene edits through the transaction layer instead of
calling ad hoc `SceneSource` closures directly.

The current flow is:

`egui -> AppCommand -> AppModel -> WorkspaceTransaction -> Workspace::apply_transaction -> SceneSource -> Workspace save -> M05 reactive rebuild`

M05 conflict checking and own-write suppression remain unchanged. The GUI still verifies the
on-disk fingerprint before committing a transaction so stale external edits are not overwritten.

## Future M07 boundary

Still intentionally not implemented in this tranche:

- undo/redo
- inverse operation generation
- persistent history log
- snapshot creation/restoration
- history queries
- full structured before/after diffs

Those build on this transaction foundation rather than bypassing it.
