# M05 — Reactive Editing Loop

## Goal

Make Morphos feel immediate: source edits, GUI edits, and programmatic edits should propagate into geometry and the viewport with minimal ceremony.

A completed milestone means a user can edit a scene and watch valid changes appear live while invalid intermediate edits produce diagnostics without destroying the last valid preview.

## Scope

Owns file watching, debounce/coalescing, rebuild scheduling, last-good-state behavior, and live source synchronization.

## Subgoals

### File watching

- [ ] Watch canonical workspace source files for external changes.
- [ ] Debounce editor save bursts.
- [ ] Coalesce related filesystem events into one logical reload.
- [ ] Ignore Morphos's own writes where appropriate to avoid reload loops.
- [ ] Detect source file replacement/rename patterns used by common editors.

### Last-good-state behavior

- [ ] Keep the last successfully parsed/evaluated scene visible when new source is invalid.
- [ ] Surface parse/build diagnostics for the failed revision.
- [ ] Automatically recover when the source becomes valid again.
- [ ] Clearly distinguish source revision from last successful geometry revision.

### Reactive rebuild

- [ ] Convert parsed source changes into workspace revisions.
- [ ] Determine affected scene nodes/parameters.
- [ ] Rebuild only invalidated geometry where the evaluator supports it.
- [ ] Cancel or supersede stale rebuild requests when newer edits arrive.
- [ ] Prevent an older asynchronous rebuild result from replacing a newer one.

### GUI-source synchronization

- [ ] Define ownership rules for simultaneous GUI and external text edits.
- [ ] Persist GUI edits back to source through the scene/TOML layer.
- [ ] Confirm that a GUI edit becomes observable as a normal workspace revision.
- [ ] Avoid feedback loops between GUI writes and filesystem watching.

### Performance targets

- [ ] Measure edit-to-preview latency for a trivial scene.
- [ ] Measure edit-to-preview latency for the benchmark scene from M03.
- [ ] Add basic instrumentation for parse, evaluation, mesh upload, and total refresh time.
- [ ] Document acceptable initial latency targets rather than prematurely optimizing.

### Tests

- [ ] Add tests for invalid-source → last-good-preview → recovery.
- [ ] Add tests for stale rebuild suppression.
- [ ] Add tests for write/reload-loop prevention.
- [ ] Add tests for rapid consecutive edits.

## Completion criteria

- Editing and saving TOML visibly updates the scene without restarting the app.
- Invalid intermediate source does not blank or corrupt the last good viewport state.
- Rapid edits converge on the newest workspace revision.

