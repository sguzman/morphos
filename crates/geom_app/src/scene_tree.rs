use geom_scene::{NodeId, NodeKind, ParamId, ScalarExpr, SceneDocument};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneTreeModel {
    entries: BTreeMap<NodeId, SceneTreeEntry>,
    roots: Vec<NodeId>,
    unreferenced: Vec<NodeId>,
}

impl SceneTreeModel {
    pub fn from_scene(scene: &SceneDocument) -> Self {
        let mut entries = BTreeMap::new();
        let mut incoming_counts: BTreeMap<NodeId, usize> = BTreeMap::new();
        let mut parameter_dependents: BTreeMap<ParamId, BTreeSet<NodeId>> = BTreeMap::new();

        for node in scene.nodes().values() {
            let dependency_ids = dependency_ids(node.kind());
            for dependency in &dependency_ids {
                *incoming_counts.entry(dependency.clone()).or_insert(0) += 1;
            }
            let scalar_parameter_dependencies =
                scalar_parameter_dependencies(node.kind(), node.transform());
            for parameter in &scalar_parameter_dependencies {
                parameter_dependents
                    .entry(parameter.clone())
                    .or_default()
                    .insert(node.id().clone());
            }
            entries.insert(
                node.id().clone(),
                SceneTreeEntry {
                    node_id: node.id().clone(),
                    label: node.label().map(str::to_owned),
                    kind_label: kind_label(node.kind()).to_owned(),
                    dependency_ids,
                    incoming_reference_count: 0,
                    is_root: node.id() == scene.root(),
                    scalar_parameter_dependencies,
                },
            );
        }

        for (node_id, count) in incoming_counts {
            if let Some(entry) = entries.get_mut(&node_id) {
                entry.incoming_reference_count = count;
            }
        }

        let mut visited = BTreeSet::new();
        let mut roots = Vec::new();
        if entries.contains_key(scene.root()) {
            roots.push(scene.root().clone());
            visited.insert(scene.root().clone());
        }

        let mut unreferenced = entries
            .values()
            .filter(|entry| {
                !visited.contains(&entry.node_id) && entry.incoming_reference_count == 0
            })
            .map(|entry| entry.node_id.clone())
            .collect::<Vec<_>>();
        unreferenced.sort();

        Self {
            entries,
            roots,
            unreferenced,
        }
    }

    pub fn entries(&self) -> &BTreeMap<NodeId, SceneTreeEntry> {
        &self.entries
    }

    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    pub fn unreferenced(&self) -> &[NodeId] {
        &self.unreferenced
    }

    pub fn entry(&self, node_id: &NodeId) -> Option<&SceneTreeEntry> {
        self.entries.get(node_id)
    }

    pub fn preserve_selection(
        &self,
        selected: Option<&NodeId>,
        renamed_from: Option<&NodeId>,
        renamed_to: Option<&NodeId>,
    ) -> Option<NodeId> {
        let selected = selected?;
        if self.entries.contains_key(selected) {
            return Some(selected.clone());
        }
        match (renamed_from, renamed_to) {
            (Some(from), Some(to)) if selected == from && self.entries.contains_key(to) => {
                Some(to.clone())
            }
            _ => None,
        }
    }

