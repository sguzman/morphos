# M03 — Geometry IR & Evaluation

## Goal

Create Morphos's project-owned geometry representation and evaluation pipeline, with third-party geometry libraries hidden behind backend interfaces.

A completed milestone means typed scene data can deterministically evaluate into preview/export geometry without coupling the rest of the app to one CSG implementation.

## Scope

Owns geometry IR, dependency evaluation, backend traits, mesh results, and evaluation caching.

## Subgoals

### Geometry IR

- [ ] Define a backend-neutral `Shape`/geometry IR.
- [ ] Represent primitives, transforms, and boolean composition in the IR.
- [ ] Keep source node IDs attached through evaluation for diagnostics/provenance.
- [ ] Define an evaluated mesh type or adapter owned by Morphos.
- [ ] Define geometry bounds and basic statistics independent of the backend.

### Evaluation graph

- [ ] Resolve scene references into an evaluable dependency graph.
- [ ] Detect cycles before geometry evaluation.
- [ ] Evaluate nodes in deterministic dependency order.
- [ ] Allow evaluation of a selected output/subtree rather than always rebuilding the entire scene.
- [ ] Add revision-aware caching so unchanged subtrees can be reused.
- [ ] Invalidate only dependent nodes when a parameter/node changes.

### Backend abstraction

- [ ] Define a `GeometryBackend` trait/interface owned by Morphos.
- [ ] Implement one production backend using an existing Rust-accessible CSG library.
- [ ] Convert backend output into the Morphos mesh representation.
- [ ] Normalize backend errors into project-owned diagnostics.
- [ ] Ensure backend-specific types do not leak into workspace, UI, CLI, or AI crates.

### Parameters and transforms

- [ ] Resolve reusable parameters during evaluation.
- [ ] Support transform composition with documented ordering.
- [ ] Reject non-finite or otherwise invalid transform/parameter values.
- [ ] Expose evaluated parameter values for diagnostics/UI.

### Tests and benchmarks

- [ ] Add deterministic primitive evaluation tests.
- [ ] Add union/difference/intersection tests.
- [ ] Add transform-order tests.
- [ ] Add dependency/cycle tests.
- [ ] Add cache invalidation tests.
- [ ] Add a small benchmark scene to track rebuild latency.

## Completion criteria

- A scene can be evaluated headlessly into a valid mesh.
- Re-evaluating after a local edit does not require rebuilding unrelated subtrees.
- Swapping the geometry backend is possible without changing the scene schema or GUI architecture.

