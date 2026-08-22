# Morphos Workspace Transactions (M07 Final)

## Goal

M07 establishes a project-owned semantic mutation boundary between GUI/automation callers and the
canonical TOML-backed workspace state.

The final M07 architecture now includes:

- typed `WorkspaceOp` scene mutations
- atomic `WorkspaceTransaction` grouping
- actor and intent metadata
- in-memory transaction-level undo/redo
- durable per-transaction history
- named snapshots
- restore-through-transaction semantics
- simple project-owned history queries
- semantic transaction diffs and current-vs-history comparisons

## Operation model

`geom_workspace` owns the structured mutation boundary through:

- `WorkspaceOp`
- `WorkspaceTransaction`
- `WorkspaceTransactionCommit`
- `TransactionActor`
- stable `OperationId`, `TransactionId`, and `SnapshotId`

Current scene-oriented operations cover the M06 authoring surface plus semantic full-state restore:

- `AddNode`
- `ReplaceNode`
- `DeleteNode`
- `RenameNode`
- `DuplicateNode`
- `SetNodeLabel`
- `SetCompositionChildren`
- `SetParameterScalar`
- `SetTransformComponent`
- `SetPrimitiveScalar`
- `SetRootNode`
- `ReplaceScene`

`ReplaceScene` is intentionally narrow: it exists to support truthful snapshot restore and semantic
revision comparison without fabricating arbitrary text diffs.

## Workspace metadata decision

Workspace metadata mutations intentionally remain outside `WorkspaceOp`.

Reasons:

- M07’s structured transaction boundary is primarily about semantic scene state.
- workspace name/description already have direct workspace-owned APIs
- metadata changes do not need to be forced into scene-history transactions to make scene editing,
  undo/redo, snapshots, or semantic diffs reliable

If a later milestone needs durable metadata provenance unified with scene history, Morphos can add a
separate metadata transaction surface then. M07 does not require it to complete the scene/history
architecture.

## Transaction model

A `WorkspaceTransaction` contains:

- one stable transaction ID
- one actor
- an optional human-readable intent/summary
- an ordered list of typed semantic operations

Transactions are rejected if they contain no operations.

Transactions are applied through `Workspace::apply_transaction`:

1. parse the current canonical source into `SceneSource`
2. validate the current semantic scene
3. apply every `WorkspaceOp` against a temporary source copy
4. capture inverse operations from the pre-operation semantic state
5. stage the final canonical source write
6. persist one durable history entry
7. save canonical source normally
8. return one coherent commit record

Canonical workspace state is only swapped into the live workspace after the staged transaction
persists successfully.

## Undo / Redo

Undo and redo operate at transaction granularity through the in-memory `UndoRedoManager`.

Rules:

- a successful new commit is pushed onto the undo stack
- undo applies one inverse transaction and moves that record to redo
- redo reapplies the original forward transaction and returns it to undo
- any new non-redo commit after undo clears redo
- external source reloads clear both stacks because they bypass in-memory structured provenance

Undo and redo reuse the normal transaction application path, so they also create new durable audit
history entries with fresh IDs and explicit undo/redo intent text.

Persistent restartable undo stacks are intentionally out of scope for M07.

## Durable history

Committed source-changing transactions are persisted under:

`<workspace>/.morphos/history/<revision>-<transaction-id>.toml`

Each history entry is project-owned, versioned, and independent of egui/Bevy.

Each entry records:

- history format version
- transaction ID
- actor
- optional intent
- timestamp in milliseconds since Unix epoch
- revision before and after commit
- affected node IDs and parameter IDs
- per-operation IDs
- per-operation kind strings and concise summaries
- canonical pre-transaction source text
- canonical post-transaction source text

The stored before/after canonical source texts let Morphos produce exact semantic diffs for one
transaction and compare current state against a historical committed revision without pretending raw
text diffs are the semantic model.

## Snapshot layout

Named snapshots are persisted in the same reserved history area:

`<workspace>/.morphos/history/<snapshot-id>.snapshot.toml`

Each snapshot records:

- snapshot format version
- stable snapshot ID
- human-readable name
- actor
- creation time in milliseconds since Unix epoch
- source revision the snapshot was created from
- canonical source text sufficient for faithful restore

Snapshots are listed through `Workspace::snapshots()` and loaded through `Workspace::snapshot(...)`.

## Snapshot restore semantics

Snapshot restore never silently overwrites the workspace.

Restore flow:

`snapshot -> SceneDocument -> WorkspaceOp::ReplaceScene -> WorkspaceTransaction -> apply_transaction -> durable history -> canonical save`

That means restore:

- reuses normal scene validation
- reuses normal persistence and atomicity rules
- emits a new history entry
- preserves the audit trail of the restore event itself

## History query API

M07 adds simple presentation-neutral history queries through:

- `Workspace::history_entries()`
- `Workspace::query_history(&HistoryQuery)`

Supported filters:

- revision
- actor
- `NodeId`
- `ParamId`
- time range

The API intentionally remains simple iterator/filter style rather than introducing a database or
query language.

## Semantic diff model

Morphos now exposes semantic scene diffs through `WorkspaceSceneDiff`.

Each diff includes:

- before revision
- after revision
- affected `NodeId`s
- affected `ParamId`s
- structured semantic changes
- one concise human-readable summary

Current structured change kinds include:

- root change
- parameter add/remove/change
- node add/remove
- node change with structured changed-field categories such as label, transform, composition
  children, primitive shape, kind, and extensions

The diff model is semantic and project-owned. Raw text diffs are not the canonical explanation of a
workspace change.

## Comparison API

M07 supports comparing current canonical state against:

- one persisted snapshot through `Workspace::compare_current_to_snapshot(...)`
- one historical committed revision through `Workspace::compare_current_to_revision(...)`
- one committed transaction through `Workspace::transaction_diff(...)`

Historical revision comparison is exact because durable history stores the canonical post-commit
source text for each committed revision.

## External-edit policy

Raw external TOML edits are intentionally outside durable transaction history in M07.

When Morphos observes an external source reload, it:

- reloads canonical source text
- clears in-memory undo/redo
- does not synthesize fake semantic `WorkspaceOp` history from the raw text change

That boundary is conservative on purpose. Morphos does not yet have a trustworthy semantic derivation
step for arbitrary external text edits.

## Robustness and failure policy

Current durability policy:

- if history entry persistence fails, the transaction fails and canonical source remains untouched
- if source persistence fails after a new history entry is written, Morphos removes that new
  history entry before returning the source persistence error
- malformed, unsupported, or partial history entries return typed `WorkspaceHistoryError`s
- malformed, unsupported, or partial snapshot files return typed `WorkspaceSnapshotError`s

Morphos uses simple versioned files appropriate to the existing workspace architecture rather than a
database or VCS.

## GUI integration

`geom_app::AppModel` routes canonical GUI scene edits through `WorkspaceTransaction` rather than ad
hoc scene-text rewrites.

Current flow:

`egui -> AppCommand -> AppModel -> WorkspaceTransaction -> Workspace::apply_transaction -> SceneSource -> Workspace save -> M05 reactive rebuild`

Undo/redo uses that same path and does not bypass the existing watcher/build boundary.

## M07 boundary

M07 is complete at the workspace/history layer.

Still intentionally out of scope:

- M08 functionality
- database-backed history
- branching version-control behavior
- geometry-level diffs
- productized history UI/CLI beyond the current programmatic APIs
