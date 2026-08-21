//! Declarative scene schema and source-preserving editing for Morphos.
//!
//! `geom_scene` owns the first typed scene language for `source/scene.toml`.
//! It intentionally separates:
//!
//! - `SceneSource`: a source-preserving editable TOML representation
//! - `SceneDocument`: a strongly typed semantic scene model
//!
//! Callers can keep invalid raw text in `SceneSource`-like storage outside this
//! crate and preserve the last valid `SceneDocument` returned by `parse_scene`.
//! A failed parse or validation never mutates a previously validated scene.
//!
//! Schema conventions in M02:
//!
//! - `schema_version = 1` is explicit and independent of workspace format
//! - `root = "<node-id>"` defines the primary output node
//! - the coordinate system is right-handed with `+Y` as up
//! - translation units are abstract scene units
//! - rotations use Euler XYZ degrees in a `rotate_deg` vector
//! - local transform application order is scale, then rotate, then translate
//! - `scale` must be strictly positive on every axis
//! - named parameters currently support one typed value kind: scalar
//! - targeted source edits preserve unrelated comments/order/formatting by
//!   mutating a `toml_edit::DocumentMut` in place instead of regenerating the
//!   whole file from the semantic model
//! - unknown structural fields are rejected, while designated `extensions`
//!   sections are preserved as opaque-but-typed metadata

use indexmap::IndexMap;
use std::error::Error;
use std::fmt;
use std::ops::Range;
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value, value};

/// The current supported scene schema version.
pub const SCENE_SCHEMA_VERSION: u32 = 1;

/// Parses, migrates, and validates a scene source string into a typed document.
pub fn parse_scene(source: &str) -> Result<SceneDocument, SceneError> {
    let scene_source = SceneSource::parse(source)?;
    let scene_source = migrate_to_current(scene_source)?;
    scene_source.validate()
}

/// Establishes the M02 migration boundary for future schema upgrades.
pub fn migrate_to_current(scene_source: SceneSource) -> Result<SceneSource, SceneError> {
    let version = scene_source.schema_version()?;
    if version == SCENE_SCHEMA_VERSION {
        Ok(scene_source)
    } else {
        Err(SceneError::new(
            SceneErrorKind::UnsupportedSchemaVersion {
                found: version,
                supported: SCENE_SCHEMA_VERSION,
            },
            span_for_item(
                scene_source.document.get("schema_version"),
                scene_source.text(),
            ),
        ))
    }
}

/// A source-preserving editable TOML scene representation.
#[derive(Debug, Clone)]
pub struct SceneSource {
    text: String,
    document: DocumentMut,
}

impl SceneSource {
    /// Parses TOML source into an editable representation without semantic validation.
    pub fn parse(source: &str) -> Result<Self, SceneError> {
        let document = source.parse::<DocumentMut>().map_err(|error| {
            let span = error
                .span()
                .map(|range| SourceSpan::from_range(source, range));
            SceneError::new(
                SceneErrorKind::Parse {
                    message: error.to_string(),
                },
                span,
            )
        })?;

        Ok(Self {
            text: source.to_owned(),
            document,
        })
    }

    /// Returns the current scene source text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the parsed but still source-oriented scene schema version.
    pub fn schema_version(&self) -> Result<u32, SceneError> {
        let item = self
            .document
            .get("schema_version")
            .ok_or_else(|| SceneError::new(missing_field_error("schema_version", None), None))?;
        parse_u32_item(item, "schema_version", &self.text)
    }

    /// Validates the current source against the M02 schema.
    pub fn validate(&self) -> Result<SceneDocument, SceneError> {
        validate_scene_source(&self.document, &self.text)
    }

    /// Returns the current editable scene as owned source text.
    pub fn into_text(self) -> String {
        self.text
    }

    /// Updates a named scalar parameter in place and revalidates the result.
    pub fn set_parameter_scalar(
        &mut self,
        parameter: &ParamId,
        value_number: f64,
    ) -> Result<SceneDocument, SceneError> {
        ensure_finite_scalar(value_number, "parameter value", None, &self.text)?;
        let parameter_table = match self.document.get_mut("params").and_then(Item::as_table_mut) {
            Some(table) => table,
            None => return Err(SceneError::new(missing_field_error("params", None), None)),
        };
        let parameter_item = match parameter_table
            .get_mut(parameter.as_str())
            .and_then(Item::as_table_mut)
        {
            Some(table) => table,
            None => {
                return Err(SceneError::new(
                    SceneErrorKind::MissingParameter {
                        parameter: parameter.clone(),
                    },
                    None,
                ));
            }
        };

        replace_item_value_preserving_decor(&mut parameter_item["value"], value_number);
        self.refresh_after_edit()
    }

    /// Updates one primitive scalar property in place and revalidates the result.
    pub fn set_primitive_scalar(
        &mut self,
        node: &NodeId,
        field: PrimitiveScalarField,
        value_number: f64,
    ) -> Result<SceneDocument, SceneError> {
        let text = self.text.clone();
        let node_table = self.lookup_node_table_mut(node)?;
        let kind_item = match node_table.get("kind") {
            Some(item) => item,
            None => {
                return Err(SceneError::new(
                    missing_field_error("kind", Some(node.as_str().to_owned())),
                    None,
                ));
            }
        };
        let kind = parse_string_item(kind_item, "kind", &text)?;

        let (key, axis_or_none) = field.toml_path_for_kind(kind.as_str()).ok_or_else(|| {
            SceneError::new(
                SceneErrorKind::InvalidEditTarget {
                    message: format!(
                        "primitive field `{}` does not apply to node kind `{kind}`",
                        field.as_str()
                    ),
                },
                span_for_item(node_table.get("kind"), &text),
            )
        })?;

        ensure_positive_scalar(
            value_number,
            field.validation_label(),
            span_for_item(node_table.get(key), &text),
            &text,
        )?;

        match axis_or_none {
            Some(axis) => {
                node_table[key][axis] = value(value_number);
            }
            None => {
                node_table[key] = value(value_number);
            }
        }

        self.refresh_after_edit()
    }

    /// Updates a transform component in place and revalidates the result.
    pub fn set_transform_component(
        &mut self,
        node: &NodeId,
        property: TransformProperty,
        axis: Axis,
        value_number: f64,
    ) -> Result<SceneDocument, SceneError> {
        let text = self.text.clone();
        let node_table = self.lookup_node_table_mut(node)?;
        if matches!(property, TransformProperty::Scale) {
            ensure_positive_scalar(
                value_number,
                "scale component",
                span_for_item(node_table.get("transform"), &text),
                &text,
            )?;
        } else {
            ensure_finite_scalar(
                value_number,
                "transform component",
                span_for_item(node_table.get("transform"), &text),
                &text,
            )?;
        }

        let property_key = property.key();
        node_table["transform"][property_key][axis.key()] = value(value_number);
        self.refresh_after_edit()
    }

    /// Updates the root/output node reference.
    pub fn set_root_node(&mut self, node: &NodeId) -> Result<SceneDocument, SceneError> {
        self.document["root"] = value(node.as_str());
        self.refresh_after_edit()
    }

    /// Updates or clears a node label in place and revalidates the result.
    pub fn set_node_label(
        &mut self,
        node: &NodeId,
        label: Option<&str>,
    ) -> Result<SceneDocument, SceneError> {
        let node_table = self.lookup_node_table_mut(node)?;
        match label.map(str::trim).filter(|label| !label.is_empty()) {
            Some(label) => {
                node_table["label"] = value(label);
            }
            None => {
                node_table.remove("label");
            }
        }
        self.refresh_after_edit()
    }

    /// Updates composition children in place and revalidates the result.
    pub fn set_composition_children(
        &mut self,
        node: &NodeId,
        children: &[NodeId],
    ) -> Result<SceneDocument, SceneError> {
        let text = self.text.clone();
        if children.len() < 2 {
            return Err(SceneError::new(
                SceneErrorKind::InvalidValue {
                    context: format!("nodes.{}.children", node.as_str()),
                    message: "composition nodes require at least two child references".to_owned(),
                },
                None,
            ));
        }

        let node_table = self.lookup_node_table_mut(node)?;
        let kind_item = node_table.get("kind").ok_or_else(|| {
            SceneError::new(
                missing_field_error("kind", Some(node.as_str().to_owned())),
                None,
            )
        })?;
        let kind = parse_string_item(kind_item, "kind", &text)?;
        if !matches!(kind.as_str(), "union" | "difference" | "intersection") {
            return Err(SceneError::new(
                SceneErrorKind::InvalidEditTarget {
                    message: format!("node `{node}` is not a composition node"),
                },
                span_for_item(node_table.get("kind"), &text),
            ));
        }

        let mut array = toml_edit::Array::default();
        for child in children {
            array.push(child.as_str());
        }
        node_table["children"] = Item::Value(Value::Array(array));
        self.refresh_after_edit()
    }

    /// Adds a new node table with a default transform and kind-specific fields.
    pub fn add_node(
        &mut self,
        node: &NodeId,
        kind: SceneNodeDraft,
    ) -> Result<SceneDocument, SceneError> {
        let nodes_table = match self.document.get_mut("nodes").and_then(Item::as_table_mut) {
            Some(table) => table,
            None => return Err(SceneError::new(missing_field_error("nodes", None), None)),
        };
        if nodes_table.contains_key(node.as_str()) {
            return Err(SceneError::new(
                SceneErrorKind::InvalidIdentifier {
                    kind: "node",
                    value: format!("{} already exists", node.as_str()),
                },
                None,
            ));
        }

        let mut table = Table::new();
        table["kind"] = value(kind.kind_name());
        if let Some(label) = kind.default_label() {
            table["label"] = value(label);
        }
        for (key, item) in kind.kind_specific_items() {
            table[key] = item;
        }
        table["transform"] = default_transform_item();
        nodes_table.insert(node.as_str(), Item::Table(table));
        self.refresh_after_edit()
    }

