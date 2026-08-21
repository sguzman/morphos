# M09 — Validation & Diagnostics

## Goal

Make failures legible and local. Morphos should explain what is wrong, where it is wrong, what is affected, and whether the last good geometry remains usable.

A completed milestone means source, scene, geometry, and export failures share a coherent diagnostic system used by CLI, GUI, and AI.

## Scope

Owns diagnostic types, severity, source spans, affected entities, validation passes, and presentation-neutral reporting.

## Subgoals

### Diagnostic model

- [ ] Define project-owned diagnostic severity levels.
- [ ] Define stable diagnostic codes.
- [ ] Attach source file/span when available.
- [ ] Attach node/parameter IDs when available.
- [ ] Support primary message, notes, and suggested remediation text.
- [ ] Separate errors that block evaluation from warnings that permit it.

### Static scene validation

- [ ] Validate IDs and references.
- [ ] Detect dependency cycles.
- [ ] Validate numeric ranges and finite values.
- [ ] Validate required composition children/operands.
- [ ] Validate unsupported/unknown backend capability requests.
- [ ] Add extensible validation-pass registration.

### Geometry diagnostics

- [ ] Normalize backend failures into diagnostics.
- [ ] Report empty results where unexpected.
- [ ] Report suspicious/invalid mesh output when detectable.
- [ ] Attach rebuild/evaluation timing to optional diagnostic telemetry.
- [ ] Distinguish source-valid but geometry-failed revisions from parse failures.

### UI/CLI presentation

- [ ] Render diagnostics in the desktop app.
- [ ] Add click/focus hooks for diagnostics tied to nodes/source.
- [ ] Print concise human diagnostics in CLI mode.
- [ ] Emit structured diagnostics in JSON/machine mode.
- [ ] Keep diagnostic formatting out of the core diagnostic data types.

### Tests

- [ ] Add golden tests for representative diagnostic structures.
- [ ] Add source-span tests.
- [ ] Add cycle/broken-reference tests.
- [ ] Add backend-error normalization tests.
- [ ] Add GUI/CLI-independent serialization tests.

## Completion criteria

- The same underlying diagnostic can be surfaced in the GUI, CLI, or AI context.
- Errors point back to meaningful source/node context.
- Invalid edits fail locally instead of turning into unexplained blank scenes.

