//! Backend-neutral geometry IR and deterministic evaluation for Morphos.
//!
//! `geom_geometry` consumes validated `geom_scene::SceneDocument` values and
//! turns them into Morphos-owned mesh/bounds/statistics results through a
//! replaceable backend boundary.
//!
//! M03 conventions:
//!
//! - parameter expressions resolve to concrete finite `f64` values
//! - local transforms apply as scale, then Euler XYZ rotation in degrees, then
//!   translation
//! - dependency traversal is deterministic and follows declared child order
//! - third-party backend types remain internal to the backend implementation
//! - cache entries reuse unchanged subtrees and invalidate only dependents

use geom_diagnostics::{Diagnostic, DiagnosticCode, DiagnosticReport};
use geom_scene::{
    CompositionNode, NodeId, NodeKind, ParamId, ParameterDefinition, ScalarExpr, SceneDocument,
    Transform, Vector3Expr,
};
use glam::{DMat4, DVec3};
use indexmap::{IndexMap, IndexSet};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::fmt;
use std::hash::{Hash, Hasher};

/// A backend-neutral resolved geometry graph built from a scene.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryGraph {
    root: NodeId,
    parameters: IndexMap<ParamId, ResolvedParameter>,
    nodes: IndexMap<NodeId, GeometryNode>,
    dependents: HashMap<NodeId, Vec<NodeId>>,
}

impl GeometryGraph {
    /// Builds a fully resolved graph from a validated scene document.
    pub fn from_scene(scene: &SceneDocument) -> Result<Self, GeometryError> {
        let parameters = resolve_parameters(scene.parameters())?;
        let mut nodes = IndexMap::new();

        for (node_id, node) in scene.nodes() {
            let mut parameter_dependencies = Vec::new();
            let operation = resolve_node_operation(
                node_id,
                node.kind(),
                &parameters,
                &mut parameter_dependencies,
            )?;
            let transform = resolve_transform(node_id, node.transform(), &parameters)?;
            let geometry_dependencies = match node.kind() {
                NodeKind::Union(composition)
                | NodeKind::Difference(composition)
                | NodeKind::Intersection(composition) => composition
                    .children
                    .iter()
                    .map(|child| child.target().clone())
                    .collect(),
                _ => Vec::new(),
            };

            nodes.insert(
                node_id.clone(),
                GeometryNode {
                    source_id: node_id.clone(),
                    operation,
                    transform,
                    geometry_dependencies,
                    parameter_dependencies,
                },
            );
        }

        let mut dependents: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for (node_id, node) in &nodes {
            for dependency in &node.geometry_dependencies {
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .push(node_id.clone());
            }
        }
        for dependent_list in dependents.values_mut() {
            dependent_list.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        }

        let graph = Self {
            root: scene.root().clone(),
            parameters,
            nodes,
            dependents,
        };

        graph.dependency_order_for(scene.root())?;
        Ok(graph)
    }

    /// Returns the root output node for the graph.
    pub fn root(&self) -> &NodeId {
        &self.root
    }

    /// Returns resolved parameter values.
    pub fn parameters(&self) -> &IndexMap<ParamId, ResolvedParameter> {
        &self.parameters
    }

    /// Returns resolved geometry nodes keyed by source `NodeId`.
    pub fn nodes(&self) -> &IndexMap<NodeId, GeometryNode> {
        &self.nodes
    }

    /// Returns reverse dependency edges for invalidation/inspection.
    pub fn dependents(&self) -> &HashMap<NodeId, Vec<NodeId>> {
        &self.dependents
    }

    fn dependency_order_for(&self, requested: &NodeId) -> Result<Vec<NodeId>, GeometryError> {
        if !self.nodes.contains_key(requested) {
            return Err(GeometryError::new(
                GeometryErrorKind::UnknownOutput {
                    requested: requested.clone(),
                },
                Some(requested.clone()),
            ));
        }

        let mut order = Vec::new();
        let mut visiting = Vec::new();
        let mut state: HashMap<NodeId, VisitState> = HashMap::new();
        self.visit_node(requested, &mut state, &mut visiting, &mut order)?;
        Ok(order)
    }

    fn visit_node(
        &self,
        node_id: &NodeId,
        state: &mut HashMap<NodeId, VisitState>,
        visiting: &mut Vec<NodeId>,
        order: &mut Vec<NodeId>,
    ) -> Result<(), GeometryError> {
        match state.get(node_id).copied() {
            Some(VisitState::Visited) => return Ok(()),
            Some(VisitState::Visiting) => {
                let cycle_start = visiting
                    .iter()
                    .position(|entry| entry == node_id)
                    .unwrap_or(0);
                let mut cycle = visiting[cycle_start..].to_vec();
                cycle.push(node_id.clone());
                return Err(GeometryError::new(
                    GeometryErrorKind::DependencyCycle { cycle },
                    Some(node_id.clone()),
                ));
            }
            None => {}
        }

        let node = self.nodes.get(node_id).ok_or_else(|| {
            GeometryError::new(
                GeometryErrorKind::UnknownOutput {
                    requested: node_id.clone(),
                },
                Some(node_id.clone()),
            )
        })?;

        state.insert(node_id.clone(), VisitState::Visiting);
        visiting.push(node_id.clone());
        for dependency in &node.geometry_dependencies {
            self.visit_node(dependency, state, visiting, order)?;
        }
        visiting.pop();
        state.insert(node_id.clone(), VisitState::Visited);
        order.push(node_id.clone());
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Visited,
}

/// A resolved scene parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedParameter {
    id: ParamId,
    value: f64,
}

impl ResolvedParameter {
    /// Returns the source parameter ID.
    pub fn id(&self) -> &ParamId {
        &self.id
    }

    /// Returns the resolved finite scalar value.
    pub fn value(&self) -> f64 {
        self.value
    }
}

/// One backend-neutral node in the resolved geometry graph.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryNode {
    source_id: NodeId,
    operation: GeometryOperation,
    transform: ResolvedTransform,
    geometry_dependencies: Vec<NodeId>,
    parameter_dependencies: Vec<ParamId>,
}

impl GeometryNode {
    /// Returns the source `NodeId`.
    pub fn source_id(&self) -> &NodeId {
        &self.source_id
    }

    /// Returns the resolved operation.
    pub fn operation(&self) -> &GeometryOperation {
        &self.operation
    }

    /// Returns the resolved transform.
    pub fn transform(&self) -> &ResolvedTransform {
        &self.transform
    }

    /// Returns direct geometry dependencies in declared child order.
    pub fn geometry_dependencies(&self) -> &[NodeId] {
        &self.geometry_dependencies
    }

    /// Returns directly referenced parameters.
    pub fn parameter_dependencies(&self) -> &[ParamId] {
        &self.parameter_dependencies
    }
}

/// A resolved node operation independent of TOML syntax.
#[derive(Debug, Clone, PartialEq)]
pub enum GeometryOperation {
    Primitive(PrimitiveShape),
    Union { children: Vec<NodeId> },
    Difference { children: Vec<NodeId> },
    Intersection { children: Vec<NodeId> },
}

/// A backend-neutral resolved primitive.
#[derive(Debug, Clone, PartialEq)]
pub enum PrimitiveShape {
    Box { size: [f64; 3] },
    Sphere { radius: f64 },
    Cylinder { radius: f64, height: f64 },
    Capsule { radius: f64, height: f64 },
    Plane { width: f64, depth: f64 },
    Profile { width: f64, height: f64 },
}

/// A concrete transform resolved from scene expressions.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTransform {
    translation: [f64; 3],
    rotation_deg: [f64; 3],
    scale: [f64; 3],
}

impl ResolvedTransform {
    /// Returns the translation vector.
    pub fn translation(&self) -> [f64; 3] {
        self.translation
    }

    /// Returns Euler XYZ rotation in degrees.
    pub fn rotation_deg(&self) -> [f64; 3] {
        self.rotation_deg
    }

    /// Returns positive XYZ scale values.
    pub fn scale(&self) -> [f64; 3] {
        self.scale
    }