    /// Renames a node and updates root/child references.
    pub fn rename_node(&mut self, from: &NodeId, to: &NodeId) -> Result<SceneDocument, SceneError> {
        if from == to {
            return self.validate();
        }

        let old_item = {
            let nodes_table = match self.document.get_mut("nodes").and_then(Item::as_table_mut) {
                Some(table) => table,
                None => return Err(SceneError::new(missing_field_error("nodes", None), None)),
            };
            if nodes_table.contains_key(to.as_str()) {
                return Err(SceneError::new(
                    SceneErrorKind::InvalidIdentifier {
                        kind: "node",
                        value: format!("{} already exists", to.as_str()),
                    },
                    None,
                ));
            }
            nodes_table.remove(from.as_str()).ok_or_else(|| {
                SceneError::new(SceneErrorKind::MissingNode { node: from.clone() }, None)
            })?
        };

        {
            let nodes_table = self
                .document
                .get_mut("nodes")
                .and_then(Item::as_table_mut)
                .expect("nodes table exists");
            nodes_table.insert(to.as_str(), old_item);

            for (_node_key, node_item) in nodes_table.iter_mut() {
                let Some(node_table) = node_item.as_table_mut() else {
                    continue;
                };
                let Some(children) = node_table.get_mut("children").and_then(Item::as_array_mut)
                else {
                    continue;
                };
                for child in children.iter_mut() {
                    if child.as_str().is_some_and(|value| value == from.as_str()) {
                        *child = Value::from(to.as_str());
                    }
                }
            }
        }

        let root_matches = self
            .document
            .get("root")
            .and_then(Item::as_str)
            .is_some_and(|current| current == from.as_str());
        if root_matches {
            self.document["root"] = value(to.as_str());
        }

        self.refresh_after_edit()
    }

    /// Duplicates an existing node table under a new ID.
    pub fn duplicate_node(
        &mut self,
        source: &NodeId,
        duplicate: &NodeId,
    ) -> Result<SceneDocument, SceneError> {
        let source_item = {
            let nodes_table = match self.document.get("nodes").and_then(Item::as_table) {
                Some(table) => table,
                None => return Err(SceneError::new(missing_field_error("nodes", None), None)),
            };
            if nodes_table.contains_key(duplicate.as_str()) {
                return Err(SceneError::new(
                    SceneErrorKind::InvalidIdentifier {
                        kind: "node",
                        value: format!("{} already exists", duplicate.as_str()),
                    },
                    None,
                ));
            }
            nodes_table.get(source.as_str()).cloned().ok_or_else(|| {
                SceneError::new(
                    SceneErrorKind::MissingNode {
                        node: source.clone(),
                    },
                    None,
                )
            })?
        };

        let nodes_table = self
            .document
            .get_mut("nodes")
            .and_then(Item::as_table_mut)
            .expect("nodes table exists");
        nodes_table.insert(duplicate.as_str(), source_item);
        self.refresh_after_edit()
    }

    /// Deletes a node table. Callers remain responsible for dependency policy.
    pub fn delete_node(&mut self, node: &NodeId) -> Result<SceneDocument, SceneError> {
        let nodes_table = match self.document.get_mut("nodes").and_then(Item::as_table_mut) {
            Some(table) => table,
            None => return Err(SceneError::new(missing_field_error("nodes", None), None)),
        };
        let removed = nodes_table.remove(node.as_str()).is_some();
        if !removed {
            return Err(SceneError::new(
                SceneErrorKind::MissingNode { node: node.clone() },
                None,
            ));
        }
        self.refresh_after_edit()
    }

    /// Returns a simple source hook for a node table header if present.
    pub fn node_source_location(&self, node: &NodeId) -> Option<SourceLocation> {
        find_header_location(&self.text, &format!("[nodes.{}]", node.as_str()))
    }

    /// Returns a simple source hook for a parameter table header if present.
    pub fn parameter_source_location(&self, parameter: &ParamId) -> Option<SourceLocation> {
        find_header_location(&self.text, &format!("[params.{}]", parameter.as_str()))
    }

    fn lookup_node_table_mut(&mut self, node: &NodeId) -> Result<&mut Table, SceneError> {
        let nodes_table = match self.document.get_mut("nodes").and_then(Item::as_table_mut) {
            Some(table) => table,
            None => return Err(SceneError::new(missing_field_error("nodes", None), None)),
        };
        match nodes_table
            .get_mut(node.as_str())
            .and_then(Item::as_table_mut)
        {
            Some(table) => Ok(table),
            None => Err(SceneError::new(
                SceneErrorKind::MissingNode { node: node.clone() },
                None,
            )),
        }
    }

    fn refresh_after_edit(&mut self) -> Result<SceneDocument, SceneError> {
        self.text = self.document.to_string();
        self.validate()
    }
}

/// A typed semantic scene document independent of TOML internals.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneDocument {
    schema_version: u32,
    root: NodeId,
    parameters: IndexMap<ParamId, ParameterDefinition>,
    nodes: IndexMap<NodeId, Node>,
    extensions: SceneExtensions,
}

impl SceneDocument {
    /// Returns the scene schema version.
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the root/output node ID.
    pub fn root(&self) -> &NodeId {
        &self.root
    }

    /// Returns the declared parameters.
    pub fn parameters(&self) -> &IndexMap<ParamId, ParameterDefinition> {
        &self.parameters
    }

    /// Returns the declared nodes.
    pub fn nodes(&self) -> &IndexMap<NodeId, Node> {
        &self.nodes
    }

    /// Returns the preserved extensible metadata.
    pub fn extensions(&self) -> &SceneExtensions {
        &self.extensions
    }

    /// Serializes the semantic document into canonical deterministic TOML.
    pub fn to_canonical_source(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("schema_version = {}\n", self.schema_version));
        output.push_str(&format!("root = \"{}\"\n", self.root));

        if !self.extensions.is_empty() {
            output.push('\n');
            output.push_str("[extensions]\n");
            write_extension_table(&mut output, &self.extensions, 0);
        }

        let mut parameter_ids: Vec<_> = self.parameters.keys().cloned().collect();
        parameter_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for parameter_id in parameter_ids {
            let parameter = &self.parameters[&parameter_id];
            output.push('\n');
            output.push_str(&format!("[params.{}]\n", parameter_id));
            output.push_str("type = \"scalar\"\n");
            output.push_str(&format!(
                "value = {}\n",
                format_scalar_literal(parameter.scalar_value())
            ));
            if !parameter.extensions.is_empty() {
                output.push_str(&format!(
                    "extensions = {}\n",
                    format_extension_value(&ExtensionValue::Table(
                        parameter.extensions.clone().into_inner(),
                    ))
                ));
            }
        }

        let mut node_ids: Vec<_> = self.nodes.keys().cloned().collect();
        node_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for node_id in node_ids {
            let node = &self.nodes[&node_id];
            output.push('\n');
            output.push_str(&format!("[nodes.{}]\n", node_id));
            output.push_str(&format!("kind = \"{}\"\n", node.kind.kind_name()));
            if let Some(label) = node.label() {
                output.push_str(&format!("label = \"{}\"\n", escape_string(label)));
            }
            match node.kind() {
                NodeKind::Box(primitive) => {
                    output.push_str(&format!(
                        "size = {{ x = {}, y = {}, z = {} }}\n",
                        format_scalar_expr(&primitive.size.x),
                        format_scalar_expr(&primitive.size.y),
                        format_scalar_expr(&primitive.size.z)
                    ));
                }
                NodeKind::Sphere(primitive) => {
                    output.push_str(&format!(
                        "radius = {}\n",
                        format_scalar_expr(&primitive.radius)
                    ));
                }
                NodeKind::Cylinder(primitive) => {
                    output.push_str(&format!(
                        "radius = {}\nheight = {}\n",
                        format_scalar_expr(&primitive.radius),
                        format_scalar_expr(&primitive.height)
                    ));
                }
                NodeKind::Capsule(primitive) => {
                    output.push_str(&format!(
                        "radius = {}\nheight = {}\n",
                        format_scalar_expr(&primitive.radius),
                        format_scalar_expr(&primitive.height)
                    ));
                }
                NodeKind::Plane(primitive) => {
                    output.push_str(&format!(
                        "width = {}\ndepth = {}\n",
                        format_scalar_expr(&primitive.width),
                        format_scalar_expr(&primitive.depth)
                    ));
                }
                NodeKind::Profile(primitive) => {
                    output.push_str(&format!(
                        "width = {}\nheight = {}\n",
                        format_scalar_expr(&primitive.width),
                        format_scalar_expr(&primitive.height)
                    ));
                }
                NodeKind::Union(composition)
                | NodeKind::Difference(composition)
                | NodeKind::Intersection(composition) => {
                    output.push_str("children = [");
                    for (index, child) in composition.children.iter().enumerate() {
                        if index > 0 {
                            output.push_str(", ");
                        }
                        output.push('"');
                        output.push_str(child.target().as_str());
                        output.push('"');
                    }
                    output.push_str("]\n");
                }
            }
            output.push_str(&format!(
                "transform = {{ translate = {{ x = {}, y = {}, z = {} }}, rotate_deg = {{ x = {}, y = {}, z = {} }}, scale = {{ x = {}, y = {}, z = {} }} }}\n",
                format_scalar_expr(&node.transform.translation.x),
                format_scalar_expr(&node.transform.translation.y),
                format_scalar_expr(&node.transform.translation.z),
                format_scalar_expr(&node.transform.rotation_deg.x),
                format_scalar_expr(&node.transform.rotation_deg.y),
                format_scalar_expr(&node.transform.rotation_deg.z),
                format_scalar_expr(&node.transform.scale.x),
                format_scalar_expr(&node.transform.scale.y),
                format_scalar_expr(&node.transform.scale.z)
            ));
            if !node.extensions.is_empty() {
                output.push_str(&format!(
                    "extensions = {}\n",
                    format_extension_value(&ExtensionValue::Table(
                        node.extensions.clone().into_inner(),
                    ))
                ));
            }
        }

