# Morphos Milestones

Morphos is a Rust-native, declarative, live constructive-geometry workspace.

Its central contract is:

> A workspace is durable structured state. TOML is its human-facing source representation.  
> The GUI, CLI, geometry evaluator, and AI all operate on the same workspace model.

## How to use these milestone files

Each milestone is intentionally a **sovereign project target**. It should be possible to hand one milestone to an implementation agent and say: "Drive this milestone toward completion."

Within a milestone:

- Each unchecked item should be small enough for a lightweight coding agent to implement and verify.
- Prefer one checkbox per focused change/PR/commit.
- Do not silently expand scope into later milestones.
- Check an item only when its stated behavior exists and relevant tests pass.
- When a design decision changes a public contract, update the milestone file in the same change.
- Preserve headless operation. GUI-only shortcuts must not become the canonical implementation.
- Prefer deterministic, structured APIs over UI automation.
- Keep geometry backends behind project-owned interfaces.
- Keep AI changes inspectable, reversible, and attributable.

## Milestone order

The order below is the recommended dependency path, but each file owns a distinct aspect of the project.

1. [M00 — Project Bootstrap](M00-project-bootstrap.md)
2. [M01 — Workspace Core](M01-workspace-core.md)
3. [M02 — Declarative Scene Schema](M02-declarative-scene-schema.md)
4. [M03 — Geometry IR & Evaluation](M03-geometry-ir-and-evaluation.md)
5. [M04 — Live Viewport Shell](M04-live-viewport-shell.md)
6. [M05 — Reactive Editing Loop](M05-reactive-editing-loop.md)
7. [M06 — Scene Tree & Inspector](M06-scene-tree-and-inspector.md)
8. [M07 — Workspace Operations & History](M07-workspace-ops-and-history.md)
9. [M08 — Headless CLI & Export](M08-headless-cli-and-export.md)
10. [M09 — Validation & Diagnostics](M09-validation-and-diagnostics.md)
11. [M10 — AI Workspace API](M10-ai-workspace-api.md)
12. [M11 — AI Edit Workflow](M11-ai-edit-workflow.md)
13. [M12 — AI Presence & Provenance UI](M12-ai-presence-and-provenance-ui.md)
14. [M13 — Expressive Geometry Toolkit](M13-expressive-geometry-toolkit.md)
15. [M14 — Integration, Packaging & Release](M14-integration-packaging-release.md)

## Project-wide invariants

- TOML files must remain understandable and editable by humans.
- Invalid edits should produce diagnostics rather than destroy the last valid scene.
- A workspace must be evaluable without launching the GUI.
- Any geometry-producing action available in the GUI should have a non-GUI path.
- AI should use structured workspace APIs/operations whenever possible.
- AI-authored mutations must be distinguishable from user-authored mutations.
- Reverting AI changes must be cheap.
- Geometry backend types must not leak throughout the application.
- Saved workspaces should remain portable and versioned.

