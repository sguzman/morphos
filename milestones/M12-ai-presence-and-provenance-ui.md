# M12 — AI Presence & Provenance UI

## Goal

Make AI activity continuously legible to the user.

A completed milestone means the desktop app clearly communicates whether AI is reading, planning, proposing, editing, rebuilding, waiting for approval, finished, or failed—and lets the user inspect exactly what the AI changed.

## Scope

Owns status chips, activity feed, change highlighting, AI-focused history views, and notification behavior.

## Subgoals

### AI status model

- [ ] Define presentation-neutral AI activity states such as idle, reading, planning, proposing, applying, rebuilding, awaiting approval, completed, cancelled, and failed.
- [ ] Allow each state to carry a concise current action message.
- [ ] Include AI session ID and affected node IDs where available.
- [ ] Ensure status transitions are event-driven rather than inferred by polling UI state.

### Status chip

- [ ] Add a persistent compact AI status chip to the application shell.
- [ ] Show high-level text such as `AI edits in progress`.
- [ ] Show more specific substatus such as `Adjusting torso proportions`.
- [ ] Expose pending proposal/edit count where useful.
- [ ] Make failure/awaiting-approval states impossible to miss.
- [ ] Clicking the chip opens relevant AI activity/session details.

### Activity feed

- [ ] Add an AI activity/history panel.
- [ ] Show proposed/applied/rejected changes with timestamps/order.
- [ ] Show concise structured summaries rather than raw model prose alone.
- [ ] Link activity entries to affected nodes and workspace revisions.
- [ ] Add filters for AI/user/system edits.

### Viewport change visibility

- [ ] Highlight or outline nodes affected by the latest AI transaction.
- [ ] Make clicking an AI history item focus/select affected geometry.
- [ ] Add optional brief visual indication when a node changes.
- [ ] Avoid persistent effects that obscure actual geometry.
- [ ] Provide a way to compare current state against the pre-session snapshot.

### Notifications

- [ ] Add non-modal completion notification for AI sessions.
- [ ] Summarize number of applied/rejected/failed logical changes.
- [ ] Add actionable `Review`, `Undo session`, or `View changes` affordances.
- [ ] Prevent notification spam during multi-step live editing.
- [ ] Preserve completed session status in history even after transient notifications disappear.

### Tests

- [ ] Add AI-state transition tests independent of rendering.
- [ ] Add activity-feed ordering/filter tests.
- [ ] Add transaction-to-affected-node mapping tests.
- [ ] Add notification coalescing tests.

## Completion criteria

- The user can always answer: "Is AI doing something right now?"
- The user can answer: "What did AI just change?"
- The user can jump from an AI change record to the corresponding scene entities and revision.