        output
    }
}

/// Stable scene node identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(String);

impl NodeId {
    /// Creates a validated node identifier.
    pub fn new(raw: impl Into<String>) -> Result<Self, SceneError> {
        let raw = raw.into();
        validate_identifier("node", &raw, None, "")?;
        Ok(Self(raw))
    }

    /// Returns the identifier as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Stable parameter identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamId(String);

impl ParamId {
    /// Creates a validated parameter identifier.
    pub fn new(raw: impl Into<String>) -> Result<Self, SceneError> {
        let raw = raw.into();
        validate_identifier("parameter", &raw, None, "")?;
        Ok(Self(raw))
    }

    /// Returns the identifier as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ParamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A typed node reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeRef(NodeId);

impl NodeRef {
    /// Returns the referenced node ID.
    pub fn target(&self) -> &NodeId {
        &self.0
    }
}

/// A typed parameter reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParameterRef(ParamId);

impl ParameterRef {
    /// Returns the referenced parameter ID.
    pub fn target(&self) -> &ParamId {
        &self.0
    }
}

/// A scene node.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    id: NodeId,
    label: Option<String>,
    kind: NodeKind,
    transform: Transform,
    extensions: SceneExtensions,
}

impl Node {
    /// Returns the stable node ID.
    pub fn id(&self) -> &NodeId {
        &self.id
    }

    /// Returns the optional label.
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Returns the semantic node kind.
    pub fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// Returns the local transform.
    pub fn transform(&self) -> &Transform {
        &self.transform
    }

    /// Returns preserved node extension metadata.
    pub fn extensions(&self) -> &SceneExtensions {
        &self.extensions
    }
}

/// A reusable named parameter definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterDefinition {
    id: ParamId,
    value: ParameterValue,
    extensions: SceneExtensions,
}

impl ParameterDefinition {
    /// Returns the stable parameter ID.
    pub fn id(&self) -> &ParamId {
        &self.id
    }

    /// Returns the typed parameter value.
    pub fn value(&self) -> &ParameterValue {
        &self.value
    }

    /// Returns the scalar value for the current M02 parameter type set.
    pub fn scalar_value(&self) -> f64 {
        match self.value {
            ParameterValue::Scalar(value_number) => value_number,
        }
    }

    /// Returns preserved parameter extension metadata.
    pub fn extensions(&self) -> &SceneExtensions {
        &self.extensions
    }
}

/// The deliberately small initial parameter value set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParameterValue {
    Scalar(f64),
}

/// A scalar value used by primitives and transforms.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarExpr {
    Literal(f64),
    Parameter(ParameterRef),
}

/// An XYZ vector of scalar expressions.
#[derive(Debug, Clone, PartialEq)]
pub struct Vector3Expr {
    pub x: ScalarExpr,
    pub y: ScalarExpr,
    pub z: ScalarExpr,
}

impl Vector3Expr {
    fn identity_translation() -> Self {
        Self::literal(0.0, 0.0, 0.0)
    }

    fn identity_rotation() -> Self {
        Self::literal(0.0, 0.0, 0.0)
    }

    fn identity_scale() -> Self {
        Self::literal(1.0, 1.0, 1.0)
    }

    fn literal(x: f64, y: f64, z: f64) -> Self {
        Self {
            x: ScalarExpr::Literal(x),
            y: ScalarExpr::Literal(y),
            z: ScalarExpr::Literal(z),
        }
    }
}

/// Local transform with explicit translation, rotation, and scale.
#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    pub translation: Vector3Expr,
    pub rotation_deg: Vector3Expr,
    pub scale: Vector3Expr,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vector3Expr::identity_translation(),
            rotation_deg: Vector3Expr::identity_rotation(),
            scale: Vector3Expr::identity_scale(),
        }
    }
}

/// Supported semantic node kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Box(BoxPrimitive),
    Sphere(SpherePrimitive),
    Cylinder(CylinderPrimitive),
    Capsule(CapsulePrimitive),
    Plane(PlanePrimitive),
    Profile(ProfilePrimitive),
    Union(CompositionNode),
    Difference(CompositionNode),
    Intersection(CompositionNode),
}

impl NodeKind {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Box(_) => "box",
            Self::Sphere(_) => "sphere",
            Self::Cylinder(_) => "cylinder",
            Self::Capsule(_) => "capsule",
            Self::Plane(_) => "plane",
            Self::Profile(_) => "profile",
            Self::Union(_) => "union",
            Self::Difference(_) => "difference",
            Self::Intersection(_) => "intersection",
        }
    }
}

/// A box primitive declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxPrimitive {
    pub size: Vector3Expr,
}

/// A sphere primitive declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct SpherePrimitive {
    pub radius: ScalarExpr,
}

/// A cylinder primitive declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct CylinderPrimitive {
    pub radius: ScalarExpr,
    pub height: ScalarExpr,
}

/// A capsule primitive declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct CapsulePrimitive {
    pub radius: ScalarExpr,
    pub height: ScalarExpr,
}

/// A plane placeholder declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanePrimitive {
    pub width: ScalarExpr,
    pub depth: ScalarExpr,
}

/// A profile placeholder declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfilePrimitive {
    pub width: ScalarExpr,
    pub height: ScalarExpr,
}

/// A CSG composition declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct CompositionNode {
    pub children: Vec<NodeRef>,
}

/// Preserved extensible metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneExtensions(IndexMap<String, ExtensionValue>);

impl SceneExtensions {
    /// Returns `true` when no extensions are present.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the extension entries.
    pub fn entries(&self) -> &IndexMap<String, ExtensionValue> {
        &self.0
    }

    fn into_inner(self) -> IndexMap<String, ExtensionValue> {
        self.0
    }
}

/// Extension metadata values preserved by the schema layer.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionValue {
    String(String),
    Integer(i64),
    Float(f64),
    Bool(bool),
    Datetime(String),
    Array(Vec<ExtensionValue>),
    Table(IndexMap<String, ExtensionValue>),
}

/// A source location range with line and column data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

/// A lightweight source reveal hook for GUI callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub byte_offset: usize,
    pub line: usize,
    pub column: usize,
}

impl SourceSpan {
    fn from_range(source: &str, range: Range<usize>) -> Self {
        let (start_line, start_column) = line_column_at(source, range.start);
        let (end_line, end_column) = line_column_at(source, range.end);
        Self {
            start: range.start,
            end: range.end,
            start_line,
            start_column,
            end_line,
            end_column,
        }
    }
}

/// Public scene error wrapper.
#[derive(Debug, Clone)]
pub struct SceneError {
    kind: SceneErrorKind,
    span: Option<SourceSpan>,
}

impl SceneError {
    fn new(kind: SceneErrorKind, span: Option<SourceSpan>) -> Self {
        Self { kind, span }
    }

    /// Returns the structured error kind.
    pub fn kind(&self) -> &SceneErrorKind {
        &self.kind
    }

    /// Returns the optional source span.
    pub fn span(&self) -> Option<&SourceSpan> {
        self.span.as_ref()
    }
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(span) = &self.span {
            write!(
                f,
                "{} at line {}, column {}",
                self.kind, span.start_line, span.start_column
            )
        } else {
            self.kind.fmt(f)
        }
    }
}

impl Error for SceneError {}

/// Structured scene error categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneErrorKind {
    Parse {
        message: String,
    },
    UnsupportedSchemaVersion {
        found: u32,
        supported: u32,
    },
    MissingField {
        field: &'static str,
        context: Option<String>,
    },
    UnknownField {
        path: String,
    },
    InvalidIdentifier {
        kind: &'static str,
        value: String,
    },
    MissingRootNode {
        root: NodeId,
    },
    MissingNode {
        node: NodeId,
    },
    MissingParameter {
        parameter: ParamId,
    },
    InvalidNodeReference {
        owner: NodeId,
        target: String,
    },
    InvalidParameterReference {
        context: String,
        target: String,
    },
    InvalidValue {
        context: String,
        message: String,
    },
    InvalidEditTarget {
        message: String,
    },
    CycleDetected {
        node: NodeId,
    },
}

impl fmt::Display for SceneErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { message } => write!(f, "failed to parse scene TOML: {message}"),
            Self::UnsupportedSchemaVersion { found, supported } => write!(
                f,
                "unsupported scene schema version {found}; current supported version is {supported}"
            ),
            Self::MissingField { field, context } => {
                if let Some(context) = context {
                    write!(f, "missing required field `{field}` in `{context}`")
                } else {
                    write!(f, "missing required field `{field}`")
                }
            }
            Self::UnknownField { path } => write!(f, "unknown field `{path}`"),
            Self::InvalidIdentifier { kind, value } => {
                write!(f, "invalid {kind} identifier `{value}`")
            }
            Self::MissingRootNode { root } => {
                write!(f, "root node `{root}` does not exist")
            }
            Self::MissingNode { node } => write!(f, "node `{node}` does not exist"),
            Self::MissingParameter { parameter } => {
                write!(f, "parameter `{parameter}` does not exist")
            }
            Self::InvalidNodeReference { owner, target } => {
                write!(f, "node `{owner}` references missing node `{target}`")
            }
            Self::InvalidParameterReference { context, target } => {
                write!(f, "{context} references missing parameter `{target}`")
            }
            Self::InvalidValue { context, message } => {
                write!(f, "invalid {context}: {message}")
            }
            Self::InvalidEditTarget { message } => write!(f, "{message}"),
            Self::CycleDetected { node } => {
                write!(f, "cycle detected while resolving node `{node}`")
            }
        }
    }
}

