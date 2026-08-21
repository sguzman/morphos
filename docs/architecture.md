# Morphos Architecture Contract

This document records the repository-level rules established during M00 so later milestones do
not need to reinvent bootstrap conventions.

## Core invariants

- A workspace is the canonical owner of durable structured state.
- TOML is the human-facing source representation rather than the only internal model.
- GUI, CLI, and AI surfaces must share project-owned APIs over the same workspace state.
- Geometry backend types stay behind Morphos-owned abstractions.
- Headless workflows are first-class, not a fallback.
- AI-authored mutations must eventually be structured, attributable, inspectable, and
  reversible.
- The GUI must not become a second hidden scene model.

## Milestone discipline

- Milestone files under `milestones/` are durable project state.
- Update milestone checkboxes only after implementation and verification both exist.
- Record blockers or newly discovered follow-up work in the relevant milestone file.
- Do not pull later-milestone scope forward without a concrete dependency reason.

## M00 boundaries

M00 establishes repository structure, documentation, tooling, and the minimal crate scaffold
required for M01. It does not implement workspace behavior, parsing, geometry, viewport logic,
history, AI APIs, or export pipelines.