    /// Returns the local transform matrix using Morphos's scale -> rotate -> translate order.
    pub fn matrix(&self) -> DMat4 {
        let scale = DMat4::from_scale(DVec3::from_array(self.scale));
        let rotation = DMat4::from_euler(
            glam::EulerRot::XYZ,
            self.rotation_deg[0].to_radians(),
            self.rotation_deg[1].to_radians(),
            self.rotation_deg[2].to_radians(),
        );
        let translation = DMat4::from_translation(DVec3::from_array(self.translation));
        translation * rotation * scale
    }
}

/// Morphos-owned mesh output.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    positions: Vec<[f64; 3]>,
    triangle_indices: Vec<[u32; 3]>,
}

impl Mesh {
    /// Creates a validated mesh from positions and triangle indices.
    pub fn new(
        positions: Vec<[f64; 3]>,
        triangle_indices: Vec<[u32; 3]>,
    ) -> Result<Self, GeometryError> {
        validate_mesh(&positions, &triangle_indices)?;
        Ok(Self {
            positions,
            triangle_indices,
        })
    }

    /// Returns vertex positions in scene-space `f64`.
    pub fn positions(&self) -> &[[f64; 3]] {
        &self.positions
    }

    /// Returns triangle indices grouped by face.
    pub fn triangle_indices(&self) -> &[[u32; 3]] {
        &self.triangle_indices
    }

    /// Returns whether the mesh has no triangles.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty() || self.triangle_indices.is_empty()
    }
}

/// Axis-aligned bounds for evaluated geometry.
#[derive(Debug, Clone, PartialEq)]
pub enum Bounds {
    Empty,
    Finite { min: [f64; 3], max: [f64; 3] },
}

impl Bounds {
    /// Computes bounds from mesh positions.
    pub fn from_positions(positions: &[[f64; 3]]) -> Result<Self, GeometryError> {
        if positions.is_empty() {
            return Ok(Self::Empty);
        }

        let mut min = positions[0];
        let mut max = positions[0];
        for position in positions {
            for axis in 0..3 {
                ensure_finite(
                    *position.get(axis).unwrap_or(&f64::NAN),
                    "mesh position",
                    None,
                )?;
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
        Ok(Self::Finite { min, max })
    }

    /// Returns the minimum corner if the bounds are non-empty.
    pub fn min(&self) -> Option<[f64; 3]> {
        match self {
            Self::Empty => None,
            Self::Finite { min, .. } => Some(*min),
        }
    }

    /// Returns the maximum corner if the bounds are non-empty.
    pub fn max(&self) -> Option<[f64; 3]> {
        match self {
            Self::Empty => None,
            Self::Finite { max, .. } => Some(*max),
        }
    }

    /// Returns the center if the bounds are non-empty.
    pub fn center(&self) -> Option<[f64; 3]> {
        match self {
            Self::Empty => None,
            Self::Finite { min, max } => Some([
                (min[0] + max[0]) * 0.5,
                (min[1] + max[1]) * 0.5,
                (min[2] + max[2]) * 0.5,
            ]),
        }
    }

    /// Returns the size/extent if the bounds are non-empty.
    pub fn size(&self) -> Option<[f64; 3]> {
        match self {
            Self::Empty => None,
            Self::Finite { min, max } => Some([max[0] - min[0], max[1] - min[1], max[2] - min[2]]),
        }
    }
}

/// Basic backend-neutral evaluation statistics.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryStats {
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub evaluated_node_count: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
}

/// A successful evaluated geometry result.
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedGeometry {
    pub requested_output: NodeId,
    pub mesh: Mesh,
    pub bounds: Bounds,
    pub stats: GeometryStats,
    pub resolved_parameters: IndexMap<ParamId, ResolvedParameter>,
    pub participating_node_ids: Vec<NodeId>,
    pub evaluation_revision: u64,
}

/// The backend trait that isolates third-party geometry kernels from the rest of Morphos.
pub trait GeometryBackend {
    /// Opaque backend solid handle stored in the evaluator cache.
    type Solid: Clone;

    /// Backend-private error type.
    type Error: Error + Send + Sync + 'static;

    fn build_primitive(
        &mut self,
        source: &NodeId,
        primitive: &PrimitiveShape,
    ) -> Result<Self::Solid, Self::Error>;

    fn apply_transform(
        &mut self,
        source: &NodeId,
        solid: &Self::Solid,
        transform: &ResolvedTransform,
    ) -> Result<Self::Solid, Self::Error>;

    fn union(
        &mut self,
        source: &NodeId,
        solids: &[Self::Solid],
    ) -> Result<Self::Solid, Self::Error>;

    fn difference(
        &mut self,
        source: &NodeId,
        solids: &[Self::Solid],
    ) -> Result<Self::Solid, Self::Error>;

    fn intersection(
        &mut self,
        source: &NodeId,
        solids: &[Self::Solid],
    ) -> Result<Self::Solid, Self::Error>;

    fn to_mesh(&mut self, source: &NodeId, solid: &Self::Solid) -> Result<Mesh, Self::Error>;
}

/// Revision-aware reusable evaluator/cache owner.
#[derive(Debug)]
pub struct GeometryEvaluator<B: GeometryBackend> {
    backend: B,
    cache: HashMap<NodeId, CachedNode<B::Solid>>,
    evaluation_revision: u64,
}

impl<B: GeometryBackend> GeometryEvaluator<B> {
    /// Creates a new evaluator with empty cache state.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            cache: HashMap::new(),
            evaluation_revision: 0,
        }
    }

    /// Returns the backend instance.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Returns the backend instance mutably.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Evaluates the scene's declared root output.
    pub fn evaluate_root(
        &mut self,
        scene: &SceneDocument,
    ) -> Result<EvaluatedGeometry, GeometryError> {
        let root = scene.root().clone();
        self.evaluate_node(scene, &root)
    }

    /// Evaluates one selected output node and its transitive dependencies only.
    pub fn evaluate_node(
        &mut self,
        scene: &SceneDocument,
        requested: &NodeId,
    ) -> Result<EvaluatedGeometry, GeometryError> {
        let graph = GeometryGraph::from_scene(scene)?;
        let order = graph.dependency_order_for(requested)?;
        self.cache
            .retain(|node_id, _| graph.nodes().contains_key(node_id));

        let mut solids: HashMap<NodeId, B::Solid> = HashMap::new();
        let mut fingerprints: HashMap<NodeId, u64> = HashMap::new();
        let mut cache_hits = 0usize;
        let mut cache_misses = 0usize;

        for node_id in &order {
            let node = graph.nodes().get(node_id).expect("ordered node exists");
            let local_fingerprint = semantic_fingerprint(node);
            let mut fingerprint = local_fingerprint;
            for dependency in &node.geometry_dependencies {
                let dependency_fingerprint = *fingerprints
                    .get(dependency)
                    .expect("dependency fingerprint");
                fingerprint = combine_fingerprint(fingerprint, dependency_fingerprint);
            }

            if let Some(entry) = self.cache.get(node_id)
                && entry.fingerprint == fingerprint
            {
                cache_hits += 1;
                solids.insert(node_id.clone(), entry.solid.clone());
                fingerprints.insert(node_id.clone(), fingerprint);
                continue;
            }

            let solid = evaluate_node_with_backend(&mut self.backend, node, &solids)?;
            self.cache.insert(
                node_id.clone(),
                CachedNode {
                    fingerprint,
                    solid: solid.clone(),
                },
            );
            solids.insert(node_id.clone(), solid);
            fingerprints.insert(node_id.clone(), fingerprint);
            cache_misses += 1;
        }

        let requested_solid = solids.get(requested).ok_or_else(|| {
            GeometryError::new(
                GeometryErrorKind::UnknownOutput {
                    requested: requested.clone(),
                },
                Some(requested.clone()),
            )
        })?;
        let mesh = self
            .backend
            .to_mesh(requested, requested_solid)
            .map_err(|error| backend_error(BackendStage::MeshConversion, requested, error))?;
        let bounds = Bounds::from_positions(mesh.positions())?;
        let stats = GeometryStats {
            vertex_count: mesh.positions().len(),
            triangle_count: mesh.triangle_indices().len(),
            evaluated_node_count: order.len(),
            cache_hits,
            cache_misses,
        };

        self.evaluation_revision += 1;

        Ok(EvaluatedGeometry {
            requested_output: requested.clone(),
            mesh,
            bounds,
            stats,
            resolved_parameters: graph.parameters().clone(),
            participating_node_ids: order,
            evaluation_revision: self.evaluation_revision,
        })
    }
}

