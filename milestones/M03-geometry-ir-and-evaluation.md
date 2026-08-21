# M03 — Geometry IR & Evaluation

## Goal

Create Morphos's project-owned geometry representation and evaluation pipeline, with third-party geometry libraries hidden behind backend interfaces.

A completed milestone means typed scene data can deterministically evaluate into preview/export geometry without coupling the rest of the app to one CSG implementation.

## Scope

Owns geometry IR, dependency evaluation, backend traits, mesh results, and evaluation caching.

## Subgoals

### Geometry IR

- [x] Define a backend-neutral `Shape`/geometry IR.
- [x] Represent primitives, transforms, and boolean composition in the IR.
- [x] Keep source node IDs attached through evaluation for diagnostics/provenance.
- [x] Define an evaluated mesh type or adapter owned by Morphos.
- [x] Define geometry bounds and basic statistics independent of the backend.

### Evaluation graph

- [x] Resolve scene references into an evaluable dependency graph.
- [x] Detect cycles before geometry evaluation.
- [x] Evaluate nodes in deterministic dependency order.
- [x] Allow evaluation of a selected output/subtree rather than always rebuilding the entire scene.
- [x] Add revision-aware caching so unchanged subtrees can be reused.
- [x] Invalidate only dependent nodes when a parameter/node changes.

### Backend abstraction

- [x] Define a `GeometryBackend` trait/interface owned by Morphos.
- [x] Implement one production backend using an existing Rust-accessible CSG library.
- [x] Convert backend output into the Morphos mesh representation.
- [x] Normalize backend errors into project-owned diagnostics.
- [x] Ensure backend-specific types do not leak into workspace, UI, CLI, or AI crates.

### Parameters and transforms

- [x] Resolve reusable parameters during evaluation.
- [x] Support transform composition with documented ordering.
- [x] Reject non-finite or otherwise invalid transform/parameter values.
- [x] Expose evaluated parameter values for diagnostics/UI.

### Tests and benchmarks

- [x] Add deterministic primitive evaluation tests.
- [x] Add union/difference/intersection tests.
- [x] Add transform-order tests.
- [x] Add dependency/cycle tests.
- [x] Add cache invalidation tests.
- [x] Add a small benchmark scene to track rebuild latency.

## Notes

- Production backend selection: M03 uses the Rust-native `boolmesh` kernel behind
  `GeometryBackend`. `manifold-csg` was investigated first for robustness, but its native source
  bootstrap stalled in this Windows environment during implementation verification. The final M03
  backend remains fully encapsulated behind Morphos-owned APIs so it can be swapped later without
  changing the scene schema.
- `plane` and `profile` are preserved in the IR but intentionally return typed
  `UnsupportedShape` evaluation errors until M13 defines richer profile/extrusion semantics.

## Completion criteria

- A scene can be evaluated headlessly into a valid mesh.
- Re-evaluating after a local edit does not require rebuilding unrelated subtrees.
- Swapping the geometry backend is possible without changing the scene schema or GUI architecture.