/// A targeted editable primitive scalar field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveScalarField {
    BoxX,
    BoxY,
    BoxZ,
    SphereRadius,
    CylinderRadius,
    CylinderHeight,
    CapsuleRadius,
    CapsuleHeight,
    PlaneWidth,
    PlaneDepth,
    ProfileWidth,
    ProfileHeight,
}

/// A new scene node template used by M06 GUI creation.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneNodeDraft {
    Box,
    Sphere,
    Cylinder,
    Capsule,
    Plane,
    Profile,
    Union { children: Vec<NodeId> },
    Difference { children: Vec<NodeId> },
    Intersection { children: Vec<NodeId> },
}

impl SceneNodeDraft {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Sphere => "sphere",
            Self::Cylinder => "cylinder",
            Self::Capsule => "capsule",
            Self::Plane => "plane",
            Self::Profile => "profile",
            Self::Union { .. } => "union",
            Self::Difference { .. } => "difference",
            Self::Intersection { .. } => "intersection",
        }
    }

    fn default_label(&self) -> Option<&'static str> {
        None
    }

    fn kind_specific_items(&self) -> Vec<(&'static str, Item)> {
        match self {
            Self::Box => vec![("size", default_xyz_inline_item(1.0, 1.0, 1.0))],
            Self::Sphere => vec![("radius", value(0.5))],
            Self::Cylinder => vec![("radius", value(0.5)), ("height", value(1.0))],
            Self::Capsule => vec![("radius", value(0.25)), ("height", value(1.0))],
            Self::Plane => vec![("width", value(1.0)), ("depth", value(1.0))],
            Self::Profile => vec![("width", value(1.0)), ("height", value(1.0))],
            Self::Union { children }
            | Self::Difference { children }
            | Self::Intersection { children } => {
                let mut array = toml_edit::Array::default();
                for child in children {
                    array.push(child.as_str());
                }
                vec![("children", Item::Value(Value::Array(array)))]
            }
        }
    }
}

impl PrimitiveScalarField {
    fn as_str(self) -> &'static str {
        match self {
            Self::BoxX => "box.size.x",
            Self::BoxY => "box.size.y",
            Self::BoxZ => "box.size.z",
            Self::SphereRadius => "sphere.radius",
            Self::CylinderRadius => "cylinder.radius",
            Self::CylinderHeight => "cylinder.height",
            Self::CapsuleRadius => "capsule.radius",
            Self::CapsuleHeight => "capsule.height",
            Self::PlaneWidth => "plane.width",
            Self::PlaneDepth => "plane.depth",
            Self::ProfileWidth => "profile.width",
            Self::ProfileHeight => "profile.height",
        }
    }

    fn validation_label(self) -> &'static str {
        match self {
            Self::SphereRadius | Self::CylinderRadius | Self::CapsuleRadius => "radius",
            Self::CylinderHeight | Self::CapsuleHeight | Self::ProfileHeight => "height",
            Self::PlaneDepth => "depth",
            Self::PlaneWidth | Self::ProfileWidth => "width",
            Self::BoxX => "box x dimension",
            Self::BoxY => "box y dimension",
            Self::BoxZ => "box z dimension",
        }
    }

    fn toml_path_for_kind(self, kind: &str) -> Option<(&'static str, Option<&'static str>)> {
        match (kind, self) {
            ("box", Self::BoxX) => Some(("size", Some("x"))),
            ("box", Self::BoxY) => Some(("size", Some("y"))),
            ("box", Self::BoxZ) => Some(("size", Some("z"))),
            ("sphere", Self::SphereRadius) => Some(("radius", None)),
            ("cylinder", Self::CylinderRadius) => Some(("radius", None)),
            ("cylinder", Self::CylinderHeight) => Some(("height", None)),
            ("capsule", Self::CapsuleRadius) => Some(("radius", None)),
            ("capsule", Self::CapsuleHeight) => Some(("height", None)),
            ("plane", Self::PlaneWidth) => Some(("width", None)),
            ("plane", Self::PlaneDepth) => Some(("depth", None)),
            ("profile", Self::ProfileWidth) => Some(("width", None)),
            ("profile", Self::ProfileHeight) => Some(("height", None)),
            _ => None,
        }
    }
}

/// A targeted editable transform property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformProperty {
    Translation,
    RotationDegrees,
    Scale,
}

impl TransformProperty {
    fn key(self) -> &'static str {
        match self {
            Self::Translation => "translate",
            Self::RotationDegrees => "rotate_deg",
            Self::Scale => "scale",
        }
    }
}

/// One transform axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    fn key(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
        }
    }
}

fn validate_scene_source(
    document: &DocumentMut,
    source: &str,
) -> Result<SceneDocument, SceneError> {
    validate_allowed_fields(
        document.iter(),
        &["schema_version", "root", "params", "nodes", "extensions"],
        source,
        None,
    )?;

    let schema_version = parse_u32_item(
        document.get("schema_version").ok_or_else(|| {
            SceneError::new(
                SceneErrorKind::MissingField {
                    field: "schema_version",
                    context: None,
                },
                None,
            )
        })?,
        "schema_version",
        source,
    )?;
    if schema_version != SCENE_SCHEMA_VERSION {
        return Err(SceneError::new(
            SceneErrorKind::UnsupportedSchemaVersion {
                found: schema_version,
                supported: SCENE_SCHEMA_VERSION,
            },
            span_for_item(document.get("schema_version"), source),
        ));
    }

    let root_item = document.get("root").ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::MissingField {
                field: "root",
                context: None,
            },
            None,
        )
    })?;
    let root_text = parse_string_item(root_item, "root", source)?;
    let root_id = parse_node_id(&root_text, span_for_item(Some(root_item), source), source)?;

    let extensions = parse_extensions_item(document.get("extensions"), source)?;
    let parameters = parse_parameters(document.get("params"), source)?;
    let nodes = parse_nodes(document.get("nodes"), source, &parameters)?;

    if !nodes.contains_key(&root_id) {
        return Err(SceneError::new(
            SceneErrorKind::MissingRootNode {
                root: root_id.clone(),
            },
            span_for_item(Some(root_item), source),
        ));
    }

    validate_node_references(document.get("nodes"), &nodes, source)?;

    Ok(SceneDocument {
        schema_version,
        root: root_id,
        parameters,
        nodes,
        extensions,
    })
}

fn parse_parameters(
    params_item: Option<&Item>,
    source: &str,
) -> Result<IndexMap<ParamId, ParameterDefinition>, SceneError> {
    let Some(params_item) = params_item else {
        return Ok(IndexMap::new());
    };
    let params_table = params_item.as_table().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: "params".to_owned(),
                message: "expected `[params]` table".to_owned(),
            },
            span_for_item(Some(params_item), source),
        )
    })?;

    let mut parameters = IndexMap::new();
    for (param_key, param_item) in params_table.iter() {
        let param_id = parse_param_id(param_key, span_for_item(Some(param_item), source), source)?;
        let param_table = param_item.as_table().ok_or_else(|| {
            SceneError::new(
                SceneErrorKind::InvalidValue {
                    context: format!("param `{param_key}`"),
                    message: "expected parameter table".to_owned(),
                },
                span_for_item(Some(param_item), source),
            )
        })?;
        validate_allowed_fields(
            param_table.iter(),
            &["type", "value", "extensions"],
            source,
            Some(&format!("params.{param_key}")),
        )?;

        let type_item = param_table.get("type").ok_or_else(|| {
            SceneError::new(
                SceneErrorKind::MissingField {
                    field: "type",
                    context: Some(format!("params.{param_key}")),
                },
                None,
            )
        })?;
        let param_type = parse_string_item(type_item, "type", source)?;
        if param_type != "scalar" {
            return Err(SceneError::new(
                SceneErrorKind::InvalidValue {
                    context: format!("params.{param_key}.type"),
                    message: format!("unsupported parameter type `{param_type}`"),
                },
                span_for_item(Some(type_item), source),
            ));
        }

        let value_item = param_table.get("value").ok_or_else(|| {
            SceneError::new(
                SceneErrorKind::MissingField {
                    field: "value",
                    context: Some(format!("params.{param_key}")),
                },
                None,
            )
        })?;
        let scalar =
            parse_numeric_literal(value_item, &format!("params.{param_key}.value"), source)?;
        ensure_finite_scalar(
            scalar,
            &format!("params.{param_key}.value"),
            span_for_item(Some(value_item), source),
            source,
        )?;
        let extensions = parse_extensions_item(param_table.get("extensions"), source)?;

        parameters.insert(
            param_id.clone(),
            ParameterDefinition {
                id: param_id,
                value: ParameterValue::Scalar(scalar),
                extensions,
            },
        );
    }

    Ok(parameters)
}

