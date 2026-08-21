# M08 — Headless CLI & Export

## Goal

Make every essential geometry workflow usable without the desktop application.

A completed milestone means Morphos can validate, inspect, evaluate, render/export, and apply structured edits from the command line.

## Scope

Owns CLI commands, non-GUI execution, deterministic export, and batch-friendly exit behavior.

## Subgoals

### CLI foundation

- [ ] Create a `morphos` CLI binary separate from the desktop app binary.
- [ ] Add `morphos validate <workspace>`.
- [ ] Add `morphos inspect <workspace>` with machine-readable output option.
- [ ] Add `morphos eval <workspace>` or equivalent build command.
- [ ] Define stable exit codes for success, source error, geometry error, and I/O error.
- [ ] Ensure commands do not initialize windowing/GUI systems.

### Mesh export

- [ ] Add at least one practical mesh export format.
- [ ] Add a second export format only after the first path is cleanly abstracted.
- [ ] Export selected named output/subtree as well as default scene output.
- [ ] Add overwrite policy/flag.
- [ ] Include useful export statistics in CLI output.
- [ ] Make deterministic export settings explicit.

### Preview/render export

- [ ] Add a headless or offscreen preview image path if renderer support allows it cleanly.
- [ ] Allow camera framing presets or automatic fit.
- [ ] Return/render diagnostic failure cleanly in headless use.
- [ ] Keep image rendering optional so geometry export remains usable on systems without it.

### Structured operation tooling

- [ ] Add a command to apply a serialized `WorkspaceOp` transaction.
- [ ] Add a dry-run mode that validates and prints the diff without committing.
- [ ] Add a command to print history/recent revisions.
- [ ] Add snapshot create/list/restore commands.

### Batch and automation ergonomics

- [ ] Support JSON output for commands intended for agent/tool consumption.
- [ ] Avoid prompts in machine mode.
- [ ] Make stdout/stderr behavior predictable.
- [ ] Add an example script that batch-exports multiple workspaces/variants.

### Tests

- [ ] Add CLI integration tests for validate/inspect/export.
- [ ] Add deterministic output tests where practical.
- [ ] Add operation dry-run/apply tests.
- [ ] Add non-zero exit-code tests for invalid workspaces.

## Completion criteria

- A remote agent or script can produce geometry from a workspace with no desktop session.
- CLI output is usable by both humans and automation.
- GUI features are not required to access canonical project behavior.

