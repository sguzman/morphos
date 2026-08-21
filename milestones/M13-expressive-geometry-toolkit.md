# M13 — Expressive Geometry Toolkit

## Goal

Expand Morphos from basic boolean primitives into a practical vocabulary for highly expressive procedural objects, stylized characters, architecture, and props.

A completed milestone means users can construct shapes with substantially more personality than rigid primitive unions alone while preserving the declarative/parameterized workflow.

## Scope

Owns higher-level geometry operators and reusable constructive vocabulary.

This milestone intentionally builds on the stable IR/backend architecture rather than replacing it.

## Subgoals

### 2D/profile construction

- [ ] Add reusable 2D profile/polyline/polygon representation.
- [ ] Add extrusion.
- [ ] Add revolve/lathe.
- [ ] Add basic profile transforms.
- [ ] Add profile import path from a simple textual or SVG-derived representation if it fits cleanly.

### Higher-level constructive operators

- [ ] Add rounding/bevel-like operator where backend capability permits.
- [ ] Add taper.
- [ ] Add twist.
- [ ] Add bend.
- [ ] Add elongation/stretch operators.
- [ ] Add mirror and repetition/array operators.
- [ ] Add hull/loft/sweep where backend support is practical.

### SDF/smooth composition path

- [ ] Define SDF-compatible shape/operator subset in the project-owned IR.
- [ ] Add smooth union.
- [ ] Add smooth subtraction/difference.
- [ ] Add smooth intersection if useful.
- [ ] Add a backend/evaluator path that can polygonize SDF results.
- [ ] Keep SDF resolution/tolerance explicit and deterministic.
- [ ] Allow a scene to combine exact/mesh CSG and SDF-derived results through documented conversion boundaries.

### Reusable shape modules

- [ ] Add parameterized reusable modules/components.
- [ ] Allow module instantiation with overridden parameters.
- [ ] Add a small standard library of reusable constructive forms.
- [ ] Include at least one expressive humanoid/creature example.
- [ ] Include at least one architectural/prop example.
- [ ] Make examples editable live through exposed parameters.

### Parameter-space exploration

- [ ] Add parameter metadata for range, step, labels, and grouping.
- [ ] Add randomize-within-range action.
- [ ] Add deterministic seed support.
- [ ] Add variant generation from parameter sets.
- [ ] Add headless batch export of variants.

### Tests and demos

- [ ] Add regression scenes for every expressive operator.
- [ ] Add geometry/backend capability tests.
- [ ] Add at least one "personality through primitives" showcase workspace.
- [ ] Add at least one smooth organic/SDF showcase workspace.
- [ ] Track rebuild latency for showcase scenes.

## Completion criteria

- Morphos can create recognizably stylized, parameterized forms rather than only CAD-like primitive assemblies.
- Expressive operators remain available from TOML, GUI, CLI, and AI workflows.
- The scene language remains declarative and inspectable.