fn parse_nodes(
    nodes_item: Option<&Item>,
    source: &str,
    parameters: &IndexMap<ParamId, ParameterDefinition>,
) -> Result<IndexMap<NodeId, Node>, SceneError> {
    let nodes_item = nodes_item.ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::MissingField {
                field: "nodes",
                context: None,
            },
            None,
        )
    })?;
    let nodes_table = nodes_item.as_table().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: "nodes".to_owned(),
                message: "expected `[nodes]` table".to_owned(),
            },
            span_for_item(Some(nodes_item), source),
        )
    })?;

    let mut nodes = IndexMap::new();
    for (node_key, node_item) in nodes_table.iter() {
        let node_id = parse_node_id(node_key, span_for_item(Some(node_item), source), source)?;
        let node_table = node_item.as_table().ok_or_else(|| {
            SceneError::new(
                SceneErrorKind::InvalidValue {
                    context: format!("nodes.{node_key}"),
                    message: "expected node table".to_owned(),
                },
                span_for_item(Some(node_item), source),
            )
        })?;
        let kind_item = node_table.get("kind").ok_or_else(|| {
            SceneError::new(
                SceneErrorKind::MissingField {
                    field: "kind",
                    context: Some(format!("nodes.{node_key}")),
                },
                None,
            )
        })?;
        let kind_name = parse_string_item(kind_item, "kind", source)?;
        let label = match node_table.get("label") {
            Some(item) => Some(parse_string_item(item, "label", source)?),
            None => None,
        };
        let transform = parse_transform(node_table.get("transform"), source, parameters, node_key)?;
        let extensions = parse_extensions_item(node_table.get("extensions"), source)?;
        let context = format!("nodes.{node_key}");
        let kind = match kind_name.as_str() {
            "box" => {
                validate_allowed_fields(
                    node_table.iter(),
                    &["kind", "label", "size", "transform", "extensions"],
                    source,
                    Some(&context),
                )?;
                NodeKind::Box(BoxPrimitive {
                    size: parse_vector3_inline(
                        node_table.get("size"),
                        source,
                        parameters,
                        &context,
                        "size",
                        PositiveRule::StrictlyPositive,
                    )?,
                })
            }
            "sphere" => {
                validate_allowed_fields(
                    node_table.iter(),
                    &["kind", "label", "radius", "transform", "extensions"],
                    source,
                    Some(&context),
                )?;
                NodeKind::Sphere(SpherePrimitive {
                    radius: parse_scalar_expr(
                        node_table.get("radius"),
                        source,
                        parameters,
                        &format!("{context}.radius"),
                        PositiveRule::StrictlyPositive,
                    )?,
                })
            }
            "cylinder" => {
                validate_allowed_fields(
                    node_table.iter(),
                    &[
                        "kind",
                        "label",
                        "radius",
                        "height",
                        "transform",
                        "extensions",
                    ],
                    source,
                    Some(&context),
                )?;
                NodeKind::Cylinder(CylinderPrimitive {
                    radius: parse_scalar_expr(
                        node_table.get("radius"),
                        source,
                        parameters,
                        &format!("{context}.radius"),
                        PositiveRule::StrictlyPositive,
                    )?,
                    height: parse_scalar_expr(
                        node_table.get("height"),
                        source,
                        parameters,
                        &format!("{context}.height"),
                        PositiveRule::StrictlyPositive,
                    )?,
                })
            }
            "capsule" => {
                validate_allowed_fields(
                    node_table.iter(),
                    &[
                        "kind",
                        "label",
                        "radius",
                        "height",
                        "transform",
                        "extensions",
                    ],
                    source,
                    Some(&context),
                )?;
                NodeKind::Capsule(CapsulePrimitive {
                    radius: parse_scalar_expr(
                        node_table.get("radius"),
                        source,
                        parameters,
                        &format!("{context}.radius"),
                        PositiveRule::StrictlyPositive,
                    )?,
                    height: parse_scalar_expr(
                        node_table.get("height"),
                        source,
                        parameters,
                        &format!("{context}.height"),
                        PositiveRule::StrictlyPositive,
                    )?,
                })
            }
            "plane" => {
                validate_allowed_fields(
                    node_table.iter(),
                    &["kind", "label", "width", "depth", "transform", "extensions"],
                    source,
                    Some(&context),
                )?;
                NodeKind::Plane(PlanePrimitive {
                    width: parse_scalar_expr(
                        node_table.get("width"),
                        source,
                        parameters,
                        &format!("{context}.width"),
                        PositiveRule::StrictlyPositive,
                    )?,
                    depth: parse_scalar_expr(
                        node_table.get("depth"),
                        source,
                        parameters,
                        &format!("{context}.depth"),
                        PositiveRule::StrictlyPositive,
                    )?,
                })
            }
            "profile" => {
                validate_allowed_fields(
                    node_table.iter(),
                    &[
                        "kind",
                        "label",
                        "width",
                        "height",
                        "transform",
                        "extensions",
                    ],
                    source,
                    Some(&context),
                )?;
                NodeKind::Profile(ProfilePrimitive {
                    width: parse_scalar_expr(
                        node_table.get("width"),
                        source,
                        parameters,
                        &format!("{context}.width"),
                        PositiveRule::StrictlyPositive,
                    )?,
                    height: parse_scalar_expr(
                        node_table.get("height"),
                        source,
                        parameters,
                        &format!("{context}.height"),
                        PositiveRule::StrictlyPositive,
                    )?,
                })
            }
            "union" | "difference" | "intersection" => {
                validate_allowed_fields(
                    node_table.iter(),
                    &["kind", "label", "children", "transform", "extensions"],
                    source,
                    Some(&context),
                )?;
                let composition = parse_children(node_table.get("children"), source, node_key)?;
                match kind_name.as_str() {
                    "union" => NodeKind::Union(composition),
                    "difference" => NodeKind::Difference(composition),
                    _ => NodeKind::Intersection(composition),
                }
            }
            other => {
                return Err(SceneError::new(
                    SceneErrorKind::InvalidValue {
                        context: format!("nodes.{node_key}.kind"),
                        message: format!("unsupported node kind `{other}`"),
                    },
                    span_for_item(Some(kind_item), source),
                ));
            }
        };

        nodes.insert(
            node_id.clone(),
            Node {
                id: node_id,
                label,
                kind,
                transform,
                extensions,
            },
        );
    }

    Ok(nodes)
}

fn validate_node_references(
    nodes_item: Option<&Item>,
    nodes: &IndexMap<NodeId, Node>,
    source: &str,
) -> Result<(), SceneError> {
    let node_lookup: IndexMap<String, NodeId> = nodes
        .keys()
        .cloned()
        .map(|node| (node.as_str().to_owned(), node))
        .collect();
    let nodes_table = nodes_item.and_then(Item::as_table);

    for node in nodes.values() {
        match node.kind() {
            NodeKind::Union(composition)
            | NodeKind::Difference(composition)
            | NodeKind::Intersection(composition) => {
                if composition.children.len() < 2 {
                    let span = nodes_table
                        .and_then(|table| table.get(node.id().as_str()))
                        .and_then(|item| item.as_table())
                        .and_then(|table| span_for_item(table.get("children"), source));
                    return Err(SceneError::new(
                        SceneErrorKind::InvalidValue {
                            context: format!("nodes.{}.children", node.id()),
                            message: "composition nodes require at least two child references"
                                .to_owned(),
                        },
                        span.or_else(|| context_span(source, "children")),
                    ));
                }
                for child in &composition.children {
                    if !node_lookup.contains_key(child.target().as_str()) {
                        let span = nodes_table
                            .and_then(|table| table.get(node.id().as_str()))
                            .and_then(|item| item.as_table())
                            .and_then(|table| span_for_item(table.get("children"), source));
                        return Err(SceneError::new(
                            SceneErrorKind::InvalidNodeReference {
                                owner: node.id().clone(),
                                target: child.target().as_str().to_owned(),
                            },
                            span.or_else(|| context_span(source, "children")),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    let mut visiting: Vec<NodeId> = Vec::new();
    let mut visit_state: IndexMap<NodeId, VisitState> = nodes
        .keys()
        .cloned()
        .map(|node| (node, VisitState::NotVisited))
        .collect();

    for node_id in nodes.keys() {
        detect_cycles(node_id, nodes, &mut visit_state, &mut visiting)?;
    }

    Ok(())
}

fn detect_cycles(
    node_id: &NodeId,
    nodes: &IndexMap<NodeId, Node>,
    visit_state: &mut IndexMap<NodeId, VisitState>,
    visiting: &mut Vec<NodeId>,
) -> Result<(), SceneError> {
    match visit_state.get(node_id) {
        Some(VisitState::Visiting) => {
            return Err(SceneError::new(
                SceneErrorKind::CycleDetected {
                    node: node_id.clone(),
                },
                None,
            ));
        }
        Some(VisitState::Visited) => return Ok(()),
        _ => {}
    }

    visit_state.insert(node_id.clone(), VisitState::Visiting);
    visiting.push(node_id.clone());

    if let Some(node) = nodes.get(node_id) {
        match node.kind() {
            NodeKind::Union(composition)
            | NodeKind::Difference(composition)
            | NodeKind::Intersection(composition) => {
                for child in &composition.children {
                    detect_cycles(child.target(), nodes, visit_state, visiting)?;
                }
            }
            _ => {}
        }
    }

    visiting.pop();
    visit_state.insert(node_id.clone(), VisitState::Visited);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    NotVisited,
    Visiting,
    Visited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PositiveRule {
    AnyFinite,
    StrictlyPositive,
}

fn parse_transform(
    transform_item: Option<&Item>,
    source: &str,
    parameters: &IndexMap<ParamId, ParameterDefinition>,
    node_key: &str,
) -> Result<Transform, SceneError> {
    let Some(transform_item) = transform_item else {
        return Ok(Transform::default());
    };
    let transform_table = transform_item.as_inline_table().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: format!("nodes.{node_key}.transform"),
                message: "expected inline table for transform".to_owned(),
            },
            span_for_item(Some(transform_item), source),
        )
    })?;
    validate_allowed_fields(
        transform_table.iter(),
        &["translate", "rotate_deg", "scale"],
        source,
        Some(&format!("nodes.{node_key}.transform")),
    )?;

    let translation = parse_vector3_inline_from_inline(
        transform_table.get("translate"),
        source,
        parameters,
        &format!("nodes.{node_key}.transform.translate"),
        PositiveRule::AnyFinite,
    )?
    .unwrap_or_else(Vector3Expr::identity_translation);
    let rotation_deg = parse_vector3_inline_from_inline(
        transform_table.get("rotate_deg"),
        source,
        parameters,
        &format!("nodes.{node_key}.transform.rotate_deg"),
        PositiveRule::AnyFinite,
    )?
    .unwrap_or_else(Vector3Expr::identity_rotation);
    let scale = parse_vector3_inline_from_inline(
        transform_table.get("scale"),
        source,
        parameters,
        &format!("nodes.{node_key}.transform.scale"),
        PositiveRule::StrictlyPositive,
    )?
    .unwrap_or_else(Vector3Expr::identity_scale);

    Ok(Transform {
        translation,
        rotation_deg,
        scale,
    })
}

fn parse_vector3_inline(
    item: Option<&Item>,
    source: &str,
    parameters: &IndexMap<ParamId, ParameterDefinition>,
    context: &str,
    field_name: &'static str,
    positive_rule: PositiveRule,
) -> Result<Vector3Expr, SceneError> {
    let item = item.ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::MissingField {
                field: field_name,
                context: Some(context.to_owned()),
            },
            None,
        )
    })?;
    let inline = item.as_inline_table().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: format!("{context}.{field_name}"),
                message: "expected inline table".to_owned(),
            },
            span_for_item(Some(item), source),
        )
    })?;
    parse_xyz_table(
        inline,
        source,
        parameters,
        &format!("{context}.{field_name}"),
        positive_rule,
    )
}