    pub fn filtered_matches(&self, query: &str) -> BTreeSet<NodeId> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return self.entries.keys().cloned().collect();
        }

        let needle = trimmed.to_ascii_lowercase();
        self.entries
            .values()
            .filter(|entry| {
                entry
                    .node_id
                    .as_str()
                    .to_ascii_lowercase()
                    .contains(&needle)
                    || entry
                        .label
                        .as_deref()
                        .map(|label| label.to_ascii_lowercase().contains(&needle))
                        .unwrap_or(false)
            })
            .map(|entry| entry.node_id.clone())
            .collect()
    }

    pub fn parameter_dependents(&self, parameter: &ParamId) -> Vec<NodeId> {
        let mut dependents = self
            .entries
            .values()
            .filter(|entry| entry.scalar_parameter_dependencies.contains(parameter))
            .map(|entry| entry.node_id.clone())
            .collect::<Vec<_>>();
        dependents.sort();
        dependents
    }

    pub fn direct_dependents(&self, node: &NodeId) -> Vec<NodeId> {
        let mut dependents = self
            .entries
            .values()
            .filter(|entry| {
                entry
                    .dependency_ids
                    .iter()
                    .any(|dependency| dependency == node)
            })
            .map(|entry| entry.node_id.clone())
            .collect::<Vec<_>>();
        dependents.sort();
        dependents
    }

    pub fn transitive_dependents(&self, node: &NodeId) -> Vec<NodeId> {
        let mut out = BTreeSet::new();
        let mut stack = self.direct_dependents(node);
        while let Some(next) = stack.pop() {
            if out.insert(next.clone()) {
                stack.extend(self.direct_dependents(&next));
            }
        }
        out.into_iter().collect()
    }

    pub fn transitive_parameter_dependents(&self, parameter: &ParamId) -> Vec<NodeId> {
        let direct = self.parameter_dependents(parameter);
        let mut out: BTreeSet<NodeId> = direct.iter().cloned().collect();
        let mut stack = direct;
        while let Some(next) = stack.pop() {
            for dependent in self.direct_dependents(&next) {
                if out.insert(dependent.clone()) {
                    stack.push(dependent);
                }
            }
        }
        out.into_iter().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneTreeEntry {
    pub node_id: NodeId,
    pub label: Option<String>,
    pub kind_label: String,
    pub dependency_ids: Vec<NodeId>,
    pub incoming_reference_count: usize,
    pub is_root: bool,
    pub scalar_parameter_dependencies: BTreeSet<ParamId>,
}

impl SceneTreeEntry {
    pub fn is_shared(&self) -> bool {
        self.incoming_reference_count > 1
    }
}

fn dependency_ids(kind: &NodeKind) -> Vec<NodeId> {
    match kind {
        NodeKind::Union(node) | NodeKind::Difference(node) | NodeKind::Intersection(node) => node
            .children
            .iter()
            .map(|child| child.target().clone())
            .collect(),
        _ => Vec::new(),
    }
}

fn scalar_parameter_dependencies(
    kind: &NodeKind,
    transform: &geom_scene::Transform,
) -> BTreeSet<ParamId> {
    let mut out = BTreeSet::new();
    match kind {
        NodeKind::Box(node) => {
            collect_scalar_expr_parameter(&node.size.x, &mut out);
            collect_scalar_expr_parameter(&node.size.y, &mut out);
            collect_scalar_expr_parameter(&node.size.z, &mut out);
        }
        NodeKind::Sphere(node) => collect_scalar_expr_parameter(&node.radius, &mut out),
        NodeKind::Cylinder(node) => {
            collect_scalar_expr_parameter(&node.radius, &mut out);
            collect_scalar_expr_parameter(&node.height, &mut out);
        }
        NodeKind::Capsule(node) => {
            collect_scalar_expr_parameter(&node.radius, &mut out);
            collect_scalar_expr_parameter(&node.height, &mut out);
        }
        NodeKind::Plane(node) => {
            collect_scalar_expr_parameter(&node.width, &mut out);
            collect_scalar_expr_parameter(&node.depth, &mut out);
        }
        NodeKind::Profile(node) => {
            collect_scalar_expr_parameter(&node.width, &mut out);
            collect_scalar_expr_parameter(&node.height, &mut out);
        }
        NodeKind::Union(_) | NodeKind::Difference(_) | NodeKind::Intersection(_) => {}
    }
    collect_scalar_expr_parameter(&transform.translation.x, &mut out);
    collect_scalar_expr_parameter(&transform.translation.y, &mut out);
    collect_scalar_expr_parameter(&transform.translation.z, &mut out);
    collect_scalar_expr_parameter(&transform.rotation_deg.x, &mut out);
    collect_scalar_expr_parameter(&transform.rotation_deg.y, &mut out);
    collect_scalar_expr_parameter(&transform.rotation_deg.z, &mut out);
    collect_scalar_expr_parameter(&transform.scale.x, &mut out);
    collect_scalar_expr_parameter(&transform.scale.y, &mut out);
    collect_scalar_expr_parameter(&transform.scale.z, &mut out);
    out
}

fn collect_scalar_expr_parameter(expr: &ScalarExpr, out: &mut BTreeSet<ParamId>) {
    if let ScalarExpr::Parameter(parameter) = expr {
        out.insert(parameter.target().clone());
    }
}

fn kind_label(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Box(_) => "box",
        NodeKind::Sphere(_) => "sphere",
        NodeKind::Cylinder(_) => "cylinder",
        NodeKind::Capsule(_) => "capsule",
        NodeKind::Plane(_) => "plane",
        NodeKind::Profile(_) => "profile",
        NodeKind::Union(_) => "union",
        NodeKind::Difference(_) => "difference",
        NodeKind::Intersection(_) => "intersection",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geom_scene::parse_scene;

    const SHARED_SCENE: &str = r#"
schema_version = 1
root = "root"

[params.arm_length]
type = "scalar"
value = 2.0

[nodes.shared]
kind = "capsule"
radius = 0.2
height = { param = "arm_length" }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.left]
kind = "union"
children = ["shared", "left_tip"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.left_tip]
kind = "sphere"
radius = 0.4
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.right]
kind = "union"
children = ["shared", "right_tip"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.right_tip]
kind = "sphere"
radius = 0.4
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.root]
kind = "union"
children = ["left", "right"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.orphan]
kind = "box"
size = { x = 1.0, y = 1.0, z = 1.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;

    #[test]
    fn tree_model_marks_root_shared_and_unreferenced_nodes() {
        let scene = parse_scene(SHARED_SCENE).expect("parse");
        let tree = SceneTreeModel::from_scene(&scene);

        assert_eq!(tree.roots(), &[NodeId::new("root").expect("id")]);
        assert_eq!(tree.unreferenced(), &[NodeId::new("orphan").expect("id")]);
        assert!(
            tree.entry(&NodeId::new("shared").expect("id"))
                .expect("entry")
                .is_shared()
        );
    }

    #[test]
    fn filtering_matches_id_and_label_case_insensitively() {
        let scene = parse_scene(
            r#"
schema_version = 1
root = "robot"

[nodes.robot]
kind = "union"
children = ["torso", "cap"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.torso]
kind = "box"
label = "Main Torso"
size = { x = 1.0, y = 1.0, z = 1.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.cap]
kind = "sphere"
radius = 0.5
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#,
        )
        .expect("parse");
        let tree = SceneTreeModel::from_scene(&scene);

        let torso = NodeId::new("torso").expect("id");
        assert!(tree.filtered_matches("TORS").contains(&torso));
        assert!(tree.filtered_matches("main").contains(&torso));
    }

    #[test]
    fn selection_persists_and_tracks_explicit_rename_mapping() {
        let scene = parse_scene(SHARED_SCENE).expect("parse");
        let tree = SceneTreeModel::from_scene(&scene);
        let root = NodeId::new("root").expect("id");
        assert_eq!(
            tree.preserve_selection(Some(&root), None, None),
            Some(root.clone())
        );

        let renamed_scene = parse_scene(
            SHARED_SCENE
                .replace("\"shared\"", "\"arm_shared\"")
                .as_str(),
        )
        .expect_err("broken scene should fail before explicit test");
        let _ = renamed_scene;

        let updated = parse_scene(
            r#"
schema_version = 1
root = "root"

[nodes.arm_shared]
kind = "sphere"
radius = 0.5
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.root]
kind = "union"
children = ["arm_shared", "arm_shared"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#,
        )
        .expect("parse updated");
        let updated_tree = SceneTreeModel::from_scene(&updated);
        assert_eq!(
            updated_tree.preserve_selection(
                Some(&NodeId::new("shared").expect("id")),
                Some(&NodeId::new("shared").expect("id")),
                Some(&NodeId::new("arm_shared").expect("id"))
            ),
            Some(NodeId::new("arm_shared").expect("id"))
        );
        assert_eq!(
            updated_tree.preserve_selection(Some(&NodeId::new("shared").expect("id")), None, None),
            None
        );
    }

    #[test]
    fn parameter_dependents_include_transform_and_primitive_references() {
        let scene = parse_scene(SHARED_SCENE).expect("parse");
        let tree = SceneTreeModel::from_scene(&scene);
        let dependents = tree.parameter_dependents(&ParamId::new("arm_length").expect("id"));
        assert_eq!(dependents, vec![NodeId::new("shared").expect("id")]);
    }
}
