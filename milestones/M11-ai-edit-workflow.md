# M11 — AI Edit Workflow

## Goal

Turn the raw AI workspace API into a safe, useful co-editing workflow with proposed edits, review, live application, cancellation, and recovery.

A completed milestone means a user can ask an AI to change a workspace, inspect what it intends to do, watch approved changes arrive, and undo or reject them cleanly.

## Scope

Owns edit sessions, proposal/review/apply states, live apply behavior, and AI session cancellation.

## Subgoals

### AI edit sessions

- [x] Define an `AiEditSession` with stable session ID.
- [x] Record user request/intent separately from generated operations.
- [x] Record base workspace revision.
- [x] Track proposed, accepted, rejected, applied, and failed operations.
- [x] Close sessions with a final structured outcome.

### Suggest/review mode

- [x] Let AI submit one or more proposed transactions without committing them.
- [x] Show structured before/after diffs for proposals.
- [x] Support accept-all.
- [x] Support reject-all.
- [x] Support accepting/rejecting individual logical changes where transaction structure permits it.
- [x] Revalidate proposals if the workspace changes before approval.

### Live apply mode

- [x] Add an explicitly enabled mode where validated AI transactions can apply immediately.
- [x] Commit each coherent AI step as its own reversible transaction.
- [x] Stream session state updates to observers.
- [x] Prevent stale AI steps from applying after user edits change the base assumptions.
- [x] Make cancellation stop future AI operations without corrupting already committed history.

### Recovery and safety

- [x] Automatically create or identify a pre-session restore point.
- [x] Add "revert this AI session" as a first-class action.
- [x] Ensure failed AI operations do not partially commit.
- [x] Preserve diagnostics generated during failed/rejected proposals.
- [x] Keep user edits made during an AI session distinct from AI-authored changes.

### Headless use

- [x] Make suggest/review/apply session logic available without GUI.
- [x] Support a noninteractive policy for headless runs: propose-only, auto-apply, or fail-on-needed-approval.
- [x] Emit machine-readable session summaries.

### Tests

- [x] Add proposal accept/reject tests.
- [x] Add stale-workspace proposal tests.
- [x] Add live-apply cancellation tests.
- [x] Add revert-entire-session tests.
- [x] Add mixed user/AI edit history tests.

## Completion criteria

- AI never needs to be granted opaque "rewrite the whole workspace" authority.
- Proposed and applied changes are visible as structured workspace mutations.
- A complete AI editing session can be reverted without manually reconstructing prior state.

