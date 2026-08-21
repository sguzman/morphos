# Morphos Reactive Editing (M05)

Launch command:

```powershell
cargo run -p geom_app -- examples/workspaces/viewport-smoke
```

Timing harness:

```powershell
cargo test -p geom_app reactive_timing_harness -- --ignored --nocapture
```

## Architecture

- `geom_workspace` remains the canonical owner of durable source text and workspace revision.
- `geom_scene` remains the canonical owner of source-preserving TOML edits and semantic validation.
- `geom_geometry` remains the canonical owner of evaluation and subtree cache reuse.
- `geom_app` now owns file watching, debounce/coalescing, build generations, stale-result suppression, own-write suppression, and Bevy mesh upload.

The app watches the containing `source/` directory rather than a single file handle, then filters to the canonical `source/scene.toml` path. That keeps watcher behavior stable across common editor save-via-replace workflows where the canonical file is removed and reintroduced.

## Reactive Loop

External source flow:

1. raw filesystem events arrive through `notify`
2. the app coalesces them through a 75 ms debounce window
3. the workspace reload path runs through `Workspace::reload_source()`
4. accepted source text advances an app-owned `SourceRevision`
5. the app schedules a monotonic `BuildGeneration`
6. a dedicated worker thread parses and evaluates the snapshot while preserving one evaluator cache per app session
7. the main thread accepts only the newest generation for the current transient session
8. successful geometry uploads replace the displayed Bevy mesh without touching the camera

Internal source flow:

1. Morphos starts from the latest known source text
2. it applies a targeted `SceneSource` edit
3. it verifies the on-disk source still matches the edit base fingerprint
4. it persists through `geom_workspace`
5. it records the expected own-write fingerprint
6. it schedules the same reactive build path used by watcher-triggered changes

## Revision Semantics

- `Workspace Revision`: owned by `geom_workspace`; advances for workspace mutations and saves according to M01 semantics.
- `Source Revision`: owned by the reactive layer; advances only when accepted source text changes.
- `Build Generation`: owned by the reactive layer; advances for every scheduled rebuild, including manual rebuilds.
- `Geometry Revision`: owned by `geom_geometry`; advances only when a new evaluated geometry result is accepted.
- `Display Geometry Revision`: the viewport-visible revision derived from the accepted Morphos geometry result.

This separation makes comment-only edits normal: source revision can advance while geometry revision stays unchanged.

## Own-Write Suppression

Morphos records the fingerprint of the exact source text it just wrote. If a subsequent watcher echo resolves to the same fingerprint at the same current source revision, the event is treated as Morphos's own write and no duplicate logical rebuild is scheduled.

This is content-aware suppression, not a blind time-based ignore window. A genuinely different external save immediately after a Morphos write is still accepted as a new source revision.

## Stale Work And Cancellation

- Stale build results are rejected unless their `WorkspaceSessionId` and `BuildGeneration` match the newest request still relevant to the current session.
- Cancellation is logical rather than physical: obsolete queued builds are superseded and obsolete finished results are discarded.
- Reopen starts a fresh transient session. Old watcher events and old worker results are ignored once the new session begins.

## Last-Good Behavior

- Invalid source keeps the last successful geometry visible.
- Geometry evaluation failure keeps the last successful geometry visible.
- Later valid source automatically clears the error and replaces the viewport geometry.
- Camera yaw, pitch, target, and distance remain untouched across reactive refreshes unless the user explicitly frames geometry.

## Affected IDs And Cache Reuse

The worker retains one `GeometryEvaluator<BoolmeshBackend>` per session, so M03 subtree caching survives successive edits. The reactive layer also computes changed `NodeId` and `ParamId` sets between the last successful semantic scene and the newly parsed semantic scene for status/reporting purposes.

Formatting-only or comment-only edits parse into an unchanged `SceneDocument`, so the worker reports a semantic no-op and avoids unnecessary geometry replacement.

## Conflict Rule

Morphos does not silently overwrite known-stale external edits. Before persisting an in-app source mutation, it reads the canonical source file and compares its fingerprint to the current in-memory edit base. If they differ, the GUI/programmatic edit is rejected as a conflict and must be retried against the newest source.

## Measured Latency

Measured on Friday, August 21, 2026 with:

```powershell
cargo test -p geom_app reactive_timing_harness -- --ignored --nocapture
```

Results:

- Trivial smoke workspace edit:
  `parse 1.01 ms`, `evaluation 56.31 ms`, `mesh 0.16 ms`, `total 64.44 ms`
- Benchmark workspace edit:
  `parse 1.64 ms`, `evaluation 66.05 ms`, `mesh 0.14 ms`, `total 72.29 ms`

Initial targets:

- small local edits should feel effectively immediate in normal interactive use
- cached benchmark edits should remain comfortably interactive
- correctness wins over aggressive cancellation or speculative optimization

These are descriptive targets, not CI thresholds.

## Manual Verification

In a local desktop session:

1. launch the smoke workspace
2. orbit/pan/zoom to a non-default view
3. edit `source/scene.toml` externally and save
4. confirm the viewport refreshes automatically
5. confirm the camera stays unchanged
6. save invalid TOML
7. confirm the last good mesh remains visible and an error appears
8. repair the TOML and save
9. confirm the viewport recovers automatically
10. save repeatedly in quick succession
11. confirm the final visible geometry matches the newest source
12. trigger the small in-app parameter nudge and confirm it persists without watcher-loop churn

Environment note:

- On Friday, August 21, 2026, the coding-agent environment compiled the shell, ran the reactive tests, launched the app successfully, and recorded timing metrics.
- This environment did not claim visual inspection of the desktop window, so viewport interaction checks remain a manual follow-up in an interactive desktop session.