fn parse_vector3_inline_from_inline(
    item: Option<&Value>,
    source: &str,
    parameters: &IndexMap<ParamId, ParameterDefinition>,
    context: &str,
    positive_rule: PositiveRule,
) -> Result<Option<Vector3Expr>, SceneError> {
    let Some(item) = item else {
        return Ok(None);
    };
    let inline = item.as_inline_table().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "expected inline table".to_owned(),
            },
            span_for_value(item, source),
        )
    })?;
    Ok(Some(parse_xyz_table(
        inline,
        source,
        parameters,
        context,
        positive_rule,
    )?))
}

fn parse_xyz_table(
    inline: &InlineTable,
    source: &str,
    parameters: &IndexMap<ParamId, ParameterDefinition>,
    context: &str,
    positive_rule: PositiveRule,
) -> Result<Vector3Expr, SceneError> {
    validate_allowed_fields(inline.iter(), &["x", "y", "z"], source, Some(context))?;
    Ok(Vector3Expr {
        x: parse_scalar_value(
            inline.get("x"),
            source,
            parameters,
            &format!("{context}.x"),
            positive_rule,
        )?,
        y: parse_scalar_value(
            inline.get("y"),
            source,
            parameters,
            &format!("{context}.y"),
            positive_rule,
        )?,
        z: parse_scalar_value(
            inline.get("z"),
            source,
            parameters,
            &format!("{context}.z"),
            positive_rule,
        )?,
    })
}

fn parse_scalar_expr(
    item: Option<&Item>,
    source: &str,
    parameters: &IndexMap<ParamId, ParameterDefinition>,
    context: &str,
    positive_rule: PositiveRule,
) -> Result<ScalarExpr, SceneError> {
    let item = item.ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "missing required scalar field".to_owned(),
            },
            None,
        )
    })?;
    parse_scalar_item(item, source, parameters, context, positive_rule)
}

fn parse_scalar_value(
    value: Option<&Value>,
    source: &str,
    parameters: &IndexMap<ParamId, ParameterDefinition>,
    context: &str,
    positive_rule: PositiveRule,
) -> Result<ScalarExpr, SceneError> {
    let value = value.ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "missing required scalar field".to_owned(),
            },
            None,
        )
    })?;
    parse_scalar_value_inner(value, source, parameters, context, positive_rule)
}

fn parse_scalar_item(
    item: &Item,
    source: &str,
    parameters: &IndexMap<ParamId, ParameterDefinition>,
    context: &str,
    positive_rule: PositiveRule,
) -> Result<ScalarExpr, SceneError> {
    let value = item.as_value().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "expected scalar literal or `{ param = \"...\" }` reference".to_owned(),
            },
            span_for_item(Some(item), source),
        )
    })?;
    parse_scalar_value_inner(value, source, parameters, context, positive_rule)
}

fn parse_scalar_value_inner(
    value: &Value,
    source: &str,
    parameters: &IndexMap<ParamId, ParameterDefinition>,
    context: &str,
    positive_rule: PositiveRule,
) -> Result<ScalarExpr, SceneError> {
    if let Some(float_value) = value.as_float() {
        validate_positive_rule(
            float_value,
            positive_rule,
            context,
            span_for_value(value, source),
            source,
        )?;
        return Ok(ScalarExpr::Literal(float_value));
    }
    if let Some(integer_value) = value.as_integer() {
        let float_value = integer_value as f64;
        validate_positive_rule(
            float_value,
            positive_rule,
            context,
            span_for_value(value, source),
            source,
        )?;
        return Ok(ScalarExpr::Literal(float_value));
    }
    if let Some(inline) = value.as_inline_table() {
        validate_allowed_fields(inline.iter(), &["param"], source, Some(context))?;
        let param_value = inline.get("param").ok_or_else(|| {
            SceneError::new(
                SceneErrorKind::MissingField {
                    field: "param",
                    context: Some(context.to_owned()),
                },
                span_for_value(value, source),
            )
        })?;
        let param_name = param_value.as_str().ok_or_else(|| {
            SceneError::new(
                SceneErrorKind::InvalidValue {
                    context: context.to_owned(),
                    message: "parameter references must use a string identifier".to_owned(),
                },
                span_for_value(param_value, source),
            )
        })?;
        let param_id = parse_param_id(param_name, span_for_value(param_value, source), source)?;
        if !parameters.contains_key(&param_id) {
            return Err(SceneError::new(
                SceneErrorKind::InvalidParameterReference {
                    context: context.to_owned(),
                    target: param_name.to_owned(),
                },
                span_for_value(param_value, source),
            ));
        }
        return Ok(ScalarExpr::Parameter(ParameterRef(param_id)));
    }

    Err(SceneError::new(
        SceneErrorKind::InvalidValue {
            context: context.to_owned(),
            message: "expected number or `{ param = \"...\" }` reference".to_owned(),
        },
        span_for_value(value, source),
    ))
}

fn parse_children(
    item: Option<&Item>,
    source: &str,
    node_key: &str,
) -> Result<CompositionNode, SceneError> {
    let item = item.ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::MissingField {
                field: "children",
                context: Some(format!("nodes.{node_key}")),
            },
            None,
        )
    })?;
    let array = item.as_array().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: format!("nodes.{node_key}.children"),
                message: "expected array of node IDs".to_owned(),
            },
            span_for_item(Some(item), source),
        )
    })?;

    let mut children = Vec::new();
    for value in array.iter() {
        let child_name = value.as_str().ok_or_else(|| {
            SceneError::new(
                SceneErrorKind::InvalidValue {
                    context: format!("nodes.{node_key}.children"),
                    message: "child references must be strings".to_owned(),
                },
                span_for_value(value, source),
            )
        })?;
        let child_id = parse_node_id(child_name, span_for_value(value, source), source)?;
        children.push(NodeRef(child_id));
    }

    Ok(CompositionNode { children })
}

fn parse_extensions_item(item: Option<&Item>, source: &str) -> Result<SceneExtensions, SceneError> {
    let Some(item) = item else {
        return Ok(SceneExtensions::default());
    };

    let table = if let Some(table) = item.as_table() {
        table
            .iter()
            .map(|(key, item)| {
                item_to_extension_value(item, source).map(|value| (key.to_owned(), value))
            })
            .collect::<Result<IndexMap<_, _>, _>>()?
    } else if let Some(inline) = item.as_inline_table() {
        inline
            .iter()
            .map(|(key, value)| {
                value_to_extension_value(value, source).map(|value| (key.to_owned(), value))
            })
            .collect::<Result<IndexMap<_, _>, _>>()?
    } else {
        return Err(SceneError::new(
            SceneErrorKind::InvalidValue {
                context: "extensions".to_owned(),
                message: "extensions must be a table or inline table".to_owned(),
            },
            span_for_item(Some(item), source),
        ));
    };

    Ok(SceneExtensions(table))
}

fn item_to_extension_value(item: &Item, source: &str) -> Result<ExtensionValue, SceneError> {
    if let Some(value) = item.as_value() {
        value_to_extension_value(value, source)
    } else if let Some(table) = item.as_table() {
        let mut entries = IndexMap::new();
        for (key, child) in table.iter() {
            entries.insert(key.to_owned(), item_to_extension_value(child, source)?);
        }
        Ok(ExtensionValue::Table(entries))
    } else {
        Err(SceneError::new(
            SceneErrorKind::InvalidValue {
                context: "extensions".to_owned(),
                message: "unsupported extension value".to_owned(),
            },
            span_for_item(Some(item), source),
        ))
    }
}

fn value_to_extension_value(value: &Value, source: &str) -> Result<ExtensionValue, SceneError> {
    if let Some(string) = value.as_str() {
        return Ok(ExtensionValue::String(string.to_owned()));
    }
    if let Some(integer) = value.as_integer() {
        return Ok(ExtensionValue::Integer(integer));
    }
    if let Some(float_value) = value.as_float() {
        return Ok(ExtensionValue::Float(float_value));
    }
    if let Some(boolean) = value.as_bool() {
        return Ok(ExtensionValue::Bool(boolean));
    }
    if let Some(datetime) = value.as_datetime() {
        return Ok(ExtensionValue::Datetime(datetime.to_string()));
    }
    if let Some(array) = value.as_array() {
        let mut values = Vec::new();
        for child in array.iter() {
            values.push(value_to_extension_value(child, source)?);
        }
        return Ok(ExtensionValue::Array(values));
    }
    if let Some(table) = value.as_inline_table() {
        let mut entries = IndexMap::new();
        for (key, child) in table.iter() {
            entries.insert(key.to_owned(), value_to_extension_value(child, source)?);
        }
        return Ok(ExtensionValue::Table(entries));
    }
    Err(SceneError::new(
        SceneErrorKind::InvalidValue {
            context: "extensions".to_owned(),
            message: "unsupported extension value".to_owned(),
        },
        span_for_value(value, source),
    ))
}