#[derive(Debug, Clone)]
struct CachedNode<Solid> {
    fingerprint: u64,
    solid: Solid,
}

/// Production backend backed by the Rust-native `boolmesh` kernel.
#[derive(Debug, Clone)]
pub struct BoolmeshBackend {
    circle_segments: usize,
    sphere_stacks: usize,
}

impl Default for BoolmeshBackend {
    fn default() -> Self {
        Self {
            circle_segments: 32,
            sphere_stacks: 16,
        }
    }
}

impl BoolmeshBackend {
    /// Creates a backend with the default tessellation quality.
    pub fn new() -> Self {
        Self::default()
    }
}

impl GeometryBackend for BoolmeshBackend {
    type Solid = boolmesh::prelude::Manifold;
    type Error = BoolmeshBackendError;

    fn build_primitive(
        &mut self,
        source: &NodeId,
        primitive: &PrimitiveShape,
    ) -> Result<Self::Solid, Self::Error> {
        use boolmesh::prelude::{
            OpType, compute_boolean, generate_cube, generate_cylinder, generate_uv_sphere,
        };

        match primitive {
            PrimitiveShape::Box { size } => {
                let mut mesh = generate_cube().map_err(BoolmeshBackendError)?;
                mesh.scale(size[0], size[1], size[2]);
                Ok(mesh)
            }
            PrimitiveShape::Sphere { radius } => {
                let mut mesh = generate_uv_sphere(self.circle_segments, self.sphere_stacks)
                    .map_err(BoolmeshBackendError)?;
                mesh.scale(*radius, *radius, *radius);
                Ok(mesh)
            }
            PrimitiveShape::Cylinder { radius, height } => {
                generate_cylinder(*radius, *height, self.circle_segments, 1)
                    .map_err(BoolmeshBackendError)
            }
            PrimitiveShape::Capsule { radius, height } => {
                if *height < radius * 2.0 {
                    return Err(BoolmeshBackendError(format!(
                        "capsule `{}` height {} must be at least twice the radius {}",
                        source, height, radius
                    )));
                }

                let shaft_height = (*height - radius * 2.0).max(0.0);
                let cylinder = if shaft_height == 0.0 {
                    let mut mesh = generate_uv_sphere(self.circle_segments, self.sphere_stacks)
                        .map_err(BoolmeshBackendError)?;
                    mesh.scale(*radius, *radius, *radius);
                    mesh
                } else {
                    generate_cylinder(*radius, shaft_height, self.circle_segments, 1)
                        .map_err(BoolmeshBackendError)?
                };
                let mut sphere = generate_uv_sphere(self.circle_segments, self.sphere_stacks)
                    .map_err(BoolmeshBackendError)?;
                sphere.scale(*radius, *radius, *radius);

                let mut top = sphere.clone();
                top.translate(0.0, shaft_height * 0.5, 0.0);
                let mut bottom = sphere;
                bottom.translate(0.0, -shaft_height * 0.5, 0.0);
                let with_top =
                    compute_boolean(&cylinder, &top, OpType::Add).map_err(BoolmeshBackendError)?;
                compute_boolean(&with_top, &bottom, OpType::Add).map_err(BoolmeshBackendError)
            }
            PrimitiveShape::Plane { .. } | PrimitiveShape::Profile { .. } => {
                Err(BoolmeshBackendError(format!(
                    "unsupported placeholder primitive at node `{source}`"
                )))
            }
        }
    }

    fn apply_transform(
        &mut self,
        _source: &NodeId,
        solid: &Self::Solid,
        transform: &ResolvedTransform,
    ) -> Result<Self::Solid, Self::Error> {
        let mut transformed = solid.clone();
        transformed.scale(transform.scale[0], transform.scale[1], transform.scale[2]);
        transformed.rotate(
            transform.rotation_deg[0].to_radians(),
            transform.rotation_deg[1].to_radians(),
            transform.rotation_deg[2].to_radians(),
        );
        transformed.translate(
            transform.translation[0],
            transform.translation[1],
            transform.translation[2],
        );
        Ok(transformed)
    }

    fn union(
        &mut self,
        source: &NodeId,
        solids: &[Self::Solid],
    ) -> Result<Self::Solid, Self::Error> {
        use boolmesh::prelude::{OpType, compute_boolean};

        let mut iter = solids.iter();
        let Some(first) = iter.next() else {
            return Err(BoolmeshBackendError(format!(
                "union `{source}` requires at least one child"
            )));
        };
        let mut result = first.clone();
        for solid in iter {
            result = compute_boolean(&result, solid, OpType::Add).map_err(BoolmeshBackendError)?;
        }
        Ok(result)
    }

    fn difference(
        &mut self,
        source: &NodeId,
        solids: &[Self::Solid],
    ) -> Result<Self::Solid, Self::Error> {
        use boolmesh::prelude::{OpType, compute_boolean};

        let mut iter = solids.iter();
        let Some(first) = iter.next() else {
            return Err(BoolmeshBackendError(format!(
                "difference `{source}` requires at least one child"
            )));
        };
        let mut result = first.clone();
        for solid in iter {
            result =
                compute_boolean(&result, solid, OpType::Subtract).map_err(BoolmeshBackendError)?;
        }
        Ok(result)
    }

    fn intersection(
        &mut self,
        source: &NodeId,
        solids: &[Self::Solid],
    ) -> Result<Self::Solid, Self::Error> {
        use boolmesh::prelude::{OpType, compute_boolean};

        let mut iter = solids.iter();
        let Some(first) = iter.next() else {
            return Err(BoolmeshBackendError(format!(
                "intersection `{source}` requires at least one child"
            )));
        };
        let mut result = first.clone();
        for solid in iter {
            result =
                compute_boolean(&result, solid, OpType::Intersect).map_err(BoolmeshBackendError)?;
        }
        Ok(result)
    }

    fn to_mesh(&mut self, source: &NodeId, solid: &Self::Solid) -> Result<Mesh, Self::Error> {
        let positions = solid
            .ps
            .iter()
            .map(|position| [position.x, position.y, position.z])
            .collect::<Vec<_>>();
        let triangles = solid
            .get_indices()
            .iter()
            .map(|triangle| {
                Ok([
                    u32::try_from(triangle.x).map_err(|_| {
                        BoolmeshBackendError(format!("vertex index exceeds u32 for `{source}`"))
                    })?,
                    u32::try_from(triangle.y).map_err(|_| {
                        BoolmeshBackendError(format!("vertex index exceeds u32 for `{source}`"))
                    })?,
                    u32::try_from(triangle.z).map_err(|_| {
                        BoolmeshBackendError(format!("vertex index exceeds u32 for `{source}`"))
                    })?,
                ])
            })
            .collect::<Result<Vec<_>, _>>()?;

        Mesh::new(positions, triangles).map_err(|error| BoolmeshBackendError(error.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct BoolmeshBackendError(String);

impl fmt::Display for BoolmeshBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Error for BoolmeshBackendError {}

/// Structured geometry/evaluation error.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryError {
    kind: GeometryErrorKind,
    source_node: Option<NodeId>,
}

impl GeometryError {
    fn new(kind: GeometryErrorKind, source_node: Option<NodeId>) -> Self {
        Self { kind, source_node }
    }

    /// Returns the structured error kind.
    pub fn kind(&self) -> &GeometryErrorKind {
        &self.kind
    }

    /// Returns the most relevant source node when available.
    pub fn source_node(&self) -> Option<&NodeId> {
        self.source_node.as_ref()
    }
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source_node {
            Some(node) => write!(f, "{} (node `{node}`)", self.kind),
            None => self.kind.fmt(f),
        }
    }
}

impl Error for GeometryError {}

/// Error categories normalized by Morphos.
#[derive(Debug, Clone, PartialEq)]
pub enum GeometryErrorKind {
    UnknownOutput {
        requested: NodeId,
    },
    UnknownParameter {
        parameter: ParamId,
    },
    InvalidParameterValue {
        parameter: ParamId,
        reason: &'static str,
    },
    InvalidTransform {
        reason: &'static str,
    },
    InvalidPrimitive {
        reason: &'static str,
    },
    InvalidComposition {
        operator: &'static str,
        child_count: usize,
    },
    DependencyCycle {
        cycle: Vec<NodeId>,
    },
    UnsupportedShape {
        shape: &'static str,
    },
    InvalidMesh {
        reason: &'static str,
    },
    BackendFailure {
        stage: BackendStage,
        message: String,
    },
}

impl fmt::Display for GeometryErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOutput { requested } => {
                write!(f, "unknown output node `{requested}`")
            }
            Self::UnknownParameter { parameter } => {
                write!(f, "unknown parameter `{parameter}`")
            }
            Self::InvalidParameterValue { parameter, reason } => {
                write!(f, "invalid parameter `{parameter}`: {reason}")
            }
            Self::InvalidTransform { reason } => write!(f, "invalid transform: {reason}"),
            Self::InvalidPrimitive { reason } => write!(f, "invalid primitive: {reason}"),
            Self::InvalidComposition {
                operator,
                child_count,
            } => write!(
                f,
                "invalid `{operator}` composition: expected at least 2 children, found {child_count}"
            ),
            Self::DependencyCycle { cycle } => {
                write!(
                    f,
                    "dependency cycle detected: {}",
                    cycle
                        .iter()
                        .map(NodeId::as_str)
                        .collect::<Vec<_>>()
                        .join(" -> ")
                )
            }
            Self::UnsupportedShape { shape } => write!(f, "unsupported shape `{shape}`"),
            Self::InvalidMesh { reason } => write!(f, "invalid mesh: {reason}"),
            Self::BackendFailure { stage, message } => {
                write!(f, "backend {stage} failed: {message}")
            }
        }
    }
}

