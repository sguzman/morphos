# Morphos Workspace Transactions (M07 Tranche 3)

## Goal

M07 introduces a project-owned structured mutation boundary between GUI/automation callers and the
canonical TOML-backed workspace state.

This milestone now adds:

- typed `WorkspaceOp` scene mutations
- atomic `WorkspaceTransaction` grouping
- transaction actor and optional human-readable intent
- affected-target reporting for `NodeId` and `ParamId`
- one coherent commit record returned for each accepted transaction
- durable per-transaction history entries in `.morphos/history`
- typed history read errors for malformed, unsupported, or partial entries

It still does not add snapshots, restore flows, or full before/after diffs.

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

## Undo / Redo model

Undo and redo operate at transaction granularity.

One original committed transaction is treated as one reversible unit. If a transaction contained
multiple dependent operations, undo reverses them coherently as one inverse transaction rather than
leaving partially undone intermediate states visible.

The current in-memory owner is `UndoRedoManager`, which maintains:

- an undo stack of committed reversible transactions
- a redo stack of undone transactions

Rules in this tranche:

- a successful new commit is pushed onto the undo stack
- undo moves that transaction record to the redo stack
- redo reapplies the original forward transaction and returns it to the undo stack
- any new non-redo commit after an undo clears the redo stack
- external source reloads clear both stacks because they bypass structured in-memory provenance

Undo and redo themselves now also emit durable history entries because they reuse the same
structured transaction path with fresh transaction and operation IDs plus explicit undo/redo
intent text.

## Validation and atomicity

Transactions are applied through `Workspace::apply_transaction`.

The workflow is:

1. parse the current canonical source into `SceneSource`
2. apply every `WorkspaceOp` to that temporary source
3. stop immediately if any operation fails validation
4. persist the final source once only after every operation succeeds
5. return one `WorkspaceTransactionCommit` summary, including the captured inverse transaction

Because canonical workspace state is not mutated until the full transaction validates, one bad
operation prevents partial mutation from reaching observers or disk.

Undo and redo reuse the same transaction application path, so they preserve the same atomicity
guarantees.

## Inverse capture strategy

Inverse operations are captured while the forward transaction is being validated and applied.

For each forward operation, Morphos inspects the semantic scene state that exists immediately
before that operation runs and derives the inverse operation from that pre-operation state.

Examples:

- `AddNode` inverse is `DeleteNode`
- `DeleteNode` inverse is a full-node restore operation using the exact semantic node previously
  present
- `RenameNode A -> B` inverse is `RenameNode B -> A`
- parameter/transform/primitive/label/root edits capture the old semantic value exactly
- composition child edits capture the prior ordered child list exactly

For multi-operation transactions, inverse operations are applied in reverse order. This is
important when later operations depend on earlier ones, such as rename-then-edit or
rename-then-reference-update workflows.

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

Intent is an optional trimmed human-readable summary. It is returned in the commit record and is
also persisted into the durable history entry for later inspection.

Undo and redo currently reissue transactions with fresh IDs and fresh operation IDs while carrying
explicit undo/redo intent text for later provenance expansion.

## Durable history

Committed source-changing transactions now write one project-owned versioned history file under:

`<workspace>/.morphos/history/<revision>-<transaction-id>.toml`

Each entry records:

- format version
- transaction ID
- actor
- optional intent
- revision before and after commit
- affected node IDs and parameter IDs
- per-operation IDs
- per-operation kind strings and concise summaries

Entries are sorted by revision/file name for chronological reads through
`Workspace::history_entries()`.

The history layer stays independent of egui and Bevy. It lives entirely in `geom_workspace`.

## History robustness and failure policy

History entries are written before the staged source save is finalized, but the live in-memory
workspace is only swapped into place after both history and source persistence succeed.

Current policy:

- if history entry persistence fails, the transaction fails and canonical source is left untouched
- if source persistence fails after a new history entry is written, Morphos removes that new
  history entry before returning the source persistence error
- history readers return typed errors for malformed entries, unsupported format versions, invalid
  file naming, and leftover `.tmp` partial files

This keeps canonical scene state conservative and avoids silently accepting unaudited edits, while
remaining simple enough for deterministic tests.

## External-edit policy

Raw external TOML edits are intentionally outside durable transaction history in this tranche.

When Morphos observes an external source reload, it:

- reloads the canonical source text
- clears the in-memory undo/redo stacks
- does not synthesize fake `WorkspaceOp` history entries from the text diff

That boundary is conservative on purpose. Morphos does not yet have a reliable semantic derivation
step that can turn arbitrary external text edits into truthful structured transaction provenance, so
this tranche leaves those edits outside the durable audit log instead of fabricating misleading
history.

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

`geom_app` now exposes simple Undo / Redo actions through the authoring UI and app-facing
availability state via `UndoRedoAvailability`.

Undo/redo saves source normally, triggers one normal reactive rebuild, and does not bypass the
existing watcher/build separation.

## Future M07 boundary

Still intentionally not implemented in this tranche:

- snapshot creation/restoration
- history queries
- full structured before/after diffs

Those build on this transaction foundation rather than bypassing it.