fn validate_allowed_fields<'a>(
    iter: impl Iterator<Item = (&'a str, impl Sized)>,
    allowed: &[&str],
    source: &str,
    context: Option<&str>,
) -> Result<(), SceneError> {
    for (key, _) in iter {
        if !allowed.contains(&key) {
            let path = context
                .map(|context| format!("{context}.{key}"))
                .unwrap_or_else(|| key.to_owned());
            return Err(SceneError::new(
                SceneErrorKind::UnknownField { path },
                find_key_span(source, key),
            ));
        }
    }
    Ok(())
}

fn parse_string_item(item: &Item, context: &str, source: &str) -> Result<String, SceneError> {
    let value = item.as_value().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "expected string".to_owned(),
            },
            span_for_item(Some(item), source),
        )
    })?;
    let string = value.as_str().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "expected string".to_owned(),
            },
            span_for_value(value, source),
        )
    })?;
    Ok(string.to_owned())
}

fn parse_u32_item(item: &Item, context: &str, source: &str) -> Result<u32, SceneError> {
    let value = item.as_value().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "expected integer".to_owned(),
            },
            span_for_item(Some(item), source),
        )
    })?;
    let integer = value.as_integer().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "expected integer".to_owned(),
            },
            span_for_value(value, source),
        )
    })?;
    u32::try_from(integer).map_err(|_| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "expected non-negative 32-bit integer".to_owned(),
            },
            span_for_value(value, source),
        )
    })
}

fn parse_numeric_literal(item: &Item, context: &str, source: &str) -> Result<f64, SceneError> {
    let value = item.as_value().ok_or_else(|| {
        SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "expected numeric literal".to_owned(),
            },
            span_for_item(Some(item), source),
        )
    })?;
    if let Some(float_value) = value.as_float() {
        return Ok(float_value);
    }
    if let Some(integer_value) = value.as_integer() {
        return Ok(integer_value as f64);
    }
    Err(SceneError::new(
        SceneErrorKind::InvalidValue {
            context: context.to_owned(),
            message: "expected numeric literal".to_owned(),
        },
        span_for_value(value, source),
    ))
}

fn parse_node_id(raw: &str, span: Option<SourceSpan>, source: &str) -> Result<NodeId, SceneError> {
    validate_identifier("node", raw, span.clone(), source)?;
    Ok(NodeId(raw.to_owned()))
}

fn parse_param_id(
    raw: &str,
    span: Option<SourceSpan>,
    source: &str,
) -> Result<ParamId, SceneError> {
    validate_identifier("parameter", raw, span.clone(), source)?;
    Ok(ParamId(raw.to_owned()))
}

fn validate_identifier(
    kind: &'static str,
    raw: &str,
    span: Option<SourceSpan>,
    _source: &str,
) -> Result<(), SceneError> {
    let mut chars = raw.chars();
    let Some(first) = chars.next() else {
        return Err(SceneError::new(
            SceneErrorKind::InvalidIdentifier {
                kind,
                value: raw.to_owned(),
            },
            span,
        ));
    };

    if !first.is_ascii_alphabetic() {
        return Err(SceneError::new(
            SceneErrorKind::InvalidIdentifier {
                kind,
                value: raw.to_owned(),
            },
            span,
        ));
    }

    if chars.any(|character| {
        !(character.is_ascii_alphanumeric() || character == '_' || character == '-')
    }) {
        return Err(SceneError::new(
            SceneErrorKind::InvalidIdentifier {
                kind,
                value: raw.to_owned(),
            },
            span,
        ));
    }

    Ok(())
}

fn missing_field_error(field: &'static str, context: Option<String>) -> SceneErrorKind {
    SceneErrorKind::MissingField { field, context }
}

fn validate_positive_rule(
    value_number: f64,
    positive_rule: PositiveRule,
    context: &str,
    span: Option<SourceSpan>,
    source: &str,
) -> Result<(), SceneError> {
    match positive_rule {
        PositiveRule::AnyFinite => ensure_finite_scalar(value_number, context, span, source),
        PositiveRule::StrictlyPositive => {
            ensure_positive_scalar(value_number, context, span, source)
        }
    }
}

fn ensure_finite_scalar(
    value_number: f64,
    context: &str,
    span: Option<SourceSpan>,
    source: &str,
) -> Result<(), SceneError> {
    if value_number.is_finite() {
        Ok(())
    } else {
        Err(SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "value must be finite".to_owned(),
            },
            span.or_else(|| context_span(source, context_leaf(context))),
        ))
    }
}

fn ensure_positive_scalar(
    value_number: f64,
    context: &str,
    span: Option<SourceSpan>,
    source: &str,
) -> Result<(), SceneError> {
    ensure_finite_scalar(value_number, context, span.clone(), source)?;
    if value_number > 0.0 {
        Ok(())
    } else {
        Err(SceneError::new(
            SceneErrorKind::InvalidValue {
                context: context.to_owned(),
                message: "value must be strictly positive".to_owned(),
            },
            span.or_else(|| context_span(source, context_leaf(context))),
        ))
    }
}

fn span_for_item(item: Option<&Item>, source: &str) -> Option<SourceSpan> {
    item.and_then(Item::span)
        .map(|range| SourceSpan::from_range(source, range))
}

fn span_for_value(value: &Value, source: &str) -> Option<SourceSpan> {
    value
        .span()
        .map(|range| SourceSpan::from_range(source, range))
}

fn find_key_span(source: &str, key: &str) -> Option<SourceSpan> {
    source
        .find(key)
        .map(|start| SourceSpan::from_range(source, start..start + key.len()))
}

fn context_leaf(context: &str) -> &str {
    context.rsplit('.').next().unwrap_or(context)
}

fn context_span(source: &str, context: &str) -> Option<SourceSpan> {
    find_key_span(source, context)
}

fn replace_item_value_preserving_decor(item: &mut Item, value_number: f64) {
    let existing_decor = item.as_value().map(|value| value.decor().clone());
    let mut new_value = toml_edit::Value::from(value_number);
    if let Some(decor) = existing_decor {
        *new_value.decor_mut() = decor;
    }
    *item = Item::Value(new_value);
}