pub fn diagnostic_from_geometry_error(error: &GeometryError) -> Diagnostic {
    let mut diagnostic = match error.kind() {
        GeometryErrorKind::UnknownOutput { requested } => Diagnostic::error(
            DiagnosticCode::unknown_output(),
            format!("unknown output node `{requested}`"),
        )
        .with_node_id(requested.as_str()),
        GeometryErrorKind::UnknownParameter { parameter } => Diagnostic::error(
            DiagnosticCode::unknown_parameter(),
            format!("unknown parameter `{parameter}`"),
        )
        .with_parameter_id(parameter.as_str()),
        GeometryErrorKind::InvalidParameterValue { parameter, reason } => {
            let code = if *reason == "value must be finite" {
                DiagnosticCode::nonfinite_value()
            } else {
                DiagnosticCode::invalid_value()
            };
            Diagnostic::error(code, format!("invalid parameter `{parameter}`: {reason}"))
                .with_parameter_id(parameter.as_str())
        }
        GeometryErrorKind::InvalidTransform { reason } => {
            let code = if *reason == "scale must be positive" {
                DiagnosticCode::invalid_scale()
            } else if reason.contains("finite") {
                DiagnosticCode::nonfinite_value()
            } else {
                DiagnosticCode::invalid_value()
            };
            Diagnostic::error(code, format!("invalid transform: {reason}"))
        }
        GeometryErrorKind::InvalidPrimitive { reason } => {
            let code = if reason.contains("finite") {
                DiagnosticCode::nonfinite_value()
            } else {
                DiagnosticCode::invalid_primitive()
            };
            Diagnostic::error(code, format!("invalid primitive: {reason}"))
        }
        GeometryErrorKind::InvalidComposition {
            operator,
            child_count,
        } => Diagnostic::error(
            DiagnosticCode::invalid_composition(),
            format!(
                "invalid `{operator}` composition: expected at least 2 children, found {child_count}"
            ),
        )
        .with_context("operator", *operator)
        .with_context("child_count", child_count.to_string()),
        GeometryErrorKind::DependencyCycle { cycle } => Diagnostic::error(
            DiagnosticCode::dependency_cycle(),
            format!(
                "dependency cycle detected: {}",
                cycle
                    .iter()
                    .map(NodeId::as_str)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
        )
        .with_note("Geometry evaluation could not produce an acyclic dependency order."),
        GeometryErrorKind::UnsupportedShape { shape } => Diagnostic::error(
            DiagnosticCode::unsupported_geometry(),
            format!("unsupported shape `{shape}`"),
        )
        .with_remediation("Choose a backend-supported primitive or switch to a supported export path."),
        GeometryErrorKind::InvalidMesh { reason } => {
            let code = if *reason == "mesh is empty" {
                DiagnosticCode::empty_geometry()
            } else {
                DiagnosticCode::invalid_mesh()
            };
            Diagnostic::error(code, format!("invalid mesh: {reason}"))
        }
        GeometryErrorKind::BackendFailure { stage, message } => Diagnostic::error(
            DiagnosticCode::geometry_backend(),
            format!("geometry backend failed during {stage}: {message}"),
        )
        .with_context("backend_stage", stage.to_string()),
    };

    if let Some(node) = error.source_node() {
        diagnostic = diagnostic.with_node_id(node.as_str());
    }

    diagnostic
}

pub fn validate_evaluated_geometry(geometry: &EvaluatedGeometry) -> DiagnosticReport {
    let mut diagnostics = Vec::new();

    if geometry.mesh.is_empty() {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::empty_geometry(),
                format!(
                    "geometry evaluation for `{}` produced an empty mesh",
                    geometry.requested_output
                ),
            )
            .with_node_id(geometry.requested_output.as_str()),
        );
    }

    for position in geometry.mesh.positions() {
        for coordinate in position {
            if !coordinate.is_finite() {
                diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::invalid_mesh(),
                        format!(
                            "geometry evaluation for `{}` produced a non-finite mesh position",
                            geometry.requested_output
                        ),
                    )
                    .with_node_id(geometry.requested_output.as_str()),
                );
                break;
            }
        }
    }

    if let Some(size) = geometry.bounds.size()
        && size.iter().any(|value| !value.is_finite())
    {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::invalid_mesh(),
                format!(
                    "geometry evaluation for `{}` produced non-finite bounds",
                    geometry.requested_output
                ),
            )
            .with_node_id(geometry.requested_output.as_str()),
        );
    }

    DiagnosticReport::new(diagnostics)
}

pub fn validate_backend_support(scene: &SceneDocument) -> DiagnosticReport {
    let mut diagnostics = Vec::new();
    for node in scene.nodes().values() {
        match node.kind() {
            NodeKind::Plane(_) => diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::unsupported_geometry(),
                    format!(
                        "node `{}` uses `plane`, which the current geometry backend does not support",
                        node.id()
                    ),
                )
                .with_node_id(node.id().as_str())
                .with_remediation("Replace `plane` with a backend-supported primitive."),
            ),
            NodeKind::Profile(_) => diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::unsupported_geometry(),
                    format!(
                        "node `{}` uses `profile`, which the current geometry backend does not support",
                        node.id()
                    ),
                )
                .with_node_id(node.id().as_str())
                .with_remediation("Replace `profile` with a backend-supported primitive."),
            ),
            _ => {}
        }
    }
    DiagnosticReport::new(diagnostics)
}

/// Backend operation stage for normalized errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendStage {
    Primitive,
    Transform,
    BooleanUnion,
    BooleanDifference,
    BooleanIntersection,
    MeshConversion,
}

impl fmt::Display for BackendStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Primitive => "primitive build",
            Self::Transform => "transform",
            Self::BooleanUnion => "union",
            Self::BooleanDifference => "difference",
            Self::BooleanIntersection => "intersection",
            Self::MeshConversion => "mesh conversion",
        };
        f.write_str(label)
    }
}

