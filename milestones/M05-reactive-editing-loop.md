# M05 — Reactive Editing Loop

## Goal

Make Morphos feel immediate: source edits, GUI edits, and programmatic edits should propagate into geometry and the viewport with minimal ceremony.

A completed milestone means a user can edit a scene and watch valid changes appear live while invalid intermediate edits produce diagnostics without destroying the last valid preview.

## Scope

Owns file watching, debounce/coalescing, rebuild scheduling, last-good-state behavior, and live source synchronization.

## Subgoals

### File watching

- [x] Watch canonical workspace source files for external changes.
- [x] Debounce editor save bursts.
- [x] Coalesce related filesystem events into one logical reload.
- [x] Ignore Morphos's own writes where appropriate to avoid reload loops.
- [x] Detect source file replacement/rename patterns used by common editors.

### Last-good-state behavior

- [x] Keep the last successfully parsed/evaluated scene visible when new source is invalid.
- [x] Surface parse/build diagnostics for the failed revision.
- [x] Automatically recover when the source becomes valid again.
- [x] Clearly distinguish source revision from last successful geometry revision.

### Reactive rebuild

- [x] Convert parsed source changes into workspace revisions.
- [x] Determine affected scene nodes/parameters.
- [x] Rebuild only invalidated geometry where the evaluator supports it.
- [x] Cancel or supersede stale rebuild requests when newer edits arrive.
- [x] Prevent an older asynchronous rebuild result from replacing a newer one.

### GUI-source synchronization

- [x] Define ownership rules for simultaneous GUI and external text edits.
- [x] Persist GUI edits back to source through the scene/TOML layer.
- [x] Confirm that a GUI edit becomes observable as a normal workspace revision.
- [x] Avoid feedback loops between GUI writes and filesystem watching.

### Performance targets

- [x] Measure edit-to-preview latency for a trivial scene.
- [x] Measure edit-to-preview latency for the benchmark scene from M03.
- [x] Add basic instrumentation for parse, evaluation, mesh upload, and total refresh time.
- [x] Document acceptable initial latency targets rather than prematurely optimizing.

### Tests

- [x] Add tests for invalid-source → last-good-preview → recovery.
- [x] Add tests for stale rebuild suppression.
- [x] Add tests for write/reload-loop prevention.
- [x] Add tests for rapid consecutive edits.

## Completion criteria

- Editing and saving TOML visibly updates the scene without restarting the app.
- Invalid intermediate source does not blank or corrupt the last good viewport state.
- Rapid edits converge on the newest workspace revision.

## Verification Notes

- `geom_app` now owns a dedicated reactive layer with source fingerprints, source revisions,
  build generations, transient workspace-session IDs, a 75 ms debounce/coalescing boundary, a
  watched `source/` directory, and a dedicated build worker that preserves one
  `GeometryEvaluator<BoolmeshBackend>` cache per session.
- In-app parameter nudges persist back through `SceneSource`, `geom_workspace`, disk, and the
  same generation-tagged reactive rebuild path used by watcher-triggered external edits.
- Last-good geometry remains visible across invalid source revisions and later valid revisions
  recover automatically.
- Deterministic tests cover invalid-to-recovery, stale generation suppression, own-write echo
  suppression, semantic no-op edits, rapid consecutive edits, programmatic edits, and stale
  session results after reopen.
- Timing measurements were captured on Friday, August 21, 2026 with
  `cargo test -p geom_app reactive_timing_harness -- --ignored --nocapture`:
  smoke workspace `parse 1.01 ms`, `evaluation 56.31 ms`, `mesh 0.16 ms`, `total 64.44 ms`;
  benchmark workspace `parse 1.64 ms`, `evaluation 66.05 ms`, `mesh 0.14 ms`, `total 72.29 ms`.
- The coding-agent environment launched the shell successfully on Friday, August 21, 2026, but
  did not claim visual inspection of the live desktop window. Interactive viewport confirmation
  remains a manual local follow-up.

