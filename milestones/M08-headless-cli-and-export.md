# M08 — Headless CLI & Export

## Goal

Make every essential geometry workflow usable without the desktop application.

A completed milestone means Morphos can validate, inspect, evaluate, render/export, and apply structured edits from the command line.

## Scope

Owns CLI commands, non-GUI execution, deterministic export, and batch-friendly exit behavior.

## Subgoals

### CLI foundation

- [x] Create a `morphos` CLI binary separate from the desktop app binary.
- [x] Add `morphos validate <workspace>`.
- [x] Add `morphos inspect <workspace>` with machine-readable output option.
- [x] Add `morphos eval <workspace>` or equivalent build command.
- [x] Define stable exit codes for success, source error, geometry error, and I/O error.
- [x] Ensure commands do not initialize windowing/GUI systems.

### Mesh export

- [x] Add at least one practical mesh export format.
- [ ] Add a second export format only after the first path is cleanly abstracted.
- [x] Export selected named output/subtree as well as default scene output.
- [x] Add overwrite policy/flag.
- [x] Include useful export statistics in CLI output.
- [x] Make deterministic export settings explicit.

### Preview/render export

- [ ] Add a headless or offscreen preview image path if renderer support allows it cleanly.
- [ ] Allow camera framing presets or automatic fit.
- [ ] Return/render diagnostic failure cleanly in headless use.
- [ ] Keep image rendering optional so geometry export remains usable on systems without it.

### Structured operation tooling

- [x] Add a command to apply a serialized `WorkspaceOp` transaction.
- [x] Add a dry-run mode that validates and prints the diff without committing.
- [x] Add a command to print history/recent revisions.
- [x] Add snapshot create/list/restore commands.

### Batch and automation ergonomics

- [x] Support JSON output for commands intended for agent/tool consumption.
- [x] Avoid prompts in machine mode.
- [x] Make stdout/stderr behavior predictable.
- [x] Add an example script that batch-exports multiple workspaces/variants.

### Tests

- [x] Add CLI integration tests for validate/inspect/export.
- [x] Add deterministic output tests where practical.
- [x] Add operation dry-run/apply tests.
- [x] Add non-zero exit-code tests for invalid workspaces.

## Completion criteria

- A remote agent or script can produce geometry from a workspace with no desktop session.
- CLI output is usable by both humans and automation.
- GUI features are not required to access canonical project behavior.

