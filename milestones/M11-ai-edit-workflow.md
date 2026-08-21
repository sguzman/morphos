# M11 — AI Edit Workflow

## Goal

Turn the raw AI workspace API into a safe, useful co-editing workflow with proposed edits, review, live application, cancellation, and recovery.

A completed milestone means a user can ask an AI to change a workspace, inspect what it intends to do, watch approved changes arrive, and undo or reject them cleanly.

## Scope

Owns edit sessions, proposal/review/apply states, live apply behavior, and AI session cancellation.

## Subgoals

### AI edit sessions

- [ ] Define an `AiEditSession` with stable session ID.
- [ ] Record user request/intent separately from generated operations.
- [ ] Record base workspace revision.
- [ ] Track proposed, accepted, rejected, applied, and failed operations.
- [ ] Close sessions with a final structured outcome.

### Suggest/review mode

- [ ] Let AI submit one or more proposed transactions without committing them.
- [ ] Show structured before/after diffs for proposals.
- [ ] Support accept-all.
- [ ] Support reject-all.
- [ ] Support accepting/rejecting individual logical changes where transaction structure permits it.
- [ ] Revalidate proposals if the workspace changes before approval.

### Live apply mode

- [ ] Add an explicitly enabled mode where validated AI transactions can apply immediately.
- [ ] Commit each coherent AI step as its own reversible transaction.
- [ ] Stream session state updates to observers.
- [ ] Prevent stale AI steps from applying after user edits change the base assumptions.
- [ ] Make cancellation stop future AI operations without corrupting already committed history.

### Recovery and safety

- [ ] Automatically create or identify a pre-session restore point.
- [ ] Add "revert this AI session" as a first-class action.
- [ ] Ensure failed AI operations do not partially commit.
- [ ] Preserve diagnostics generated during failed/rejected proposals.
- [ ] Keep user edits made during an AI session distinct from AI-authored changes.

### Headless use

- [ ] Make suggest/review/apply session logic available without GUI.
- [ ] Support a noninteractive policy for headless runs: propose-only, auto-apply, or fail-on-needed-approval.
- [ ] Emit machine-readable session summaries.

### Tests

- [ ] Add proposal accept/reject tests.
- [ ] Add stale-workspace proposal tests.
- [ ] Add live-apply cancellation tests.
- [ ] Add revert-entire-session tests.
- [ ] Add mixed user/AI edit history tests.

## Completion criteria

- AI never needs to be granted opaque "rewrite the whole workspace" authority.
- Proposed and applied changes are visible as structured workspace mutations.
- A complete AI editing session can be reverted without manually reconstructing prior state.

