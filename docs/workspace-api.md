# Workspace API

M10 adds a provider-independent, headless workspace API for AI agents and other automation.
M11 extends that API into a safe AI edit-session workflow with review, live apply, cancellation,
restore points, revert handling, and a minimal desktop review surface.

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
- optional features such as the stdio transport and AI edit sessions

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

## AI Edit Sessions

M11 adds a persisted `AiEditSession` model under the reserved workspace AI data area.

Each session records:

- stable session ID
- user intent as a distinct field from AI-generated operations
- base workspace revision
- policy (`propose_only`, `auto_apply`, or `fail_on_approval_required`)
- session status
- proposals with independent stable IDs
- accepted, rejected, applied, and failed proposal tracking
- accumulated diagnostics
- event history
- optional restore point
- final structured outcome

### Session states

The current workflow uses serializable deterministic session states:

- `open`
- `awaiting_review`
- `applying`
- `completed`
- `cancelled`
- `failed`
- `reverted`

### Proposal model

Each proposal stores:

- proposal ID
- session ID
- base revision
- optional rationale
- original `TransactionProposal`
- structured semantic diff
- diagnostics
- affected nodes and parameters
- expected rebuild scope
- proposal state
- revalidation outcome
- resulting transaction ID and revision when applied

### Review workflow

The default safe path is:

1. start a session
2. submit one or more proposals
3. inspect structured diff and diagnostics
4. accept or reject individual proposals
5. optionally accept all or reject all
6. complete, cancel, or revert the session

Rejected and failed proposals remain inspectable in persisted session state.

### Stale proposal behavior

Approval remains revision-aware.

If a proposal was created against revision `N` and the current workspace is now at a later
revision, approval does not blindly apply the stale transaction. Morphos first attempts an
explicit revalidation dry-run against the current revision:

- if valid, the proposal is marked as stale-but-revalidated and then committed
- if invalid, the proposal remains stale and no commit occurs

Live auto-apply steps do not auto-rebase stale work. They are marked stale instead of committing.

### Live apply mode

`auto_apply` sessions commit each coherent AI step as its own `WorkspaceTransaction`. This keeps
history, diagnostics, and revert behavior visible at the same granularity as the session steps.

### Events and observers

Sessions persist deterministic event streams including:

- `session_started`
- `proposal_added`
- `proposal_accepted`
- `proposal_rejected`
- `proposal_failed`
- `proposal_stale`
- `transaction_applied`
- `session_cancelled`
- `session_completed`
- `session_reverted`

The stdio protocol exposes these through explicit polling rather than notifications or prompts.

### Restore points and revert

The first mutating session action lazily creates a restore snapshot using the existing workspace
snapshot system.

`revert_ai_edit_session` restores that snapshot only when it is safe to do so. If later
non-session edits exist after the session base revision, Morphos returns a structured revert
conflict instead of silently deleting interleaved user work.

### Mixed user and AI history

AI-applied transactions now persist optional session/proposal correlation metadata alongside the
existing M07 durable history records. This lets later tooling and tests answer which transactions
belonged to which AI session without creating a separate authoritative history ledger.

### Headless policies

The current M11 headless policies are:

- `propose_only`
- `auto_apply`
- `fail_on_approval_required`

No headless path prompts interactively.

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

M11 adds:

- `start_edit_session`
- `get_edit_session`
- `list_edit_sessions`
- `submit_proposal`
- `accept_proposal`
- `reject_proposal`
- `accept_all`
- `reject_all`
- `submit_live_step`
- `cancel_edit_session`
- `complete_edit_session`
- `revert_edit_session`
- `get_edit_session_events`

## Example interaction

```json
{"version":1,"id":"1","method":"get_workspace_summary","workspace_root":"examples/workspaces/viewport-smoke","params":{}}
{"version":1,"id":"2","method":"get_node","workspace_root":"examples/workspaces/viewport-smoke","params":{"node_id":"cap","include_source_snippet":true}}
{"version":1,"id":"3","method":"dry_run_transaction","workspace_root":"examples/workspaces/viewport-smoke","params":{"expected_revision":0,"actor":"ai","intent":"label cap","operations":[{"kind":"set_node_label","node_id":"cap","label":"Top Cap"}]}}
{"version":1,"id":"4","method":"apply_transaction","workspace_root":"examples/workspaces/viewport-smoke","params":{"expected_revision":0,"actor":"ai","intent":"label cap","operations":[{"kind":"set_node_label","node_id":"cap","label":"Top Cap"}]}}
{"version":1,"id":"5","method":"start_edit_session","workspace_root":"examples/workspaces/viewport-smoke","params":{"user_intent":"make the cap label clearer","policy":"propose_only"}}
{"version":1,"id":"6","method":"submit_proposal","workspace_root":"examples/workspaces/viewport-smoke","params":{"session_id":"<session-id>","request":{"rationale":"review cap label","proposal":{"expected_revision":0,"actor":"ai","intent":"rename cap label","operations":[{"kind":"set_node_label","node_id":"cap","label":"Review Cap"}]}}}}
{"version":1,"id":"7","method":"accept_proposal","workspace_root":"examples/workspaces/viewport-smoke","params":{"session_id":"<session-id>","proposal_id":"<proposal-id>"}}
```

Responses preserve IDs and return either `result` or a structured `error`.

## Verification

M10 and M11 coverage includes:

- bounded read API tests
- stale-revision mutation tests
- dry-run vs apply behavior tests
- invalid proposal diagnostic tests
- protocol version and malformed-request tests
- end-to-end fake-agent transport tests using only the public M10 API
- AI session accept/reject tests
- stale proposal revalidation tests
- live-apply cancellation tests
- revert-session safety tests
- mixed user/AI history conflict tests
- stdio M11 workflow tests for start, submit, reject, accept, cancel, and event polling
