//! Structured, headless workspace API for local tools and AI agents.
//!
//! The API is intentionally provider-independent and operates only through
//! project-owned workspace, scene, geometry, and diagnostics crates.

pub mod protocol;

use geom_diagnostics::Diagnostic;
use geom_geometry::{
    BoolmeshBackend, Bounds, EvaluatedGeometry, GeometryEvaluator, validate_backend_support,
    validate_evaluated_geometry,
};
use geom_scene::{
    NodeId, NodeKind, ParamId, ScalarExpr, SceneDocument, SceneSource, SourceLocation,
    diagnostic_from_scene_error, parse_scene_report,
};
use geom_workspace::{
    AffectedTargets, HistoryQuery, TimeRange, TransactionActor, Workspace, WorkspaceDirectory,
    WorkspaceHistoryEntry, WorkspaceOp, WorkspaceSceneChange, WorkspaceTransaction,
    WorkspaceTransactionError,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

pub const API_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiVersion {
    pub protocol_version: u32,
}

impl Default for ApiVersion {
    fn default() -> Self {
        Self {
            protocol_version: API_PROTOCOL_VERSION,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionBounds {
    pub limit: usize,
}

impl CollectionBounds {
    pub const fn new(limit: usize) -> Self {
        Self { limit }
    }
}

impl Default for CollectionBounds {
    fn default() -> Self {
        Self { limit: 64 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedList<T> {
    pub items: Vec<T>,
    pub total_count: usize,
    pub truncated: bool,
}

impl<T> BoundedList<T> {
    fn from_items(mut items: Vec<T>, bounds: CollectionBounds) -> Self {
        let total_count = items.len();
        let truncated = total_count > bounds.limit;
        if truncated {
            items.truncate(bounds.limit);
        }
        Self {
            items,
            total_count,
            truncated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReadContext<T> {
    pub revision: u64,
    pub value: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiCapabilities {
    pub version: ApiVersion,
    pub workspace_op_kinds: Vec<String>,
    pub node_kinds: Vec<String>,
    pub geometry_backend_capabilities: Vec<String>,
    pub export_formats: Vec<String>,
    pub preview_available: bool,
    pub diagnostics_available: bool,
    pub history_available: bool,
    pub snapshots_available: bool,
    pub dry_run_available: bool,
    pub source_snippet_available: bool,
    pub optional_features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDiagnosticSummary {
    pub total: usize,
    pub blocking: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentHistorySummary {
    pub count: usize,
    pub latest_transaction_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSummaryView {
    pub workspace_id: String,
    pub workspace_name: String,
    pub workspace_root: String,
    pub revision: u64,
    pub workspace_format_version: u32,
    pub scene_schema_version: Option<u32>,
    pub root_output: Option<String>,
    pub node_count: usize,
    pub parameter_count: usize,
    pub diagnostics: WorkspaceDiagnosticSummary,
    pub recent_history: RecentHistorySummary,
    pub snapshot_count: usize,
    pub capabilities: ApiCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneTreeNodeView {
    pub node_id: String,
    pub label: Option<String>,
    pub kind: String,
    pub dependencies: Vec<String>,
    pub is_root: bool,
    pub is_shared: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneTreeView {
    pub root_nodes: Vec<String>,
    pub unreferenced_nodes: Vec<String>,
    pub nodes: BoundedList<SceneTreeNodeView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeQuery {
    pub node_id: String,
    #[serde(default)]
    pub include_source_snippet: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDetailView {
    pub node_id: String,
    pub kind: String,
    pub label: Option<String>,
    pub is_root: bool,
    pub dependencies: Vec<String>,
    pub direct_dependents: Vec<String>,
    pub transitive_dependents: Vec<String>,
    pub parameter_dependencies: Vec<String>,
    pub transform: TransformView,
    pub properties: NodePropertiesView,
    pub source_location: Option<SourceLocationView>,
    pub source_snippet: Option<SourceSnippetView>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocationView {
    pub line: usize,
    pub column: usize,
    pub byte_offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSnippetView {
    pub start_line: usize,
    pub end_line: usize,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformView {
    pub translation: VectorExprView,
    pub rotation_deg: VectorExprView,
    pub scale: VectorExprView,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorExprView {
    pub x: ScalarExprView,
    pub y: ScalarExprView,
    pub z: ScalarExprView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScalarExprView {
    Literal { value: String },
    Parameter { parameter_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodePropertiesView {
    Box {
        size: VectorExprView,
    },
    Sphere {
        radius: ScalarExprView,
    },
    Cylinder {
        radius: ScalarExprView,
        height: ScalarExprView,
    },
    Capsule {
        radius: ScalarExprView,
        height: ScalarExprView,
    },
    Plane {
        width: ScalarExprView,
        depth: ScalarExprView,
    },
    Profile {
        width: ScalarExprView,
        height: ScalarExprView,
    },
    Union {
        children: Vec<String>,
    },
    Difference {
        children: Vec<String>,
    },
    Intersection {
        children: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterView {
    pub parameter_id: String,
    pub scalar_value: f64,
    pub direct_dependents: Vec<String>,
    pub transitive_dependents: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub source_location: Option<SourceLocationView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DiagnosticFilter {
    pub node_id: Option<String>,
    pub parameter_id: Option<String>,
    pub severity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SourceSnippetRequest {
    pub node_id: Option<String>,
    pub parameter_id: Option<String>,
    pub line_radius: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecentHistoryQuery {
    pub actor: Option<String>,
    pub node_id: Option<String>,
    pub parameter_id: Option<String>,
    pub start_millis: Option<u64>,
    pub end_millis: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntryView {
    pub transaction_id: String,
    pub actor: String,
    pub intent: Option<String>,
    pub revision_before: u64,
    pub revision_after: u64,
    pub timestamp_millis: u64,
    pub affected_node_ids: Vec<String>,
    pub affected_parameter_ids: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeometryStatsView {
    pub requested_output: String,
    pub bounds: BoundsView,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub evaluated_node_count: usize,
    pub participating_node_ids: Vec<String>,
    pub resolved_parameters: BoundedList<ResolvedParameterView>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BoundsView {
    Empty,
    Finite { min: [f64; 3], max: [f64; 3] },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedParameterView {
    pub parameter_id: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluateOutputRequest {
    pub node_id: Option<String>,
    pub parameter_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewRequest {
    pub node_id: Option<String>,
    pub width: u32,
    pub height: u32,
    pub destination: Option<String>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewArtifactView {
    pub requested_output: String,
    pub revision: u64,
    pub relative_path: String,
    pub absolute_path: String,
    pub width: u32,
    pub height: u32,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionProposal {
    pub expected_revision: u64,
    pub actor: String,
    pub intent: Option<String>,
    pub operations: Vec<WorkspaceOpRequest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceOpRequest {
    SetParameterScalar {
        parameter_id: String,
        value: f64,
    },
    SetNodeLabel {
        node_id: String,
        label: Option<String>,
    },
    RenameNode {
        from: String,
        to: String,
    },
    SetRootNode {
        node_id: String,
    },
    DeleteNode {
        node_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationResultView {
    pub accepted: bool,
    pub base_revision: u64,
    pub resulting_revision: Option<u64>,
    pub transaction_id: Option<String>,
    pub affected_node_ids: Vec<String>,
    pub affected_parameter_ids: Vec<String>,
    pub diff: Option<SceneDiffView>,
    pub diagnostics: Vec<Diagnostic>,
    pub expected_rebuild_scope: RebuildScopeView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneDiffView {
    pub before_revision: u64,
    pub after_revision: u64,
    pub summary: String,
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebuildScopeView {
    pub affected_node_ids: Vec<String>,
    pub affected_parameter_ids: Vec<String>,
}

#[derive(Debug, Error)]
pub enum WorkspaceApiError {
    #[error("workspace API request is invalid: {message}")]
    InvalidRequest { message: String },

    #[error("workspace API revision mismatch: expected {expected}, current {current}")]
    StaleRevision { expected: u64, current: u64 },

    #[error("workspace API validation failed: {message}")]
    Validation {
        message: String,
        diagnostics: Vec<Diagnostic>,
    },

    #[error("workspace API workspace failure: {message}")]
    Workspace { message: String },
}

pub struct WorkspaceApi;

impl WorkspaceApi {
    pub fn capabilities() -> ApiCapabilities {
        ApiCapabilities {
            version: ApiVersion::default(),
            workspace_op_kinds: vec![
                "set_parameter_scalar".to_owned(),
                "set_node_label".to_owned(),
                "rename_node".to_owned(),
                "set_root_node".to_owned(),
                "delete_node".to_owned(),
            ],
            node_kinds: vec![
                "box",
                "sphere",
                "cylinder",
                "capsule",
                "plane",
                "profile",
                "union",
                "difference",
                "intersection",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            geometry_backend_capabilities: vec![
                "box".to_owned(),
                "sphere".to_owned(),
                "cylinder".to_owned(),
                "capsule".to_owned(),
            ],
            export_formats: vec!["obj".to_owned(), "stl".to_owned()],
            preview_available: true,
            diagnostics_available: true,
            history_available: true,
            snapshots_available: true,
            dry_run_available: true,
            source_snippet_available: true,
            optional_features: vec!["stdio_transport".to_owned()],
        }
    }

    pub fn get_workspace_summary(
        workspace: &Workspace,
    ) -> Result<WorkspaceReadContext<WorkspaceSummaryView>, WorkspaceApiError> {
        let scene = parse_scene_report(workspace.source_text()).ok();
        let diagnostics = workspace_diagnostics(workspace);
        let history =
            workspace
                .history_entries()
                .map_err(|error| WorkspaceApiError::Workspace {
                    message: error.to_string(),
                })?;
        let snapshots = workspace
            .snapshots()
            .map_err(|error| WorkspaceApiError::Workspace {
                message: error.to_string(),
            })?;
        let summary = workspace.summary();
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: WorkspaceSummaryView {
                workspace_id: summary.workspace_id().to_string(),
                workspace_name: summary.name().to_owned(),
                workspace_root: summary.root().display().to_string(),
                revision: summary.revision().get(),
                workspace_format_version: summary.format_version(),
                scene_schema_version: scene.as_ref().map(SceneDocument::schema_version),
                root_output: scene.as_ref().map(|scene| scene.root().to_string()),
                node_count: scene.as_ref().map(|scene| scene.nodes().len()).unwrap_or(0),
                parameter_count: scene
                    .as_ref()
                    .map(|scene| scene.parameters().len())
                    .unwrap_or(0),
                diagnostics: WorkspaceDiagnosticSummary {
                    total: diagnostics.len(),
                    blocking: diagnostics
                        .iter()
                        .filter(|diagnostic| diagnostic.blocking)
                        .count(),
                },
                recent_history: RecentHistorySummary {
                    count: history.len().min(5),
                    latest_transaction_id: history
                        .last()
                        .map(|entry| entry.transaction_id().to_string()),
                },
                snapshot_count: snapshots.len(),
                capabilities: Self::capabilities(),
            },
        })
    }

    pub fn get_scene_tree(
        workspace: &Workspace,
        bounds: CollectionBounds,
    ) -> Result<WorkspaceReadContext<SceneTreeView>, WorkspaceApiError> {
        let scene = parse_workspace_scene(workspace)?;
        let projection = SceneProjection::from_scene(&scene);
        let nodes = projection
            .ordered_node_ids()
            .into_iter()
            .filter_map(|node_id| projection.entries.get(&node_id))
            .cloned()
            .map(SceneTreeNodeView::from)
            .collect::<Vec<_>>();
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: SceneTreeView {
                root_nodes: projection.roots.iter().map(ToString::to_string).collect(),
                unreferenced_nodes: projection
                    .unreferenced
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                nodes: BoundedList::from_items(nodes, bounds),
            },
        })
    }

    pub fn get_node(
        workspace: &Workspace,
        query: &NodeQuery,
    ) -> Result<WorkspaceReadContext<NodeDetailView>, WorkspaceApiError> {
        let scene = parse_workspace_scene(workspace)?;
        let projection = SceneProjection::from_scene(&scene);
        let node_id = parse_node_id(&query.node_id)?;
        let node =
            scene
                .nodes()
                .get(&node_id)
                .ok_or_else(|| WorkspaceApiError::InvalidRequest {
                    message: format!("unknown node `{}`", query.node_id),
                })?;
        let diagnostics = filtered_diagnostics(workspace, Some(&node_id), None);
        let source = SceneSource::parse(workspace.source_text()).ok();
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: NodeDetailView {
                node_id: node.id().to_string(),
                kind: node_kind_name(node.kind()).to_owned(),
                label: node.label().map(str::to_owned),
                is_root: scene.root() == node.id(),
                dependencies: projection.dependencies(node.id()),
                direct_dependents: projection.direct_dependents(node.id()),
                transitive_dependents: projection.transitive_dependents(node.id()),
                parameter_dependencies: projection.parameter_dependencies(node.id()),
                transform: TransformView::from(node.transform()),
                properties: NodePropertiesView::from(node.kind()),
                source_location: source
                    .as_ref()
                    .and_then(|source| source.node_source_location(node.id()))
                    .map(SourceLocationView::from),
                source_snippet: if query.include_source_snippet {
                    source_snippet_for_node(workspace.source_text(), source.as_ref(), node.id(), 3)
                } else {
                    None
                },
                diagnostics,
            },
        })
    }

    pub fn get_parameters(
        workspace: &Workspace,
        bounds: CollectionBounds,
    ) -> Result<WorkspaceReadContext<BoundedList<ParameterView>>, WorkspaceApiError> {
        let scene = parse_workspace_scene(workspace)?;
        let projection = SceneProjection::from_scene(&scene);
        let source = SceneSource::parse(workspace.source_text()).ok();
        let parameters = scene
            .parameters()
            .values()
            .map(|parameter| ParameterView {
                parameter_id: parameter.id().to_string(),
                scalar_value: parameter.scalar_value(),
                direct_dependents: projection.parameter_dependents(parameter.id()),
                transitive_dependents: projection.transitive_parameter_dependents(parameter.id()),
                diagnostics: filtered_diagnostics(workspace, None, Some(parameter.id())),
                source_location: source
                    .as_ref()
                    .and_then(|source| source.parameter_source_location(parameter.id()))
                    .map(SourceLocationView::from),
            })
            .collect::<Vec<_>>();
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: BoundedList::from_items(parameters, bounds),
        })
    }

    pub fn get_diagnostics(
        workspace: &Workspace,
        filter: DiagnosticFilter,
        bounds: CollectionBounds,
    ) -> Result<WorkspaceReadContext<BoundedList<Diagnostic>>, WorkspaceApiError> {
        let node_id = match filter.node_id.as_deref() {
            Some(value) => Some(parse_node_id(value)?),
            None => None,
        };
        let parameter_id = match filter.parameter_id.as_deref() {
            Some(value) => Some(parse_param_id(value)?),
            None => None,
        };
        let mut diagnostics =
            filtered_diagnostics(workspace, node_id.as_ref(), parameter_id.as_ref());
        if let Some(severity) = filter.severity.as_deref() {
            diagnostics.retain(|diagnostic| {
                serde_json::to_value(diagnostic.severity)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .map(|value| value == severity)
                    .unwrap_or(false)
            });
        }
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: BoundedList::from_items(diagnostics, bounds),
        })
    }

    pub fn get_recent_history(
        workspace: &Workspace,
        query: RecentHistoryQuery,
    ) -> Result<WorkspaceReadContext<BoundedList<HistoryEntryView>>, WorkspaceApiError> {
        let mut history_query = HistoryQuery::default();
        if let Some(actor) = query.actor {
            history_query = history_query.with_actor(parse_actor(&actor)?);
        }
        if let Some(node_id) = query.node_id {
            history_query = history_query.with_node(parse_node_id(&node_id)?);
        }
        if let Some(parameter_id) = query.parameter_id {
            history_query = history_query.with_parameter(parse_param_id(&parameter_id)?);
        }
        if let (Some(start), Some(end)) = (query.start_millis, query.end_millis) {
            history_query = history_query.with_time_range(TimeRange::new(start, end));
        }
        let entries = workspace
            .query_history(&history_query)
            .map_err(|error| WorkspaceApiError::Workspace {
                message: error.to_string(),
            })?
            .into_iter()
            .map(HistoryEntryView::from)
            .collect::<Vec<_>>();
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: BoundedList::from_items(
                entries,
                CollectionBounds::new(query.limit.unwrap_or(20)),
            ),
        })
    }

    pub fn evaluate_output(
        workspace: &Workspace,
        request: EvaluateOutputRequest,
    ) -> Result<WorkspaceReadContext<GeometryStatsView>, WorkspaceApiError> {
        let scene = parse_workspace_scene(workspace)?;
        let node_id = match request.node_id.as_deref() {
            Some(node_id) => Some(parse_node_id(node_id)?),
            None => None,
        };
        let diagnostics = backend_and_geometry_diagnostics(&scene, node_id.as_ref())?;
        let evaluation = evaluate_scene(&scene, node_id.as_ref())?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: GeometryStatsView::from_evaluated(
                &evaluation,
                diagnostics,
                request.parameter_limit,
            ),
        })
    }

    pub fn request_preview(
        workspace: &Workspace,
        request: PreviewRequest,
    ) -> Result<WorkspaceReadContext<PreviewArtifactView>, WorkspaceApiError> {
        let scene = parse_workspace_scene(workspace)?;
        let node_id = match request.node_id.as_deref() {
            Some(node_id) => Some(parse_node_id(node_id)?),
            None => None,
        };
        let diagnostics = backend_and_geometry_diagnostics(&scene, node_id.as_ref())?;
        let evaluation = evaluate_scene(&scene, node_id.as_ref())?;
        let artifact = render_preview_artifact(
            workspace,
            &evaluation,
            request.destination.as_deref(),
            request.width,
            request.height,
            request.overwrite,
        )?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: PreviewArtifactView {
                requested_output: evaluation.requested_output.to_string(),
                revision: workspace.revision().get(),
                relative_path: artifact
                    .strip_prefix(workspace.root())
                    .unwrap_or(&artifact)
                    .display()
                    .to_string(),
                absolute_path: artifact.display().to_string(),
                width: request.width,
                height: request.height,
                diagnostics,
            },
        })
    }

    pub fn source_snippet(
        workspace: &Workspace,
        request: SourceSnippetRequest,
    ) -> Result<WorkspaceReadContext<Option<SourceSnippetView>>, WorkspaceApiError> {
        let source = SceneSource::parse(workspace.source_text()).map_err(|error| {
            WorkspaceApiError::Workspace {
                message: error.to_string(),
            }
        })?;
        let snippet = if let Some(node_id) = request.node_id.as_deref() {
            let node_id = parse_node_id(node_id)?;
            source_snippet_for_node(
                workspace.source_text(),
                Some(&source),
                &node_id,
                request.line_radius,
            )
        } else if let Some(parameter_id) = request.parameter_id.as_deref() {
            let parameter_id = parse_param_id(parameter_id)?;
            source_snippet_for_parameter(
                workspace.source_text(),
                Some(&source),
                &parameter_id,
                request.line_radius,
            )
        } else {
            None
        };
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: snippet,
        })
    }

    pub fn dry_run_transaction(
        workspace: &Workspace,
        proposal: TransactionProposal,
    ) -> Result<WorkspaceReadContext<MutationResultView>, WorkspaceApiError> {
        let transaction = transaction_from_proposal(&proposal, workspace.revision().get())?;
        let diff = dry_run_diff(workspace, &transaction)?;
        let affected_targets = transaction.affected_targets();
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: MutationResultView {
                accepted: true,
                base_revision: proposal.expected_revision,
                resulting_revision: None,
                transaction_id: Some(transaction.id().to_string()),
                affected_node_ids: affected_targets
                    .node_ids()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                affected_parameter_ids: affected_targets
                    .parameter_ids()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                diff: Some(diff),
                diagnostics: Vec::new(),
                expected_rebuild_scope: rebuild_scope_from_targets(&affected_targets),
            },
        })
    }

    pub fn apply_transaction(
        workspace: &mut Workspace,
        proposal: TransactionProposal,
    ) -> Result<WorkspaceReadContext<MutationResultView>, WorkspaceApiError> {
        let transaction = transaction_from_proposal(&proposal, workspace.revision().get())?;
        let diff = dry_run_diff(workspace, &transaction)?;
        let commit = workspace
            .apply_transaction(&transaction)
            .map_err(workspace_transaction_error)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: MutationResultView {
                accepted: true,
                base_revision: proposal.expected_revision,
                resulting_revision: Some(commit.revision_after().get()),
                transaction_id: Some(commit.transaction_id().to_string()),
                affected_node_ids: commit
                    .affected_targets()
                    .node_ids()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                affected_parameter_ids: commit
                    .affected_targets()
                    .parameter_ids()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                diff: Some(diff),
                diagnostics: Vec::new(),
                expected_rebuild_scope: rebuild_scope_from_targets(commit.affected_targets()),
            },
        })
    }
}

#[derive(Debug, Clone)]
struct SceneProjection {
    entries: BTreeMap<NodeId, SceneProjectionEntry>,
    roots: Vec<NodeId>,
    unreferenced: Vec<NodeId>,
}

#[derive(Debug, Clone)]
struct SceneProjectionEntry {
    node_id: NodeId,
    label: Option<String>,
    kind: String,
    dependencies: Vec<NodeId>,
    incoming_reference_count: usize,
    parameter_dependencies: BTreeSet<ParamId>,
}

impl SceneProjection {
    fn from_scene(scene: &SceneDocument) -> Self {
        let mut entries = BTreeMap::new();
        let mut incoming_counts: BTreeMap<NodeId, usize> = BTreeMap::new();
        for node in scene.nodes().values() {
            let dependencies = node_dependencies(node.kind());
            for dependency in &dependencies {
                *incoming_counts.entry(dependency.clone()).or_insert(0) += 1;
            }
            entries.insert(
                node.id().clone(),
                SceneProjectionEntry {
                    node_id: node.id().clone(),
                    label: node.label().map(str::to_owned),
                    kind: node_kind_name(node.kind()).to_owned(),
                    dependencies,
                    incoming_reference_count: 0,
                    parameter_dependencies: node_parameter_dependencies(
                        node.kind(),
                        node.transform(),
                    ),
                },
            );
        }
        for (node_id, count) in incoming_counts {
            if let Some(entry) = entries.get_mut(&node_id) {
                entry.incoming_reference_count = count;
            }
        }
        let roots = vec![scene.root().clone()];
        let unreferenced = entries
            .values()
            .filter(|entry| entry.node_id != *scene.root() && entry.incoming_reference_count == 0)
            .map(|entry| entry.node_id.clone())
            .collect::<Vec<_>>();
        Self {
            entries,
            roots,
            unreferenced,
        }
    }

    fn ordered_node_ids(&self) -> Vec<NodeId> {
        self.entries.keys().cloned().collect()
    }

    fn dependencies(&self, node_id: &NodeId) -> Vec<String> {
        self.entries
            .get(node_id)
            .map(|entry| entry.dependencies.iter().map(ToString::to_string).collect())
            .unwrap_or_default()
    }

    fn direct_dependents(&self, node_id: &NodeId) -> Vec<String> {
        self.entries
            .values()
            .filter(|entry| {
                entry
                    .dependencies
                    .iter()
                    .any(|dependency| dependency == node_id)
            })
            .map(|entry| entry.node_id.to_string())
            .collect()
    }

    fn transitive_dependents(&self, node_id: &NodeId) -> Vec<String> {
        let mut out = BTreeSet::new();
        let mut stack = self
            .direct_dependents(node_id)
            .into_iter()
            .map(|value| NodeId::new(value).expect("projected dependent id"))
            .collect::<Vec<_>>();
        while let Some(next) = stack.pop() {
            if out.insert(next.clone()) {
                stack.extend(
                    self.direct_dependents(&next)
                        .into_iter()
                        .map(|value| NodeId::new(value).expect("projected dependent id")),
                );
            }
        }
        out.into_iter().map(|node_id| node_id.to_string()).collect()
    }

    fn parameter_dependencies(&self, node_id: &NodeId) -> Vec<String> {
        self.entries
            .get(node_id)
            .map(|entry| {
                entry
                    .parameter_dependencies
                    .iter()
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn parameter_dependents(&self, parameter: &ParamId) -> Vec<String> {
        self.entries
            .values()
            .filter(|entry| entry.parameter_dependencies.contains(parameter))
            .map(|entry| entry.node_id.to_string())
            .collect()
    }

    fn transitive_parameter_dependents(&self, parameter: &ParamId) -> Vec<String> {
        let direct = self
            .parameter_dependents(parameter)
            .into_iter()
            .map(|value| NodeId::new(value).expect("parameter dependent id"))
            .collect::<Vec<_>>();
        let mut out: BTreeSet<NodeId> = direct.iter().cloned().collect();
        let mut stack = direct;
        while let Some(next) = stack.pop() {
            for dependent in self.direct_dependents(&next) {
                let dependent_id = NodeId::new(dependent).expect("transitive parameter dependent");
                if out.insert(dependent_id.clone()) {
                    stack.push(dependent_id);
                }
            }
        }
        out.into_iter().map(|node| node.to_string()).collect()
    }
}

impl From<SceneProjectionEntry> for SceneTreeNodeView {
    fn from(value: SceneProjectionEntry) -> Self {
        Self {
            node_id: value.node_id.to_string(),
            label: value.label,
            kind: value.kind,
            dependencies: value.dependencies.iter().map(ToString::to_string).collect(),
            is_root: false,
            is_shared: value.incoming_reference_count > 1,
        }
    }
}

impl From<&geom_scene::Transform> for TransformView {
    fn from(value: &geom_scene::Transform) -> Self {
        Self {
            translation: VectorExprView::from(&value.translation),
            rotation_deg: VectorExprView::from(&value.rotation_deg),
            scale: VectorExprView::from(&value.scale),
        }
    }
}

impl From<&geom_scene::Vector3Expr> for VectorExprView {
    fn from(value: &geom_scene::Vector3Expr) -> Self {
        Self {
            x: ScalarExprView::from(&value.x),
            y: ScalarExprView::from(&value.y),
            z: ScalarExprView::from(&value.z),
        }
    }
}

impl From<&ScalarExpr> for ScalarExprView {
    fn from(value: &ScalarExpr) -> Self {
        match value {
            ScalarExpr::Literal(number) => Self::Literal {
                value: number.to_string(),
            },
            ScalarExpr::Parameter(parameter) => Self::Parameter {
                parameter_id: parameter.target().to_string(),
            },
        }
    }
}

impl From<&NodeKind> for NodePropertiesView {
    fn from(value: &NodeKind) -> Self {
        match value {
            NodeKind::Box(node) => Self::Box {
                size: VectorExprView::from(&node.size),
            },
            NodeKind::Sphere(node) => Self::Sphere {
                radius: ScalarExprView::from(&node.radius),
            },
            NodeKind::Cylinder(node) => Self::Cylinder {
                radius: ScalarExprView::from(&node.radius),
                height: ScalarExprView::from(&node.height),
            },
            NodeKind::Capsule(node) => Self::Capsule {
                radius: ScalarExprView::from(&node.radius),
                height: ScalarExprView::from(&node.height),
            },
            NodeKind::Plane(node) => Self::Plane {
                width: ScalarExprView::from(&node.width),
                depth: ScalarExprView::from(&node.depth),
            },
            NodeKind::Profile(node) => Self::Profile {
                width: ScalarExprView::from(&node.width),
                height: ScalarExprView::from(&node.height),
            },
            NodeKind::Union(node) => Self::Union {
                children: node
                    .children
                    .iter()
                    .map(|child| child.target().to_string())
                    .collect(),
            },
            NodeKind::Difference(node) => Self::Difference {
                children: node
                    .children
                    .iter()
                    .map(|child| child.target().to_string())
                    .collect(),
            },
            NodeKind::Intersection(node) => Self::Intersection {
                children: node
                    .children
                    .iter()
                    .map(|child| child.target().to_string())
                    .collect(),
            },
        }
    }
}

impl From<SourceLocation> for SourceLocationView {
    fn from(value: SourceLocation) -> Self {
        Self {
            line: value.line,
            column: value.column,
            byte_offset: value.byte_offset,
        }
    }
}

impl From<WorkspaceHistoryEntry> for HistoryEntryView {
    fn from(value: WorkspaceHistoryEntry) -> Self {
        Self {
            transaction_id: value.transaction_id().to_string(),
            actor: actor_name(value.actor()).to_owned(),
            intent: value.intent().map(str::to_owned),
            revision_before: value.revision_before().get(),
            revision_after: value.revision_after().get(),
            timestamp_millis: value.timestamp_millis(),
            affected_node_ids: value
                .affected_targets()
                .node_ids()
                .iter()
                .map(ToString::to_string)
                .collect(),
            affected_parameter_ids: value
                .affected_targets()
                .parameter_ids()
                .iter()
                .map(ToString::to_string)
                .collect(),
            summary: value
                .operations()
                .iter()
                .map(|operation| operation.summary().to_owned())
                .collect::<Vec<_>>()
                .join("; "),
        }
    }
}

impl GeometryStatsView {
    fn from_evaluated(
        value: &EvaluatedGeometry,
        diagnostics: Vec<Diagnostic>,
        parameter_limit: usize,
    ) -> Self {
        Self {
            requested_output: value.requested_output.to_string(),
            bounds: BoundsView::from(&value.bounds),
            vertex_count: value.stats.vertex_count,
            triangle_count: value.stats.triangle_count,
            evaluated_node_count: value.stats.evaluated_node_count,
            participating_node_ids: value
                .participating_node_ids
                .iter()
                .map(ToString::to_string)
                .collect(),
            resolved_parameters: BoundedList::from_items(
                value
                    .resolved_parameters
                    .values()
                    .map(|parameter| ResolvedParameterView {
                        parameter_id: parameter.id().to_string(),
                        value: parameter.value(),
                    })
                    .collect(),
                CollectionBounds::new(parameter_limit),
            ),
            diagnostics,
        }
    }
}

impl From<&Bounds> for BoundsView {
    fn from(value: &Bounds) -> Self {
        match value {
            Bounds::Empty => Self::Empty,
            Bounds::Finite { min, max } => Self::Finite {
                min: *min,
                max: *max,
            },
        }
    }
}

fn parse_workspace_scene(workspace: &Workspace) -> Result<SceneDocument, WorkspaceApiError> {
    parse_scene_report(workspace.source_text()).map_err(|report| WorkspaceApiError::Workspace {
        message: report
            .primary_message()
            .unwrap_or("scene validation failed")
            .to_owned(),
    })
}

fn workspace_diagnostics(workspace: &Workspace) -> Vec<Diagnostic> {
    parse_scene_report(workspace.source_text())
        .err()
        .map(|report| report.diagnostics)
        .unwrap_or_default()
}

fn filtered_diagnostics(
    workspace: &Workspace,
    node_id: Option<&NodeId>,
    parameter_id: Option<&ParamId>,
) -> Vec<Diagnostic> {
    workspace_diagnostics(workspace)
        .into_iter()
        .filter(|diagnostic| {
            node_id.is_none_or(|node_id| diagnostic.node_id.as_deref() == Some(node_id.as_str()))
                && parameter_id.is_none_or(|parameter_id| {
                    diagnostic.parameter_id.as_deref() == Some(parameter_id.as_str())
                })
        })
        .collect()
}

fn evaluate_scene(
    scene: &SceneDocument,
    output: Option<&NodeId>,
) -> Result<EvaluatedGeometry, WorkspaceApiError> {
    let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
    match output {
        Some(node_id) => evaluator.evaluate_node(scene, node_id),
        None => evaluator.evaluate_root(scene),
    }
    .map_err(|error| WorkspaceApiError::Workspace {
        message: error.to_string(),
    })
}

fn backend_and_geometry_diagnostics(
    scene: &SceneDocument,
    output: Option<&NodeId>,
) -> Result<Vec<Diagnostic>, WorkspaceApiError> {
    let mut diagnostics = validate_backend_support(scene).diagnostics;
    if diagnostics.iter().any(|diagnostic| diagnostic.blocking) {
        return Ok(diagnostics);
    }
    let evaluation = evaluate_scene(scene, output)?;
    diagnostics.extend(validate_evaluated_geometry(&evaluation).diagnostics);
    Ok(diagnostics)
}

fn render_preview_artifact(
    workspace: &Workspace,
    evaluation: &EvaluatedGeometry,
    destination: Option<&str>,
    width: u32,
    height: u32,
    overwrite: bool,
) -> Result<PathBuf, WorkspaceApiError> {
    let relative_path = destination
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}.png", evaluation.requested_output.as_str())));
    let absolute_path = workspace
        .resolve_path(WorkspaceDirectory::Exports, &relative_path)
        .map_err(|error| WorkspaceApiError::Workspace {
            message: error.to_string(),
        })?;
    if absolute_path.exists() && !overwrite {
        return Err(WorkspaceApiError::InvalidRequest {
            message: format!(
                "preview destination already exists at `{}`",
                absolute_path.display()
            ),
        });
    }
    let parent = absolute_path
        .parent()
        .ok_or_else(|| WorkspaceApiError::Workspace {
            message: "preview destination has no parent".to_owned(),
        })?;
    fs::create_dir_all(parent).map_err(|error| WorkspaceApiError::Workspace {
        message: error.to_string(),
    })?;
    let image = rasterize_preview(&evaluation.mesh, &evaluation.bounds, width, height);
    image
        .save(&absolute_path)
        .map_err(|error| WorkspaceApiError::Workspace {
            message: error.to_string(),
        })?;
    Ok(absolute_path)
}

fn source_snippet_for_node(
    source_text: &str,
    source: Option<&SceneSource>,
    node_id: &NodeId,
    line_radius: usize,
) -> Option<SourceSnippetView> {
    let location = source?.node_source_location(node_id)?;
    bounded_source_snippet(source_text, location, line_radius)
}

fn source_snippet_for_parameter(
    source_text: &str,
    source: Option<&SceneSource>,
    parameter_id: &ParamId,
    line_radius: usize,
) -> Option<SourceSnippetView> {
    let location = source?.parameter_source_location(parameter_id)?;
    bounded_source_snippet(source_text, location, line_radius)
}

fn bounded_source_snippet(
    source_text: &str,
    location: SourceLocation,
    line_radius: usize,
) -> Option<SourceSnippetView> {
    let lines = source_text.lines().collect::<Vec<_>>();
    let line_index = location.line.checked_sub(1)?;
    let start = line_index.saturating_sub(line_radius);
    let end = (line_index + line_radius + 1).min(lines.len());
    Some(SourceSnippetView {
        start_line: start + 1,
        end_line: end,
        snippet: lines[start..end].join("\n"),
    })
}

fn transaction_from_proposal(
    proposal: &TransactionProposal,
    current_revision: u64,
) -> Result<WorkspaceTransaction, WorkspaceApiError> {
    if proposal.expected_revision != current_revision {
        return Err(WorkspaceApiError::StaleRevision {
            expected: proposal.expected_revision,
            current: current_revision,
        });
    }
    let operations = proposal
        .operations
        .iter()
        .map(workspace_op_from_request)
        .collect::<Result<Vec<_>, _>>()?;
    WorkspaceTransaction::new(
        parse_actor(&proposal.actor)?,
        proposal.intent.clone(),
        operations,
    )
    .map_err(|error| WorkspaceApiError::Workspace {
        message: error.to_string(),
    })
}

fn workspace_op_from_request(
    request: &WorkspaceOpRequest,
) -> Result<WorkspaceOp, WorkspaceApiError> {
    Ok(match request {
        WorkspaceOpRequest::SetParameterScalar {
            parameter_id,
            value,
        } => WorkspaceOp::SetParameterScalar {
            id: Default::default(),
            parameter_id: parse_param_id(parameter_id)?,
            value: *value,
        },
        WorkspaceOpRequest::SetNodeLabel { node_id, label } => WorkspaceOp::SetNodeLabel {
            id: Default::default(),
            node_id: parse_node_id(node_id)?,
            label: label.clone(),
        },
        WorkspaceOpRequest::RenameNode { from, to } => WorkspaceOp::RenameNode {
            id: Default::default(),
            from: parse_node_id(from)?,
            to: parse_node_id(to)?,
        },
        WorkspaceOpRequest::SetRootNode { node_id } => WorkspaceOp::SetRootNode {
            id: Default::default(),
            node_id: parse_node_id(node_id)?,
        },
        WorkspaceOpRequest::DeleteNode { node_id } => WorkspaceOp::DeleteNode {
            id: Default::default(),
            node_id: parse_node_id(node_id)?,
        },
    })
}

fn dry_run_diff(
    workspace: &Workspace,
    transaction: &WorkspaceTransaction,
) -> Result<SceneDiffView, WorkspaceApiError> {
    let before_scene = parse_workspace_scene(workspace)?;
    let mut source = SceneSource::parse(workspace.source_text()).map_err(scene_validation_error)?;
    for operation in transaction.operations() {
        apply_workspace_op_in_memory(&mut source, operation)?;
    }
    let after_scene = source.validate().map_err(scene_validation_error)?;
    Ok(build_scene_diff(
        workspace.revision().get(),
        workspace.revision().get(),
        &before_scene,
        &after_scene,
    ))
}

fn apply_workspace_op_in_memory(
    source: &mut SceneSource,
    operation: &WorkspaceOp,
) -> Result<(), WorkspaceApiError> {
    match operation {
        WorkspaceOp::SetParameterScalar {
            parameter_id,
            value,
            ..
        } => {
            source
                .set_parameter_scalar(parameter_id, *value)
                .map_err(scene_validation_error)?;
        }
        WorkspaceOp::SetNodeLabel { node_id, label, .. } => {
            source
                .set_node_label(node_id, label.as_deref())
                .map_err(scene_validation_error)?;
        }
        WorkspaceOp::RenameNode { from, to, .. } => {
            source
                .rename_node(from, to)
                .map_err(scene_validation_error)?;
        }
        WorkspaceOp::SetRootNode { node_id, .. } => {
            source
                .set_root_node(node_id)
                .map_err(scene_validation_error)?;
        }
        WorkspaceOp::DeleteNode { node_id, .. } => {
            source
                .delete_node(node_id)
                .map_err(scene_validation_error)?;
        }
        other => {
            return Err(WorkspaceApiError::Workspace {
                message: format!("unsupported dry-run operation `{other:?}`"),
            });
        }
    }
    Ok(())
}

fn build_scene_diff(
    before_revision: u64,
    after_revision: u64,
    before: &SceneDocument,
    after: &SceneDocument,
) -> SceneDiffView {
    let mut changes = Vec::new();
    if before.root() != after.root() {
        changes.push(WorkspaceSceneChange::RootChanged {
            before: before.root().clone(),
            after: after.root().clone(),
        });
    }
    for (parameter_id, before_parameter) in before.parameters() {
        match after.parameters().get(parameter_id) {
            Some(after_parameter)
                if before_parameter.scalar_value() != after_parameter.scalar_value() =>
            {
                changes.push(WorkspaceSceneChange::ParameterChanged {
                    id: parameter_id.clone(),
                    before: before_parameter.scalar_value(),
                    after: after_parameter.scalar_value(),
                });
            }
            None => changes.push(WorkspaceSceneChange::ParameterRemoved {
                id: parameter_id.clone(),
                value: before_parameter.scalar_value(),
            }),
            _ => {}
        }
    }
    for (parameter_id, after_parameter) in after.parameters() {
        if !before.parameters().contains_key(parameter_id) {
            changes.push(WorkspaceSceneChange::ParameterAdded {
                id: parameter_id.clone(),
                value: after_parameter.scalar_value(),
            });
        }
    }
    for (node_id, before_node) in before.nodes() {
        match after.nodes().get(node_id) {
            Some(after_node) if before_node != after_node => {
                changes.push(WorkspaceSceneChange::NodeChanged {
                    id: node_id.clone(),
                    before_kind: node_kind_name(before_node.kind()),
                    after_kind: node_kind_name(after_node.kind()),
                    fields: Vec::new(),
                })
            }
            None => changes.push(WorkspaceSceneChange::NodeRemoved {
                id: node_id.clone(),
                kind: node_kind_name(before_node.kind()),
            }),
            _ => {}
        }
    }
    for (node_id, after_node) in after.nodes() {
        if !before.nodes().contains_key(node_id) {
            changes.push(WorkspaceSceneChange::NodeAdded {
                id: node_id.clone(),
                kind: node_kind_name(after_node.kind()),
            });
        }
    }
    SceneDiffView {
        before_revision,
        after_revision,
        summary: format!("{} semantic changes", changes.len()),
        changes: changes.iter().map(scene_change_summary).collect(),
    }
}

fn scene_validation_error(error: geom_scene::SceneError) -> WorkspaceApiError {
    WorkspaceApiError::Validation {
        message: error.to_string(),
        diagnostics: vec![diagnostic_from_scene_error(&error, None)],
    }
}

fn workspace_transaction_error(error: WorkspaceTransactionError) -> WorkspaceApiError {
    match error {
        WorkspaceTransactionError::SceneValidation { source } => scene_validation_error(source),
        other => WorkspaceApiError::Workspace {
            message: other.to_string(),
        },
    }
}

fn rebuild_scope_from_targets(targets: &AffectedTargets) -> RebuildScopeView {
    RebuildScopeView {
        affected_node_ids: targets.node_ids().iter().map(ToString::to_string).collect(),
        affected_parameter_ids: targets
            .parameter_ids()
            .iter()
            .map(ToString::to_string)
            .collect(),
    }
}

fn scene_change_summary(change: &WorkspaceSceneChange) -> String {
    match change {
        WorkspaceSceneChange::RootChanged { before, after } => {
            format!("root changed from `{before}` to `{after}`")
        }
        WorkspaceSceneChange::ParameterAdded { id, value } => {
            format!("parameter `{id}` added with value {value}")
        }
        WorkspaceSceneChange::ParameterRemoved { id, value } => {
            format!("parameter `{id}` removed from value {value}")
        }
        WorkspaceSceneChange::ParameterChanged { id, before, after } => {
            format!("parameter `{id}` changed from {before} to {after}")
        }
        WorkspaceSceneChange::NodeAdded { id, kind } => format!("node `{id}` added as `{kind}`"),
        WorkspaceSceneChange::NodeRemoved { id, kind } => {
            format!("node `{id}` removed from `{kind}`")
        }
        WorkspaceSceneChange::NodeChanged {
            id,
            before_kind,
            after_kind,
            ..
        } => format!("node `{id}` changed from `{before_kind}` to `{after_kind}`"),
    }
}

fn actor_name(actor: TransactionActor) -> &'static str {
    match actor {
        TransactionActor::User => "user",
        TransactionActor::Ai => "ai",
        TransactionActor::CliAutomation => "cli_automation",
        TransactionActor::SystemMigration => "system_migration",
    }
}

fn parse_actor(value: &str) -> Result<TransactionActor, WorkspaceApiError> {
    match value {
        "user" => Ok(TransactionActor::User),
        "ai" => Ok(TransactionActor::Ai),
        "cli_automation" => Ok(TransactionActor::CliAutomation),
        "system_migration" => Ok(TransactionActor::SystemMigration),
        _ => Err(WorkspaceApiError::InvalidRequest {
            message: format!("unknown actor `{value}`"),
        }),
    }
}

fn parse_node_id(value: &str) -> Result<NodeId, WorkspaceApiError> {
    NodeId::new(value).map_err(|error| WorkspaceApiError::InvalidRequest {
        message: error.to_string(),
    })
}

fn parse_param_id(value: &str) -> Result<ParamId, WorkspaceApiError> {
    ParamId::new(value).map_err(|error| WorkspaceApiError::InvalidRequest {
        message: error.to_string(),
    })
}

fn node_kind_name(kind: &NodeKind) -> &'static str {
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

fn node_dependencies(kind: &NodeKind) -> Vec<NodeId> {
    match kind {
        NodeKind::Union(node) | NodeKind::Difference(node) | NodeKind::Intersection(node) => node
            .children
            .iter()
            .map(|child| child.target().clone())
            .collect(),
        _ => Vec::new(),
    }
}

fn node_parameter_dependencies(
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

fn rasterize_preview(
    mesh: &geom_geometry::Mesh,
    bounds: &Bounds,
    width: u32,
    height: u32,
) -> image::ImageBuffer<image::Rgba<u8>, Vec<u8>> {
    use image::{ImageBuffer, Rgba};
    let mut image = ImageBuffer::from_pixel(width, height, Rgba([18, 20, 26, 255]));
    if mesh.positions().is_empty() || mesh.triangle_indices().is_empty() {
        return image;
    }
    let (min, max) = match bounds {
        Bounds::Empty => return image,
        Bounds::Finite { min, max } => (*min, *max),
    };
    let extent_x = (max[0] - min[0]).abs().max(1e-6);
    let extent_y = (max[1] - min[1]).abs().max(1e-6);
    let scale = ((width as f64 - 20.0) / extent_x)
        .min((height as f64 - 20.0) / extent_y)
        .max(1.0);
    let mut depth = vec![f32::INFINITY; (width * height) as usize];
    for triangle in mesh.triangle_indices() {
        let projected = triangle.map(|index| {
            let position = mesh.positions()[index as usize];
            let x = ((position[0] - min[0]) * scale + 10.0) as f32;
            let y = (height as f64 - ((position[1] - min[1]) * scale + 10.0)) as f32;
            (x, y, position[2] as f32)
        });
        fill_triangle(
            &mut image,
            &mut depth,
            projected,
            image::Rgba([126, 196, 255, 255]),
        );
    }
    image
}

fn fill_triangle(
    image: &mut image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    depth: &mut [f32],
    triangle: [(f32, f32, f32); 3],
    color: image::Rgba<u8>,
) {
    let width = image.width() as i32;
    let height = image.height() as i32;
    let min_x = triangle
        .iter()
        .map(|vertex| vertex.0.floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, width.saturating_sub(1));
    let max_x = triangle
        .iter()
        .map(|vertex| vertex.0.ceil() as i32)
        .max()
        .unwrap_or(0)
        .clamp(0, width.saturating_sub(1));
    let min_y = triangle
        .iter()
        .map(|vertex| vertex.1.floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, height.saturating_sub(1));
    let max_y = triangle
        .iter()
        .map(|vertex| vertex.1.ceil() as i32)
        .max()
        .unwrap_or(0)
        .clamp(0, height.saturating_sub(1));
    let area = edge_function(triangle[0], triangle[1], triangle[2].0, triangle[2].1);
    if area.abs() <= f32::EPSILON {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let sample_x = x as f32 + 0.5;
            let sample_y = y as f32 + 0.5;
            let w0 = edge_function(triangle[1], triangle[2], sample_x, sample_y) / area;
            let w1 = edge_function(triangle[2], triangle[0], sample_x, sample_y) / area;
            let w2 = edge_function(triangle[0], triangle[1], sample_x, sample_y) / area;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * triangle[0].2 + w1 * triangle[1].2 + w2 * triangle[2].2;
            let index = (y as u32 * image.width() + x as u32) as usize;
            if z < depth[index] {
                depth[index] = z;
                image.put_pixel(x as u32, y as u32, color);
            }
        }
    }
}

fn edge_function(a: (f32, f32, f32), b: (f32, f32, f32), x: f32, y: f32) -> f32 {
    (x - a.0) * (b.1 - a.1) - (y - a.1) * (b.0 - a.0)
}
