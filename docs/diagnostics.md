# Morphos Validation & Diagnostics (M09)

`geom_diagnostics` is the project-owned, presentation-neutral diagnostic layer shared by scene
validation, geometry normalization, the reactive desktop app, and the headless CLI.

## Ownership

- `geom_diagnostics` owns serializable `Diagnostic` and `DiagnosticReport` types.
- `geom_scene` normalizes parse/validation failures into shared diagnostics through
  `parse_scene_report()` and `diagnostic_from_scene_error()`.
- `geom_geometry` normalizes evaluation and mesh issues through
  `diagnostic_from_geometry_error()` and `validate_evaluated_geometry()`.
- `geom_app` consumes the same reports in the reactive worker, desktop UI, and CLI.

The core crate deliberately does not depend on Bevy, egui, terminal formatting, or a specific AI
API boundary.

## Diagnostic model

Each `Diagnostic` carries:

- `severity`
- stable machine-readable `code`
- primary `message`
- optional source path/line/column/span
- optional `node_id`
- optional `parameter_id`
- free-form `notes`
- optional `remediation`
- `blocking` semantics
- optional timing telemetry
- optional string context map

`DiagnosticReport` is a thin container that preserves deterministic ordering and provides
`has_blocking()` and `primary_message()` helpers for higher layers.

## Stable code policy

Stable codes are Morphos-owned strings such as:

- `MORPHOS_SOURCE_PARSE`
- `MORPHOS_INVALID_ROOT`
- `MORPHOS_BROKEN_PARAMETER_REFERENCE`
- `MORPHOS_DEPENDENCY_CYCLE`
- `MORPHOS_UNKNOWN_OUTPUT`
- `MORPHOS_GEOMETRY_BACKEND`

These codes are the machine contract. Human-readable messages may change as long as the stable
meaning of the code does not.

## Scene validation

`geom_scene` still preserves the M02 parsing boundary, so some invalid conditions are rejected
while constructing the semantic `SceneDocument`. `parse_scene_report()` keeps that behavior but
returns a shared diagnostic report on failure.

After a semantic document exists, `validate_scene_document()` performs deterministic validation
passes for:

- invalid root ownership
- broken parameter references in semantic expressions
- non-finite parameter values
- invalid/non-positive scale values
- invalid/non-positive primitive ranges
- composition child-count requirements
- broken composition child references
- dependency cycles

The current pass registration is intentionally modest: a fixed list of validation functions rather
than a plugin framework.

## Geometry normalization

`geom_geometry` keeps its internal typed `GeometryError` model and normalizes it at the reporting
boundary. This preserves the important M09 distinction between:

- source invalid
- source valid, geometry failed

Post-evaluation checks also detect:

- empty mesh results
- non-finite mesh positions
- invalid/non-finite bounds

Backend details remain notes/context on Morphos diagnostics rather than becoming the public error
API.

## Reactive behavior

The M05 last-good semantics remain intact:

- invalid current source does not discard the last successful geometry
- geometry failures do not discard the last successful geometry
- the current build status still distinguishes scene failures from geometry failures
- the UI now explicitly says when the viewport is showing last-good geometry while the current
  revision has diagnostics

Reactive worker timings are attached to diagnostics as optional telemetry, but timings are not
part of diagnostic identity.

## CLI rendering

CLI exit codes and diagnostics are separate layers:

- `CliExitCode` communicates usage/io/source/geometry/internal categories
- diagnostics explain the Morphos problem in detail

Human CLI output includes:

- severity
- stable code
- primary message
- node/parameter context when present
- source location when present
- notes/remediation when present

`--json` output emits structured diagnostics instead of a preformatted error string whenever a
Morphos-domain diagnostic report exists.

## Desktop rendering

The desktop app renders the same diagnostics in a simple diagnostics window and the viewport
overlay. Current hooks include:

- select the referenced node from a diagnostic entry
- copy the source location for diagnostics with source context

This intentionally reuses the existing M06 scene-selection and source-reveal patterns instead of
introducing a GUI-only diagnostic model.
