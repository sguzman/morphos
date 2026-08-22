# M09 — Validation & Diagnostics

## Goal

Make failures legible and local. Morphos should explain what is wrong, where it is wrong, what is affected, and whether the last good geometry remains usable.

A completed milestone means source, scene, geometry, and export failures share a coherent diagnostic system used by CLI, GUI, and AI.

## Scope

Owns diagnostic types, severity, source spans, affected entities, validation passes, and presentation-neutral reporting.

## Subgoals

### Diagnostic model

- [x] Define project-owned diagnostic severity levels.
- [x] Define stable diagnostic codes.
- [x] Attach source file/span when available.
- [x] Attach node/parameter IDs when available.
- [x] Support primary message, notes, and suggested remediation text.
- [x] Separate errors that block evaluation from warnings that permit it.

### Static scene validation

- [x] Validate IDs and references.
- [x] Detect dependency cycles.
- [x] Validate numeric ranges and finite values.
- [x] Validate required composition children/operands.
- [x] Validate unsupported/unknown backend capability requests.
- [x] Add extensible validation-pass registration.

### Geometry diagnostics

- [x] Normalize backend failures into diagnostics.
- [x] Report empty results where unexpected.
- [x] Report suspicious/invalid mesh output when detectable.
- [x] Attach rebuild/evaluation timing to optional diagnostic telemetry.
- [x] Distinguish source-valid but geometry-failed revisions from parse failures.

### UI/CLI presentation

- [x] Render diagnostics in the desktop app.
- [x] Add click/focus hooks for diagnostics tied to nodes/source.
- [x] Print concise human diagnostics in CLI mode.
- [x] Emit structured diagnostics in JSON/machine mode.
- [x] Keep diagnostic formatting out of the core diagnostic data types.

### Tests

- [x] Add golden tests for representative diagnostic structures.
- [x] Add source-span tests.
- [x] Add cycle/broken-reference tests.
- [x] Add backend-error normalization tests.
- [x] Add GUI/CLI-independent serialization tests.

## Completion criteria

- The same underlying diagnostic can be surfaced in the GUI, CLI, or AI context.
- Errors point back to meaningful source/node context.
- Invalid edits fail locally instead of turning into unexplained blank scenes.

