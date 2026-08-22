# Workspace API

M10 adds a provider-independent, headless workspace API for AI agents and other automation.

The canonical entry point is the `geom_workspace_api` crate. It depends on the project-owned
workspace, scene, geometry, and diagnostics crates and does not depend on Bevy, egui, terminal
formatting, or any vendor SDK.

## Architecture

The API layers on top of existing Morphos systems instead of replacing them:

- `geom_workspace` remains the source of truth for durable workspace state, history, snapshots,
  and transaction application.
- `geom_scene` remains the source of truth for semantic scene structure, source snippets, and
  source-preserving edits.
- `geom_geometry` remains the source of truth for evaluation, bounds, and mesh statistics.
- `geom_diagnostics` remains the source of truth for shared structured diagnostics.

The workspace API does not expose arbitrary file writes, shell execution, or raw GUI state.
Canonical scene mutations still flow through `WorkspaceTransaction` and `WorkspaceOp`.

## Read surface

`WorkspaceApi` exposes bounded, revision-bearing read calls:

- `get_workspace_summary`
- `get_scene_tree`
- `get_node`
- `get_parameters`
- `get_diagnostics`
- `get_recent_history`
- `evaluate_output`
- `request_preview`
- `source_snippet`

Every response is wrapped in `WorkspaceReadContext<T>` so clients always receive the current
workspace revision alongside the structured payload.

## Context shaping and bounds

M10 is designed to avoid giant project dumps.

- Workspace summaries stay concise and include counts, revision, root/output, diagnostics, recent
  history summary, snapshot count, and capabilities.
- Large collections use `BoundedList<T>` with deterministic ordering, total counts, and explicit
  `truncated` state.
- Node details are selective and operate on one requested `NodeId`.
- Source snippets are opt-in and bounded around node or parameter source locations.
- Diagnostics, history, parameters, and scene tree results support bounded retrieval rather than
  defaulting to full workspace dumps.

## Capabilities

`WorkspaceApi::capabilities()` reports the structured contract supported by the current build:

- protocol version
- supported API mutation kinds
- supported node kinds
- geometry backend capability hints
- export formats
- preview, diagnostics, history, snapshots, dry-run, and source-snippet support
- optional features such as the stdio transport

Clients should use this capability response instead of assuming every optional feature exists.

## Mutation contract

Mutations are submitted as `TransactionProposal`.

Each proposal includes:

- `expected_revision`
- actor string mapped onto `TransactionActor`
- optional intent
- one or more supported `WorkspaceOpRequest` values

The M10 mutation surface is intentionally structured and bounded. It does not expose arbitrary
filesystem mutation or whole-source replacement.

### Revision safety

Every proposal is revision-checked before execution.

If the expected revision does not match the current workspace revision, the API returns a
structured stale-revision error and does not partially apply anything.

### Dry-run semantics

`dry_run_transaction` performs the same semantic validation as commit, but entirely in memory.

Dry-runs:

- do not mutate canonical workspace state
- do not append durable history
- do not increment canonical workspace revision
- return structured semantic diffs
- return affected nodes and parameters
- return expected rebuild scope

### Apply semantics

`apply_transaction` uses the canonical `WorkspaceTransaction` path. Successful commits return:

- acceptance state
- base revision
- resulting revision
- transaction ID
- affected nodes and parameters
- semantic diff
- expected rebuild scope

If validation fails, the API returns shared `Diagnostic` payloads rather than AI-specific prose.

## Preview and evaluation

Preview and evaluation are general automation features, not LLM-only helpers.

- `evaluate_output` returns geometry bounds, triangle and vertex counts, participating nodes, and
  bounded resolved parameter state.
- `request_preview` reuses headless preview logic and returns workspace-relative artifact
  references plus structured metadata and diagnostics.

Preview artifacts are written under the workspace rather than streamed as large raw image payloads
 through the protocol.

## Stdio protocol

The first external transport is a versioned stdio JSON protocol implemented by the
`morphos-workspace-api` binary.

Transport properties:

- headless, no GUI initialization
- JSON request/response envelopes
- request ID preservation
- structured errors
- stdout reserved for protocol messages
- stderr reserved for fatal process-level failures
- stateful workspace sessions inside the process so revision checks remain meaningful across
  multiple requests

Supported protocol methods mirror the public M10 API:

- `capabilities`
- `get_workspace_summary`
- `get_scene_tree`
- `get_node`
- `get_parameters`
- `get_diagnostics`
- `get_recent_history`
- `evaluate_output`
- `request_preview`
- `source_snippet`
- `dry_run_transaction`
- `apply_transaction`

## Example interaction

```json
{"version":1,"id":"1","method":"get_workspace_summary","workspace_root":"examples/workspaces/viewport-smoke","params":{}}
{"version":1,"id":"2","method":"get_node","workspace_root":"examples/workspaces/viewport-smoke","params":{"node_id":"cap","include_source_snippet":true}}
{"version":1,"id":"3","method":"dry_run_transaction","workspace_root":"examples/workspaces/viewport-smoke","params":{"expected_revision":0,"actor":"ai","intent":"label cap","operations":[{"kind":"set_node_label","node_id":"cap","label":"Top Cap"}]}}
{"version":1,"id":"4","method":"apply_transaction","workspace_root":"examples/workspaces/viewport-smoke","params":{"expected_revision":0,"actor":"ai","intent":"label cap","operations":[{"kind":"set_node_label","node_id":"cap","label":"Top Cap"}]}}
```

Responses preserve IDs and return either `result` or a structured `error`.

## Verification

M10 coverage includes:

- bounded read API tests
- stale-revision mutation tests
- dry-run vs apply behavior tests
- invalid proposal diagnostic tests
- protocol version and malformed-request tests
- end-to-end fake-agent transport tests using only the public M10 API