fn resolve_parameters(
    parameters: &IndexMap<ParamId, ParameterDefinition>,
) -> Result<IndexMap<ParamId, ResolvedParameter>, GeometryError> {
    let mut resolved = IndexMap::new();
    for (parameter_id, parameter) in parameters {
        let value = parameter.scalar_value();
        ensure_finite(
            value,
            "parameter value",
            Some(GeometryError::new(
                GeometryErrorKind::InvalidParameterValue {
                    parameter: parameter_id.clone(),
                    reason: "value must be finite",
                },
                None,
            )),
        )?;
        resolved.insert(
            parameter_id.clone(),
            ResolvedParameter {
                id: parameter_id.clone(),
                value,
            },
        );
    }
    Ok(resolved)
}

fn resolve_node_operation(
    node_id: &NodeId,
    kind: &NodeKind,
    parameters: &IndexMap<ParamId, ResolvedParameter>,
    parameter_dependencies: &mut Vec<ParamId>,
) -> Result<GeometryOperation, GeometryError> {
    let operation = match kind {
        NodeKind::Box(primitive) => GeometryOperation::Primitive(PrimitiveShape::Box {
            size: [
                resolve_scalar(
                    node_id,
                    &primitive.size.x,
                    parameters,
                    parameter_dependencies,
                )?,
                resolve_scalar(
                    node_id,
                    &primitive.size.y,
                    parameters,
                    parameter_dependencies,
                )?,
                resolve_scalar(
                    node_id,
                    &primitive.size.z,
                    parameters,
                    parameter_dependencies,
                )?,
            ],
        }),
        NodeKind::Sphere(primitive) => GeometryOperation::Primitive(PrimitiveShape::Sphere {
            radius: resolve_positive_scalar(
                node_id,
                &primitive.radius,
                "sphere radius",
                parameters,
                parameter_dependencies,
            )?,
        }),
        NodeKind::Cylinder(primitive) => GeometryOperation::Primitive(PrimitiveShape::Cylinder {
            radius: resolve_positive_scalar(
                node_id,
                &primitive.radius,
                "cylinder radius",
                parameters,
                parameter_dependencies,
            )?,
            height: resolve_positive_scalar(
                node_id,
                &primitive.height,
                "cylinder height",
                parameters,
                parameter_dependencies,
            )?,
        }),
        NodeKind::Capsule(primitive) => GeometryOperation::Primitive(PrimitiveShape::Capsule {
            radius: resolve_positive_scalar(
                node_id,
                &primitive.radius,
                "capsule radius",
                parameters,
                parameter_dependencies,
            )?,
            height: resolve_positive_scalar(
                node_id,
                &primitive.height,
                "capsule height",
                parameters,
                parameter_dependencies,
            )?,
        }),
        NodeKind::Plane(primitive) => GeometryOperation::Primitive(PrimitiveShape::Plane {
            width: resolve_positive_scalar(
                node_id,
                &primitive.width,
                "plane width",
                parameters,
                parameter_dependencies,
            )?,
            depth: resolve_positive_scalar(
                node_id,
                &primitive.depth,
                "plane depth",
                parameters,
                parameter_dependencies,
            )?,
        }),
        NodeKind::Profile(primitive) => GeometryOperation::Primitive(PrimitiveShape::Profile {
            width: resolve_positive_scalar(
                node_id,
                &primitive.width,
                "profile width",
                parameters,
                parameter_dependencies,
            )?,
            height: resolve_positive_scalar(
                node_id,
                &primitive.height,
                "profile height",
                parameters,
                parameter_dependencies,
            )?,
        }),
        NodeKind::Union(composition) => GeometryOperation::Union {
            children: resolve_children(node_id, "union", composition)?,
        },
        NodeKind::Difference(composition) => GeometryOperation::Difference {
            children: resolve_children(node_id, "difference", composition)?,
        },
        NodeKind::Intersection(composition) => GeometryOperation::Intersection {
            children: resolve_children(node_id, "intersection", composition)?,
        },
    };

    if let GeometryOperation::Primitive(PrimitiveShape::Box { size }) = &operation
        && size.iter().any(|component| *component <= 0.0)
    {
        return Err(GeometryError::new(
            GeometryErrorKind::InvalidPrimitive {
                reason: "box size components must be strictly positive",
            },
            Some(node_id.clone()),
        ));
    }

    if let GeometryOperation::Primitive(PrimitiveShape::Capsule { radius, height }) = &operation
        && *height < radius * 2.0
    {
        return Err(GeometryError::new(
            GeometryErrorKind::InvalidPrimitive {
                reason: "capsule height must be at least twice the radius",
            },
            Some(node_id.clone()),
        ));
    }

    dedup_in_declared_order(parameter_dependencies);
    Ok(operation)
}

fn resolve_children(
    node_id: &NodeId,
    operator: &'static str,
    composition: &CompositionNode,
) -> Result<Vec<NodeId>, GeometryError> {
    if composition.children.len() < 2 {
        return Err(GeometryError::new(
            GeometryErrorKind::InvalidComposition {
                operator,
                child_count: composition.children.len(),
            },
            Some(node_id.clone()),
        ));
    }
    Ok(composition
        .children
        .iter()
        .map(|child| child.target().clone())
        .collect())
}

fn resolve_transform(
    node_id: &NodeId,
    transform: &Transform,
    parameters: &IndexMap<ParamId, ResolvedParameter>,
) -> Result<ResolvedTransform, GeometryError> {
    let translation = resolve_vector3(node_id, &transform.translation, parameters)?;
    let rotation_deg = resolve_vector3(node_id, &transform.rotation_deg, parameters)?;
    let scale = resolve_positive_vector3(node_id, &transform.scale, parameters)?;

    Ok(ResolvedTransform {
        translation,
        rotation_deg,
        scale,
    })
}

fn resolve_vector3(
    node_id: &NodeId,
    vector: &Vector3Expr,
    parameters: &IndexMap<ParamId, ResolvedParameter>,
) -> Result<[f64; 3], GeometryError> {
    Ok([
        resolve_scalar(node_id, &vector.x, parameters, &mut Vec::new())?,
        resolve_scalar(node_id, &vector.y, parameters, &mut Vec::new())?,
        resolve_scalar(node_id, &vector.z, parameters, &mut Vec::new())?,
    ])
}

fn resolve_positive_vector3(
    node_id: &NodeId,
    vector: &Vector3Expr,
    parameters: &IndexMap<ParamId, ResolvedParameter>,
) -> Result<[f64; 3], GeometryError> {
    Ok([
        resolve_positive_scalar(node_id, &vector.x, "scale.x", parameters, &mut Vec::new())?,
        resolve_positive_scalar(node_id, &vector.y, "scale.y", parameters, &mut Vec::new())?,
        resolve_positive_scalar(node_id, &vector.z, "scale.z", parameters, &mut Vec::new())?,
    ])
}

fn resolve_scalar(
    node_id: &NodeId,
    expression: &ScalarExpr,
    parameters: &IndexMap<ParamId, ResolvedParameter>,
    parameter_dependencies: &mut Vec<ParamId>,
) -> Result<f64, GeometryError> {
    let value = match expression {
        ScalarExpr::Literal(value) => *value,
        ScalarExpr::Parameter(reference) => {
            let parameter = parameters.get(reference.target()).ok_or_else(|| {
                GeometryError::new(
                    GeometryErrorKind::UnknownParameter {
                        parameter: reference.target().clone(),
                    },
                    Some(node_id.clone()),
                )
            })?;
            parameter_dependencies.push(reference.target().clone());
            parameter.value()
        }
    };

    if !value.is_finite() {
        return match parameter_dependencies.last() {
            Some(parameter) => Err(GeometryError::new(
                GeometryErrorKind::InvalidParameterValue {
                    parameter: parameter.clone(),
                    reason: "value must be finite",
                },
                Some(node_id.clone()),
            )),
            None => Err(GeometryError::new(
                GeometryErrorKind::InvalidPrimitive {
                    reason: "literal values must be finite",
                },
                Some(node_id.clone()),
            )),
        };
    }
    Ok(value)
}

fn resolve_positive_scalar(
    node_id: &NodeId,
    expression: &ScalarExpr,
    label: &'static str,
    parameters: &IndexMap<ParamId, ResolvedParameter>,
    parameter_dependencies: &mut Vec<ParamId>,
) -> Result<f64, GeometryError> {
    let value = resolve_scalar(node_id, expression, parameters, parameter_dependencies)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(GeometryError::new(
            GeometryErrorKind::InvalidPrimitive { reason: label },
            Some(node_id.clone()),
        ))
    }
}

