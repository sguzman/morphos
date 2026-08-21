# Morphos Geometry Evaluation (M03)

`geom_geometry` is Morphos's headless geometry-evaluation crate.

It depends on `geom_scene`, resolves declarative scene semantics into a backend-neutral geometry
graph, and evaluates that graph into Morphos-owned mesh, bounds, and statistics types without
leaking backend-specific types into higher layers.

## Dependency direction

The intended ownership seam after M03 is:

`geom_scene` -> `geom_geometry` -> production CSG backend

`geom_workspace` remains independent of geometry evaluation and continues to own durable source
state only.

## Geometry IR

The geometry layer owns a resolved IR centered on:

- `GeometryGraph`
- `GeometryNode`
- `GeometryOperation`
- `PrimitiveShape`
- `ResolvedTransform`
- `ResolvedParameter`

This IR is backend-neutral and uses concrete resolved numeric values rather than TOML syntax.
`GeometryGraph::from_scene()` resolves:

- named parameter references into finite `f64` values
- primitive dimensions into concrete values
- transforms into concrete translation, Euler XYZ degrees, and positive scale
- composition children into explicit graph edges

Plane and profile nodes remain representable in the IR but return a typed
`GeometryErrorKind::UnsupportedShape` during mesh evaluation. M03 intentionally does not invent
M13 extrusion/profile semantics early.

## Backend boundary

`GeometryBackend` is the Morphos-owned seam around the production geometry kernel.

Its responsibilities are intentionally narrow:

- build primitive solids
- apply node-local transforms
- perform union/difference/intersection
- convert the final backend solid into Morphos's `Mesh`

The current production backend is `BoolmeshBackend`, which uses the `boolmesh` crate internally.
The public Morphos API only exposes Morphos-owned types such as `Mesh`, `Bounds`,
`GeometryStats`, `EvaluatedGeometry`, and `GeometryError`.

This keeps future GUI, CLI, export, and AI systems backend-neutral.

## Mesh, bounds, and statistics

`Mesh` is Morphos-owned and stores:

- `positions: Vec<[f64; 3]>`
- `triangle_indices: Vec<[u32; 3]>`

Mesh validation rejects:

- non-finite vertex coordinates
- triangle indices outside the vertex range

`Bounds` is an explicit enum:

- `Bounds::Empty`
- `Bounds::Finite { min, max }`

It also exposes convenience accessors for center and size.

`GeometryStats` currently reports:

- vertex count
- triangle count
- evaluated node count
- cache hits
- cache misses

## Parameter resolution

The evaluator resolves `ScalarExpr` values before backend work begins:

- `Literal(x)` -> `x`
- `Parameter(ref)` -> resolved parameter value from `SceneDocument.parameters()`

Resolved values are returned in `EvaluatedGeometry.resolved_parameters` so later diagnostics or UI
layers can inspect them directly without reopening TOML.

## Dependency graph and deterministic ordering

`GeometryGraph` stores explicit node dependencies and reverse dependents.

Evaluation order is deterministic:

- selected output traversal is depth-first
- child dependencies are visited in declared scene order
- each node is emitted post-order after its dependencies

This gives stable evaluation order for testing, caching, and future diagnostics.

Cycle protection remains present in the geometry layer even though `geom_scene` already validates
cycles earlier. A cycle returns `GeometryErrorKind::DependencyCycle` before backend work begins.

## Transform semantics

Morphos honors M02's transform contract exactly:

1. scale
2. rotate using Euler XYZ
3. translate

Scene rotations are stored in degrees. The production backend receives radians internally where
required, but the Morphos-owned transform contract remains degrees-in / scale-then-rotate-then-
translate.

Tests use asymmetric boxes and non-origin transforms so incorrect ordering does not pass
accidentally.

## Composition semantics

Supported composition operators are:

- `union`
- `difference`
- `intersection`

Difference preserves declared child ordering and is evaluated left-to-right:

`(((child0 - child1) - child2) - ...)`

This avoids silently changing scene meaning to match a backend shortcut.

## Evaluator and cache lifecycle

`GeometryEvaluator<B>` owns reusable backend state and cached node solids.

Each evaluation:

1. rebuilds the resolved `GeometryGraph` from the current `SceneDocument`
2. computes deterministic dependency order for the requested output node
3. computes semantic fingerprints from:
   primitive/operator semantics
   resolved numeric inputs
   transform values
   parameter dependencies
   transitive child fingerprints
4. reuses cached solids when the fingerprint matches
5. rebuilds only changed nodes and their dependents
6. prunes cache entries for nodes no longer present in the scene

Selected-node evaluation only traverses the requested subtree, so unrelated branches are not
visited or rebuilt.

## Cache invalidation rules

Changing a parameter or node only invalidates:

- the directly affected node
- its transitive dependents

Unrelated branches remain reusable. Instrumented backend tests prove this by checking which
backend operations are called after a local edit rather than only inspecting cache lengths.

## Geometry revision semantics

`GeometryEvaluator` tracks an internal monotonic `evaluation_revision`.

This revision is evaluator-local state for cache lifecycle and result metadata. It is not coupled
to M01 workspace revisions and does not introduce a dependency on `geom_workspace`.

## Headless example

```rust
let scene = geom_scene::parse_scene(source_text)?;
let mut evaluator = geom_geometry::GeometryEvaluator::new(
    geom_geometry::BoolmeshBackend::new(),
);
let evaluated = evaluator.evaluate_root(&scene)?;

assert!(evaluated.stats.triangle_count > 0);
assert!(!evaluated.mesh.is_empty());
```

This path requires no GUI, renderer, Bevy, or graphics context.