fn line_column_at(source: &str, byte_index: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for (index, character) in source.char_indices() {
        if index >= byte_index {
            break;
        }
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn format_scalar_expr(expression: &ScalarExpr) -> String {
    match expression {
        ScalarExpr::Literal(value_number) => format_scalar_literal(*value_number),
        ScalarExpr::Parameter(parameter) => {
            format!("{{ param = \"{}\" }}", parameter.target())
        }
    }
}

fn format_scalar_literal(value_number: f64) -> String {
    let mut formatted = value_number.to_string();
    if !formatted.contains('.') && !formatted.contains('e') && !formatted.contains('E') {
        formatted.push_str(".0");
    }
    formatted
}

fn format_extension_value(value: &ExtensionValue) -> String {
    match value {
        ExtensionValue::String(text) => format!("\"{}\"", escape_string(text)),
        ExtensionValue::Integer(value_number) => value_number.to_string(),
        ExtensionValue::Float(value_number) => format_scalar_literal(*value_number),
        ExtensionValue::Bool(value_bool) => value_bool.to_string(),
        ExtensionValue::Datetime(text) => text.clone(),
        ExtensionValue::Array(values) => {
            let body = values
                .iter()
                .map(format_extension_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{body}]")
        }
        ExtensionValue::Table(entries) => {
            let mut keys: Vec<_> = entries.keys().cloned().collect();
            keys.sort();
            let body = keys
                .into_iter()
                .map(|key| format!("{key} = {}", format_extension_value(&entries[&key])))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{ {body} }}")
        }
    }
}

fn write_extension_table(output: &mut String, extensions: &SceneExtensions, indent: usize) {
    let mut keys: Vec<_> = extensions.0.keys().cloned().collect();
    keys.sort();
    for key in keys {
        let value = &extensions.0[&key];
        output.push_str(&" ".repeat(indent));
        output.push_str(&format!("{key} = {}\n", format_extension_value(value)));
    }
}

fn escape_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn default_transform_item() -> Item {
    let mut transform = InlineTable::new();
    transform.insert("translate", default_xyz_inline_value(0.0, 0.0, 0.0));
    transform.insert("rotate_deg", default_xyz_inline_value(0.0, 0.0, 0.0));
    transform.insert("scale", default_xyz_inline_value(1.0, 1.0, 1.0));
    Item::Value(Value::InlineTable(transform))
}

fn default_xyz_inline_item(x: f64, y: f64, z: f64) -> Item {
    Item::Value(default_xyz_inline_value(x, y, z))
}

fn default_xyz_inline_value(x: f64, y: f64, z: f64) -> Value {
    let mut table = InlineTable::new();
    table.insert("x", Value::from(x));
    table.insert("y", Value::from(y));
    table.insert("z", Value::from(z));
    Value::InlineTable(table)
}

fn find_header_location(source: &str, header: &str) -> Option<SourceLocation> {
    let byte_offset = source.find(header)?;
    let (line, column) = line_column_at(source, byte_offset);
    Some(SourceLocation {
        byte_offset,
        line,
        column,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use geom_workspace::{CreateWorkspaceOptions, Workspace};
    use std::fs;
    use tempfile::TempDir;

    const MINIMAL_SOURCE: &str = r#"
schema_version = 1
root = "cube"

[params.width]
type = "scalar"
value = 2.0

[nodes.cube]
kind = "box"
size = { x = { param = "width" }, y = 1.0, z = 3.0 }
transform = { translate = { x = 0.0, y = 1.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 45.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;

    #[test]
    fn parse_serialize_parse_preserves_semantics() {
        let document = parse_scene(MINIMAL_SOURCE).expect("parse scene");
        let canonical = document.to_canonical_source();
        let reparsed = parse_scene(&canonical).expect("reparse canonical source");
        assert_eq!(document, reparsed);
    }

    #[test]
    fn targeted_parameter_edit_preserves_comments_and_unrelated_order() {
        let source = r#"
schema_version = 1
root = "cube"

# dimensions come first on purpose
[params.width]
type = "scalar"
value = 2.0 # keep this comment

[nodes.cube]
kind = "box"
size = { x = { param = "width" }, y = 1.0, z = 3.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;
        let mut scene = SceneSource::parse(source).expect("parse source");
        scene
            .set_parameter_scalar(&ParamId::new("width").expect("param id"), 4.5)
            .expect("edit parameter");
        let updated = scene.text();

        assert!(updated.contains("# dimensions come first on purpose"));
        assert!(updated.contains("value = 4.5"));
        assert!(updated.contains("# keep this comment"));
        assert!(updated.contains("[nodes.cube]"));
        assert!(
            updated.find("[params.width]").expect("param section")
                < updated.find("[nodes.cube]").expect("node section")
        );
    }

    #[test]
    fn node_identity_is_independent_of_definition_order() {
        let source = r#"
schema_version = 1
root = "assembly"

[nodes.assembly]
kind = "union"
children = ["tail", "head"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.tail]
kind = "sphere"
radius = 0.5
transform = { translate = { x = -1.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.head]
kind = "sphere"
radius = 0.75
transform = { translate = { x = 1.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;
        let document = parse_scene(source).expect("parse scene");
        assert_eq!(document.root().as_str(), "assembly");
        let root = &document.nodes()[&NodeId::new("assembly").expect("node id")];
        match root.kind() {
            NodeKind::Union(composition) => {
                assert_eq!(composition.children[0].target().as_str(), "tail");
                assert_eq!(composition.children[1].target().as_str(), "head");
            }
            other => panic!("unexpected node kind: {other:?}"),
        }
    }

    #[test]
    fn duplicate_or_broken_ids_return_typed_errors_with_spans() {
        let duplicate = r#"
schema_version = 1
root = "cube"

[nodes.cube]
kind = "box"
size = { x = 1.0, y = 1.0, z = 1.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.cube]
kind = "sphere"
radius = 1.0
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;
        let duplicate_error = parse_scene(duplicate).expect_err("duplicate node should fail");
        assert!(matches!(
            duplicate_error.kind(),
            SceneErrorKind::Parse { .. }
        ));
        assert!(duplicate_error.span().is_some());

        let broken_reference = r#"
schema_version = 1
root = "shape"

[nodes.shape]
kind = "union"
children = ["missing", "other"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.other]
kind = "sphere"
radius = 1.0
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;
        let broken_error = parse_scene(broken_reference).expect_err("missing child should fail");
        assert!(matches!(
            broken_error.kind(),
            SceneErrorKind::InvalidNodeReference { .. }
        ));
        assert!(broken_error.span().is_some());
    }

    #[test]
    fn invalid_numbers_and_transforms_are_rejected_with_locations() {
        let invalid_scale = r#"
schema_version = 1
root = "cube"

[nodes.cube]
kind = "box"
size = { x = 1.0, y = 1.0, z = 1.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 0.0, y = 1.0, z = 1.0 } }
"#;
        let error = parse_scene(invalid_scale).expect_err("zero scale should fail");
        assert!(matches!(error.kind(), SceneErrorKind::InvalidValue { .. }));
        assert!(error.span().is_some());

        let mut scene = SceneSource::parse(MINIMAL_SOURCE).expect("parse source");
        let error = scene
            .set_transform_component(
                &NodeId::new("cube").expect("node id"),
                TransformProperty::Scale,
                Axis::X,
                f64::INFINITY,
            )
            .expect_err("non-finite scale should fail");
        assert!(matches!(error.kind(), SceneErrorKind::InvalidValue { .. }));
    }

    #[test]
    fn schema_version_and_migration_boundary_behave_correctly() {
        let current = SceneSource::parse(MINIMAL_SOURCE).expect("parse source");
        let migrated = migrate_to_current(current).expect("current version accepted");
        assert_eq!(migrated.schema_version().expect("schema version"), 1);

        let future = r#"
schema_version = 7
root = "cube"

[nodes.cube]
kind = "sphere"
radius = 1.0
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;
        let error = parse_scene(future).expect_err("future version should fail");
        assert!(matches!(
            error.kind(),
            SceneErrorKind::UnsupportedSchemaVersion { .. }
        ));
    }

    #[test]
    fn unknown_fields_are_rejected_but_extensions_survive_round_trip_and_edit() {
        let invalid = r#"
schema_version = 1
root = "cube"
bogus = true

[nodes.cube]
kind = "sphere"
radius = 1.0
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;
        let unknown_error = parse_scene(invalid).expect_err("unknown field should fail");
        assert!(matches!(
            unknown_error.kind(),
            SceneErrorKind::UnknownField { .. }
        ));

        let extensible = r#"
schema_version = 1
root = "cube"

[extensions]
author = "test"
ui = { color = "blue" }

[params.width]
type = "scalar"
value = 2.0
extensions = { units = "meters" }

[nodes.cube]
kind = "box"
size = { x = { param = "width" }, y = 1.0, z = 3.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
extensions = { material = "matte" }
"#;
        let mut scene = SceneSource::parse(extensible).expect("parse extensible source");
        let document = scene.validate().expect("validate extensible source");
        assert_eq!(
            document.extensions().entries()["author"],
            ExtensionValue::String("test".to_owned())
        );
        scene
            .set_parameter_scalar(&ParamId::new("width").expect("param id"), 3.5)
            .expect("edit with extensions");
        let reparsed = parse_scene(scene.text()).expect("reparse edited source");
        assert_eq!(
            reparsed.extensions().entries()["author"],
            ExtensionValue::String("test".to_owned())
        );
    }

    #[test]
    fn checked_in_examples_all_parse() {
        let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("scenes");
        for entry in fs::read_dir(&examples_dir).expect("read examples directory") {
            let entry = entry.expect("directory entry");
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("toml") {
                let source = fs::read_to_string(&path).expect("read example scene");
                parse_scene(&source)
                    .unwrap_or_else(|error| panic!("example {} failed: {error}", path.display()));
            }
        }
    }

    #[test]
    fn workspace_integration_preserves_scene_edit_through_save_and_reopen() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("workspace-scene");
        let mut workspace = Workspace::create(
            &workspace_root,
            CreateWorkspaceOptions::new("Scene Workspace"),
        )
        .expect("create workspace");

        workspace.replace_source(MINIMAL_SOURCE);
        workspace.save().expect("save workspace with scene");

        let reopened = Workspace::open(&workspace_root).expect("reopen workspace");
        let mut scene = SceneSource::parse(reopened.source_text()).expect("parse scene source");
        let parsed = scene.validate().expect("validate scene");
        assert_eq!(parsed.root().as_str(), "cube");

        scene
            .set_parameter_scalar(&ParamId::new("width").expect("param id"), 9.0)
            .expect("edit scene parameter");

        let mut reopened = reopened;
        reopened.replace_source(scene.into_text());
        reopened.save().expect("persist updated scene");

        let reopened_again = Workspace::open(&workspace_root).expect("reopen edited workspace");
        let reparsed = parse_scene(reopened_again.source_text()).expect("reparse edited scene");
        assert_eq!(
            reparsed.parameters()[&ParamId::new("width").expect("param id")].scalar_value(),
            9.0
        );
    }

    #[test]
    fn rename_updates_root_and_composition_references_without_rewriting_unrelated_comments() {
        let source = r#"
schema_version = 1
root = "root"

# keep this top comment
[nodes.shared]
kind = "sphere"
radius = 1.0
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.root]
kind = "union"
children = ["shared", "shared"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;
        let mut scene = SceneSource::parse(source).expect("parse");
        let document = scene
            .rename_node(
                &NodeId::new("shared").expect("from"),
                &NodeId::new("core").expect("to"),
            )
            .expect("rename");
        let updated = scene.text();

        assert!(updated.contains("# keep this top comment"));
        assert!(updated.contains("[nodes.core]"));
        assert!(updated.contains("children = [\"core\", \"core\"]"));
        assert!(
            document
                .nodes()
                .contains_key(&NodeId::new("core").expect("id"))
        );
    }

    #[test]
    fn duplicate_and_delete_structural_edits_preserve_other_sections() {
        let source = r#"
schema_version = 1
root = "root"

[params.width]
type = "scalar"
value = 2.0

[nodes.part]
kind = "box"
size = { x = { param = "width" }, y = 1.0, z = 1.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.root]
kind = "union"
children = ["part", "part"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;
        let mut scene = SceneSource::parse(source).expect("parse");
        scene
            .duplicate_node(
                &NodeId::new("part").expect("source"),
                &NodeId::new("part_copy").expect("duplicate"),
            )
            .expect("duplicate");
        let duplicated = parse_scene(scene.text()).expect("reparse duplicated");
        assert!(
            duplicated
                .nodes()
                .contains_key(&NodeId::new("part_copy").expect("id"))
        );
        match duplicated.nodes()[&NodeId::new("part_copy").expect("id")].kind() {
            NodeKind::Box(box_node) => {
                assert!(matches!(box_node.size.x, ScalarExpr::Parameter(_)));
            }
            other => panic!("unexpected duplicated kind: {other:?}"),
        }

        let mut scene = SceneSource::parse(
            r#"
schema_version = 1
root = "root"

[nodes.deletable]
kind = "sphere"
radius = 0.5
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.root]
kind = "union"
children = ["other", "other"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.other]
kind = "sphere"
radius = 0.75
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#,
        )
        .expect("parse deletable");
        scene
            .delete_node(&NodeId::new("deletable").expect("id"))
            .expect("delete");
        assert!(!scene.text().contains("[nodes.deletable]"));
        assert!(scene.text().contains("[nodes.other]"));
    }
}