fn evaluate_node_with_backend<B: GeometryBackend>(
    backend: &mut B,
    node: &GeometryNode,
    solids: &HashMap<NodeId, B::Solid>,
) -> Result<B::Solid, GeometryError> {
    let solid = match node.operation() {
        GeometryOperation::Primitive(primitive) => match primitive {
            PrimitiveShape::Plane { .. } => {
                return Err(GeometryError::new(
                    GeometryErrorKind::UnsupportedShape { shape: "plane" },
                    Some(node.source_id().clone()),
                ));
            }
            PrimitiveShape::Profile { .. } => {
                return Err(GeometryError::new(
                    GeometryErrorKind::UnsupportedShape { shape: "profile" },
                    Some(node.source_id().clone()),
                ));
            }
            _ => backend
                .build_primitive(node.source_id(), primitive)
                .map_err(|error| backend_error(BackendStage::Primitive, node.source_id(), error))?,
        },
        GeometryOperation::Union { children } => {
            let solids = collect_child_solids(node.source_id(), children, solids)?;
            backend.union(node.source_id(), &solids).map_err(|error| {
                backend_error(BackendStage::BooleanUnion, node.source_id(), error)
            })?
        }
        GeometryOperation::Difference { children } => {
            let solids = collect_child_solids(node.source_id(), children, solids)?;
            backend
                .difference(node.source_id(), &solids)
                .map_err(|error| {
                    backend_error(BackendStage::BooleanDifference, node.source_id(), error)
                })?
        }
        GeometryOperation::Intersection { children } => {
            let solids = collect_child_solids(node.source_id(), children, solids)?;
            backend
                .intersection(node.source_id(), &solids)
                .map_err(|error| {
                    backend_error(BackendStage::BooleanIntersection, node.source_id(), error)
                })?
        }
    };

    backend
        .apply_transform(node.source_id(), &solid, node.transform())
        .map_err(|error| backend_error(BackendStage::Transform, node.source_id(), error))
}

fn collect_child_solids<Solid: Clone>(
    source: &NodeId,
    children: &[NodeId],
    solids: &HashMap<NodeId, Solid>,
) -> Result<Vec<Solid>, GeometryError> {
    let mut ordered = Vec::with_capacity(children.len());
    for child in children {
        let solid = solids.get(child).ok_or_else(|| {
            GeometryError::new(
                GeometryErrorKind::UnknownOutput {
                    requested: child.clone(),
                },
                Some(source.clone()),
            )
        })?;
        ordered.push(solid.clone());
    }
    Ok(ordered)
}

fn semantic_fingerprint(node: &GeometryNode) -> u64 {
    let mut hasher = DefaultHasher::new();
    node.source_id.hash(&mut hasher);
    match node.operation() {
        GeometryOperation::Primitive(primitive) => hash_primitive(primitive, &mut hasher),
        GeometryOperation::Union { children } => {
            "union".hash(&mut hasher);
            children.hash(&mut hasher);
        }
        GeometryOperation::Difference { children } => {
            "difference".hash(&mut hasher);
            children.hash(&mut hasher);
        }
        GeometryOperation::Intersection { children } => {
            "intersection".hash(&mut hasher);
            children.hash(&mut hasher);
        }
    }
    for value in node.transform.translation {
        value.to_bits().hash(&mut hasher);
    }
    for value in node.transform.rotation_deg {
        value.to_bits().hash(&mut hasher);
    }
    for value in node.transform.scale {
        value.to_bits().hash(&mut hasher);
    }
    for parameter in &node.parameter_dependencies {
        parameter.hash(&mut hasher);
    }
    hasher.finish()
}

fn hash_primitive(primitive: &PrimitiveShape, hasher: &mut DefaultHasher) {
    match primitive {
        PrimitiveShape::Box { size } => {
            "box".hash(hasher);
            for value in size {
                value.to_bits().hash(hasher);
            }
        }
        PrimitiveShape::Sphere { radius } => {
            "sphere".hash(hasher);
            radius.to_bits().hash(hasher);
        }
        PrimitiveShape::Cylinder { radius, height } => {
            "cylinder".hash(hasher);
            radius.to_bits().hash(hasher);
            height.to_bits().hash(hasher);
        }
        PrimitiveShape::Capsule { radius, height } => {
            "capsule".hash(hasher);
            radius.to_bits().hash(hasher);
            height.to_bits().hash(hasher);
        }
        PrimitiveShape::Plane { width, depth } => {
            "plane".hash(hasher);
            width.to_bits().hash(hasher);
            depth.to_bits().hash(hasher);
        }
        PrimitiveShape::Profile { width, height } => {
            "profile".hash(hasher);
            width.to_bits().hash(hasher);
            height.to_bits().hash(hasher);
        }
    }
}

fn combine_fingerprint(left: u64, right: u64) -> u64 {
    left.rotate_left(7) ^ right.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn dedup_in_declared_order(values: &mut Vec<ParamId>) {
    let mut seen = IndexSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn validate_mesh(
    positions: &[[f64; 3]],
    triangle_indices: &[[u32; 3]],
) -> Result<(), GeometryError> {
    for position in positions {
        for coordinate in position {
            ensure_finite(
                *coordinate,
                "mesh position",
                Some(GeometryError::new(
                    GeometryErrorKind::InvalidMesh {
                        reason: "positions must be finite",
                    },
                    None,
                )),
            )?;
        }
    }

    for triangle in triangle_indices {
        for index in triangle {
            if usize::try_from(*index).map_or(true, |value| value >= positions.len()) {
                return Err(GeometryError::new(
                    GeometryErrorKind::InvalidMesh {
                        reason: "triangle index out of range",
                    },
                    None,
                ));
            }
        }
    }

    Ok(())
}

fn ensure_finite(
    value: f64,
    _context: &'static str,
    error: Option<GeometryError>,
) -> Result<(), GeometryError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(error.unwrap_or_else(|| {
            GeometryError::new(
                GeometryErrorKind::InvalidTransform {
                    reason: "value must be finite",
                },
                None,
            )
        }))
    }
}

fn backend_error<E: Error>(stage: BackendStage, source: &NodeId, error: E) -> GeometryError {
    GeometryError::new(
        GeometryErrorKind::BackendFailure {
            stage,
            message: error.to_string(),
        },
        Some(source.clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use criterion::Criterion;
    use geom_scene::parse_scene;
    use std::fs;
    use std::path::Path;

    #[derive(Debug, Default, Clone)]
    struct CountingBackend {
        calls: Vec<String>,
    }

    impl CountingBackend {
        fn counts_for(&self, prefix: &str) -> usize {
            self.calls
                .iter()
                .filter(|entry| entry.starts_with(prefix))
                .count()
        }
    }

    impl GeometryBackend for CountingBackend {
        type Solid = String;
        type Error = CountingBackendError;

        fn build_primitive(
            &mut self,
            source: &NodeId,
            primitive: &PrimitiveShape,
        ) -> Result<Self::Solid, Self::Error> {
            self.calls.push(format!("primitive:{source}"));
            Ok(format!("primitive:{primitive:?}"))
        }

        fn apply_transform(
            &mut self,
            source: &NodeId,
            solid: &Self::Solid,
            _transform: &ResolvedTransform,
        ) -> Result<Self::Solid, Self::Error> {
            self.calls.push(format!("transform:{source}"));
            Ok(format!("tx({solid})"))
        }

        fn union(
            &mut self,
            source: &NodeId,
            solids: &[Self::Solid],
        ) -> Result<Self::Solid, Self::Error> {
            self.calls.push(format!("union:{source}"));
            Ok(format!("union({})", solids.join(",")))
        }

        fn difference(
            &mut self,
            source: &NodeId,
            solids: &[Self::Solid],
        ) -> Result<Self::Solid, Self::Error> {
            self.calls.push(format!("difference:{source}"));
            Ok(format!("difference({})", solids.join(",")))
        }

        fn intersection(
            &mut self,
            source: &NodeId,
            solids: &[Self::Solid],
        ) -> Result<Self::Solid, Self::Error> {
            self.calls.push(format!("intersection:{source}"));
            Ok(format!("intersection({})", solids.join(",")))
        }

        fn to_mesh(&mut self, source: &NodeId, _solid: &Self::Solid) -> Result<Mesh, Self::Error> {
            self.calls.push(format!("mesh:{source}"));
            Mesh::new(
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                vec![[0, 1, 2]],
            )
            .map_err(|error| CountingBackendError(error.to_string()))
        }
    }

    #[derive(Debug, Clone)]
    struct CountingBackendError(String);

    impl fmt::Display for CountingBackendError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }

    impl Error for CountingBackendError {}

    const CACHE_SCENE: &str = r#"
schema_version = 1
root = "root"

[params.left_size]
type = "scalar"
value = 1.0

[params.right_size]
type = "scalar"
value = 1.0

[nodes.root]
kind = "union"
children = ["left_union", "right_union"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.left_union]
kind = "union"
children = ["left_box", "left_sphere"]
transform = { translate = { x = -2.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.left_box]
kind = "box"
size = { x = { param = "left_size" }, y = 1.0, z = 1.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.left_sphere]
kind = "sphere"
radius = 0.75
transform = { translate = { x = 0.75, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.right_union]
kind = "union"
children = ["right_box", "right_sphere"]
transform = { translate = { x = 2.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.right_box]
kind = "box"
size = { x = { param = "right_size" }, y = 1.0, z = 1.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.right_sphere]
kind = "sphere"
radius = 0.75
transform = { translate = { x = -0.75, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;

    #[test]
    fn selected_output_only_evaluates_reachable_subtree() {
        let scene = parse_scene(CACHE_SCENE).expect("parse scene");
        let mut evaluator = GeometryEvaluator::new(CountingBackend::default());
        let node_id = NodeId::new("left_union").expect("node id");

        let result = evaluator
            .evaluate_node(&scene, &node_id)
            .expect("evaluate selected subtree");

        assert_eq!(
            result
                .participating_node_ids
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["left_box", "left_sphere", "left_union"]
        );
        assert_eq!(evaluator.backend().counts_for("primitive:right"), 0);
        assert_eq!(evaluator.backend().counts_for("union:right_union"), 0);
    }

    #[test]
    fn repeated_identical_evaluation_reuses_cached_subtrees() {
        let scene = parse_scene(CACHE_SCENE).expect("parse scene");
        let mut evaluator = GeometryEvaluator::new(CountingBackend::default());

        evaluator.evaluate_root(&scene).expect("first evaluation");
        let initial_primitive_calls = evaluator.backend().counts_for("primitive:");
        let initial_union_calls = evaluator.backend().counts_for("union:");

        let second = evaluator.evaluate_root(&scene).expect("second evaluation");
        assert_eq!(
            initial_primitive_calls,
            evaluator.backend().counts_for("primitive:")
        );
        assert_eq!(
            initial_union_calls,
            evaluator.backend().counts_for("union:")
        );
        assert_eq!(second.stats.cache_hits, 7);
        assert_eq!(second.stats.cache_misses, 0);
    }

    #[test]
    fn parameter_change_invalidates_only_dependent_branch() {
        let mut source = geom_scene::SceneSource::parse(CACHE_SCENE).expect("parse source");
        let original = source.validate().expect("validate original");
        let mut evaluator = GeometryEvaluator::new(CountingBackend::default());

        evaluator
            .evaluate_root(&original)
            .expect("first evaluation");
        let original_call_count = evaluator.backend().calls.len();

        let updated_scene = source
            .set_parameter_scalar(&ParamId::new("left_size").expect("param id"), 2.0)
            .expect("update left parameter");
        let result = evaluator
            .evaluate_root(&updated_scene)
            .expect("evaluate updated scene");

        let delta = &evaluator.backend().calls[original_call_count..];
        assert!(delta.iter().any(|entry| entry == "primitive:left_box"));
        assert!(delta.iter().any(|entry| entry == "union:left_union"));
        assert!(delta.iter().any(|entry| entry == "union:root"));
        assert!(!delta.iter().any(|entry| entry == "primitive:right_box"));
        assert!(!delta.iter().any(|entry| entry == "primitive:right_sphere"));
        assert_eq!(result.stats.cache_hits, 4);
        assert_eq!(result.stats.cache_misses, 3);
    }

    #[test]
    fn dependency_order_is_deterministic_and_child_ordered() {
        let scene = parse_scene(CACHE_SCENE).expect("parse scene");
        let graph = GeometryGraph::from_scene(&scene).expect("graph");
        let order = graph.dependency_order_for(scene.root()).expect("order");
        assert_eq!(
            order.iter().map(NodeId::as_str).collect::<Vec<_>>(),
            vec![
                "left_box",
                "left_sphere",
                "left_union",
                "right_box",
                "right_sphere",
                "right_union",
                "root"
            ]
        );
    }

    #[test]
    fn graph_cycle_detection_rejects_internal_cycle() {
        let a = NodeId::new("a").expect("a");
        let b = NodeId::new("b").expect("b");
        let mut nodes = IndexMap::new();
        nodes.insert(
            a.clone(),
            GeometryNode {
                source_id: a.clone(),
                operation: GeometryOperation::Union {
                    children: vec![b.clone(), a.clone()],
                },
                transform: identity_transform(),
                geometry_dependencies: vec![b.clone()],
                parameter_dependencies: Vec::new(),
            },
        );
        nodes.insert(
            b.clone(),
            GeometryNode {
                source_id: b.clone(),
                operation: GeometryOperation::Union {
                    children: vec![a.clone(), b.clone()],
                },
                transform: identity_transform(),
                geometry_dependencies: vec![a.clone()],
                parameter_dependencies: Vec::new(),
            },
        );

        let graph = GeometryGraph {
            root: a.clone(),
            parameters: IndexMap::new(),
            nodes,
            dependents: HashMap::new(),
        };
        let error = graph
            .dependency_order_for(&a)
            .expect_err("cycle should fail");
        assert!(matches!(
            error.kind(),
            GeometryErrorKind::DependencyCycle { .. }
        ));
    }

    #[test]
    fn unsupported_placeholders_stay_in_ir_but_fail_on_evaluation() {
        let scene = parse_scene(
            r#"
schema_version = 1
root = "plane"

[nodes.plane]
kind = "plane"
width = 2.0
depth = 3.0
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#,
        )
        .expect("parse plane");

        let graph = GeometryGraph::from_scene(&scene).expect("graph");
        match graph.nodes()[scene.root()].operation() {
            GeometryOperation::Primitive(PrimitiveShape::Plane { width, depth }) => {
                assert_eq!((*width, *depth), (2.0, 3.0));
            }
            other => panic!("unexpected operation: {other:?}"),
        }

        let mut evaluator = GeometryEvaluator::new(CountingBackend::default());
        let error = evaluator
            .evaluate_root(&scene)
            .expect_err("plane should be unsupported");
        assert!(matches!(
            error.kind(),
            GeometryErrorKind::UnsupportedShape { shape: "plane" }
        ));
    }

    #[test]
    fn production_backend_box_evaluates_with_expected_bounds() {
        let result = evaluate_fixture("minimal-primitive.toml");
        assert!(result.mesh.positions().len() >= 8);
        assert!(!result.mesh.is_empty());
        assert_bounds_close(result.bounds, [-0.5, -0.5, -0.5], [0.5, 0.5, 0.5], 0.05);
    }

    #[test]
    fn production_backend_sphere_evaluates_with_expected_bounds() {
        let scene = parse_scene(
            r#"
schema_version = 1
root = "ball"

[nodes.ball]
kind = "sphere"
radius = 1.25
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#,
        )
        .expect("sphere scene");
        let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
        let result = evaluator.evaluate_root(&scene).expect("sphere evaluation");
        assert!(!result.mesh.is_empty());
        assert_bounds_close(
            result.bounds,
            [-1.25, -1.25, -1.25],
            [1.25, 1.25, 1.25],
            0.08,
        );
    }

    #[test]
    fn production_backend_cylinder_and_capsule_evaluate() {
        let scene = parse_scene(
            r#"
schema_version = 1
root = "capsule"

[nodes.capsule]
kind = "capsule"
radius = 0.5
height = 3.0
transform = { translate = { x = 0.0, y = 1.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.cylinder]
kind = "cylinder"
radius = 0.25
height = 2.0
transform = { translate = { x = 3.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#,
        )
        .expect("parse scene");
        let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
        let capsule = evaluator.evaluate_root(&scene).expect("capsule");
        assert!(!capsule.mesh.is_empty());
        assert_bounds_close(capsule.bounds, [-0.5, -0.5, -0.5], [0.5, 2.5, 0.5], 0.1);

        let cylinder_id = NodeId::new("cylinder").expect("cylinder id");
        let cylinder = evaluator
            .evaluate_node(&scene, &cylinder_id)
            .expect("cylinder");
        assert!(!cylinder.mesh.is_empty());
        assert_bounds_close(cylinder.bounds, [2.75, -1.0, -0.25], [3.25, 1.0, 0.25], 0.1);
    }

    #[test]
    fn production_backend_boolean_ops_evaluate_to_sensible_bounds() {
        let scene = parse_scene(
            r#"
schema_version = 1
root = "root"

[nodes.base]
kind = "box"
size = { x = 3.0, y = 3.0, z = 3.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.offset_sphere]
kind = "sphere"
radius = 1.2
transform = { translate = { x = 0.8, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.union_shape]
kind = "union"
children = ["base", "offset_sphere"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.cutout]
kind = "cylinder"
radius = 0.4
height = 4.0
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 90.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.diff_shape]
kind = "difference"
children = ["union_shape", "cutout"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.intersection_shape]
kind = "intersection"
children = ["base", "offset_sphere"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.root]
kind = "union"
children = ["diff_shape", "intersection_shape"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#,
        )
        .expect("parse scene");

        let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
        let result = evaluator.evaluate_root(&scene).expect("boolean evaluation");
        assert!(!result.mesh.is_empty());
        assert_bounds_close(result.bounds, [-1.5, -1.5, -1.5], [2.0, 1.5, 1.5], 0.2);

        let intersection = evaluator
            .evaluate_node(&scene, &NodeId::new("intersection_shape").expect("id"))
            .expect("intersection");
        assert!(!intersection.mesh.is_empty());
        let size = intersection
            .bounds
            .size()
            .expect("intersection bounds size");
        assert!(size[0] < 3.0);
        assert!(size[1] <= 2.4);
        assert!(size[2] <= 2.4);
    }

    #[test]
    fn parameter_resolution_is_exposed_in_results() {
        let scene = parse_scene(
            r#"
schema_version = 1
root = "cube"

[params.width]
type = "scalar"
value = 2.5

[nodes.cube]
kind = "box"
size = { x = { param = "width" }, y = 1.0, z = 1.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#,
        )
        .expect("parse scene");
        let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
        let result = evaluator.evaluate_root(&scene).expect("evaluate");

        assert_eq!(
            result.resolved_parameters[&ParamId::new("width").expect("param id")].value(),
            2.5
        );
        assert_bounds_close(result.bounds, [-1.25, -0.5, -0.5], [1.25, 0.5, 0.5], 0.08);
    }

    #[test]
    fn transform_order_matches_scale_rotate_translate() {
        let scene = parse_scene(
            r#"
schema_version = 1
root = "box"

[nodes.box]
kind = "box"
size = { x = 2.0, y = 1.0, z = 1.0 }
transform = { translate = { x = 5.0, y = 1.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 90.0 }, scale = { x = 2.0, y = 1.0, z = 1.0 } }
"#,
        )
        .expect("parse scene");
        let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
        let result = evaluator.evaluate_root(&scene).expect("evaluate");

        assert_bounds_close(result.bounds, [4.5, -1.0, -0.5], [5.5, 3.0, 0.5], 0.12);
    }

    #[test]
    fn checked_in_examples_still_parse_and_benchmark_fixture_evaluates() {
        let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("scenes");
        for entry in fs::read_dir(&examples_dir).expect("read examples") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("toml") {
                let source = fs::read_to_string(&path).expect("read example");
                let scene = parse_scene(&source)
                    .unwrap_or_else(|error| panic!("example {} failed: {error}", path.display()));
                let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
                let root_result = evaluator.evaluate_root(&scene);
                if path.file_name().and_then(|name| name.to_str())
                    == Some("benchmark-cache-tree.toml")
                {
                    assert!(root_result.is_ok(), "benchmark fixture must evaluate");
                }
            }
        }
    }

    fn evaluate_fixture(name: &str) -> EvaluatedGeometry {
        let source = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("examples")
                .join("scenes")
                .join(name),
        )
        .expect("read fixture");
        let scene = parse_scene(&source).expect("parse fixture");
        let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
        evaluator.evaluate_root(&scene).expect("evaluate fixture")
    }

    fn assert_bounds_close(bounds: Bounds, min: [f64; 3], max: [f64; 3], tolerance: f64) {
        match bounds {
            Bounds::Empty => panic!("expected non-empty bounds"),
            Bounds::Finite {
                min: actual_min,
                max: actual_max,
            } => {
                for axis in 0..3 {
                    assert!(
                        (actual_min[axis] - min[axis]).abs() <= tolerance,
                        "min axis {axis}: expected {}, got {}",
                        min[axis],
                        actual_min[axis]
                    );
                    assert!(
                        (actual_max[axis] - max[axis]).abs() <= tolerance,
                        "max axis {axis}: expected {}, got {}",
                        max[axis],
                        actual_max[axis]
                    );
                }
            }
        }
    }

    fn identity_transform() -> ResolvedTransform {
        ResolvedTransform {
            translation: [0.0, 0.0, 0.0],
            rotation_deg: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }
    }

    #[test]
    fn benchmark_smoke_runs() {
        let source = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("examples")
                .join("scenes")
                .join("benchmark-cache-tree.toml"),
        )
        .expect("read benchmark scene");
        let scene = parse_scene(&source).expect("parse benchmark scene");
        let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
        evaluator.evaluate_root(&scene).expect("cold evaluation");
        evaluator.evaluate_root(&scene).expect("warm evaluation");

        let mut criterion = Criterion::default().without_plots();
        criterion.bench_function("geometry_smoke", |bench| {
            bench.iter(|| {
                evaluator.evaluate_root(&scene).expect("bench evaluation");
            });
        });
    }

    #[test]
    fn geometry_error_normalization_preserves_code_and_node_context() {
        let node = NodeId::new("plane").expect("node");
        let diagnostic = diagnostic_from_geometry_error(&GeometryError::new(
            GeometryErrorKind::UnsupportedShape { shape: "plane" },
            Some(node.clone()),
        ));

        assert_eq!(diagnostic.code.0, "MORPHOS_UNSUPPORTED_GEOMETRY");
        assert_eq!(diagnostic.node_id.as_deref(), Some(node.as_str()));
        assert!(diagnostic.remediation.is_some());
    }

    #[test]
    fn validate_evaluated_geometry_reports_empty_mesh() {
        let geometry = EvaluatedGeometry {
            requested_output: NodeId::new("root").expect("node"),
            mesh: Mesh::new(Vec::new(), Vec::new()).expect("empty mesh is valid container"),
            bounds: Bounds::Empty,
            stats: GeometryStats {
                vertex_count: 0,
                triangle_count: 0,
                evaluated_node_count: 1,
                cache_hits: 0,
                cache_misses: 1,
            },
            resolved_parameters: IndexMap::new(),
            participating_node_ids: Vec::new(),
            evaluation_revision: 1,
        };

        let report = validate_evaluated_geometry(&geometry);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].code.0, "MORPHOS_EMPTY_GEOMETRY");
    }

    #[test]
    fn validate_backend_support_reports_unsupported_placeholders() {
        let scene = parse_scene(
            r#"
schema_version = 1
root = "plane"

[nodes.plane]
kind = "plane"
width = 2.0
depth = 3.0
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#,
        )
        .expect("parse");

        let report = validate_backend_support(&scene);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].code.0, "MORPHOS_UNSUPPORTED_GEOMETRY");
        assert_eq!(report.diagnostics[0].node_id.as_deref(), Some("plane"));
    }
}
