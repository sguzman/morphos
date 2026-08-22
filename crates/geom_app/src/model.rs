use crate::reactive::{
    BuildAcceptance, BuildGeneration, BuildOutcome, BuildOutcomeKind, BuildRequestSnapshot,
    DiagnosticStage, EditOrigin, ReactiveBuildSuccess, ReactiveBuildTimings, ReactiveController,
    ReactiveDiagnostic, ReactiveStatusSnapshot, SourceFingerprint, SourceRevision,
};
use crate::scene_tree::SceneTreeModel;
use crate::viewport::DisplayGeometryRevision;
use bevy::prelude::Resource;
use geom_diagnostics::{Diagnostic, DiagnosticReport};
use geom_geometry::{BoolmeshBackend, Bounds, GeometryEvaluator};
use geom_scene::{
    Axis, NodeId, ParamId, PrimitiveScalarField, SceneDocument, SceneNodeDraft, SourceLocation,
    TransformProperty,
};
use geom_workspace::{
    OperationId, Revision, TransactionActor, UndoRedoAvailability, UndoRedoManager, Workspace,
    WorkspaceError, WorkspaceOp, WorkspaceSummary, WorkspaceTransaction, WorkspaceTransactionError,
};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteNodeSafety {
    pub is_root: bool,
    pub direct_dependents: Vec<NodeId>,
    pub transitive_dependents: Vec<NodeId>,
}

#[derive(Debug, Resource)]
pub struct AppModel {
    workspace_path: Option<PathBuf>,
    workspace: Option<Workspace>,
    last_good_scene: Option<SceneDocument>,
    displayed_geometry: Option<DisplayedGeometry>,
    build_status: AppBuildStatus,
    reactive: ReactiveController,
    undo_redo: UndoRedoManager,
}

impl AppModel {
    pub fn new(workspace_path: Option<PathBuf>) -> Self {
        Self {
            workspace_path,
            workspace: None,
            last_good_scene: None,
            displayed_geometry: None,
            build_status: AppBuildStatus::NoWorkspace,
            reactive: ReactiveController::new(),
            undo_redo: UndoRedoManager::default(),
        }
    }

    pub fn build_status(&self) -> &AppBuildStatus {
        &self.build_status
    }

    pub fn current_scene(&self) -> Option<&SceneDocument> {
        self.last_good_scene.as_ref()
    }

    pub fn displayed_geometry(&self) -> Option<&DisplayedGeometry> {
        self.displayed_geometry.as_ref()
    }

    pub fn workspace_summary(&self) -> Option<WorkspaceSummary> {
        self.workspace.as_ref().map(Workspace::summary)
    }

    pub fn workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.workspace.as_mut()
    }

    pub fn reactive_status_snapshot(&self) -> ReactiveStatusSnapshot {
        self.reactive.status_snapshot()
    }

    pub fn current_source_revision(&self) -> SourceRevision {
        self.reactive.current_source_revision()
    }

    pub fn prepare_reopen(
        &mut self,
        now: Instant,
    ) -> Result<BuildRequestSnapshot, WorkspaceBuildError> {
        self.open_workspace_from_path()?;
        let workspace = self.workspace.as_ref().expect("workspace opened");
        self.last_good_scene = None;
        self.displayed_geometry = None;
        self.build_status = AppBuildStatus::NoWorkspace;
        self.undo_redo.clear();
        Ok(self
            .reactive
            .begin_session(workspace.source_text(), EditOrigin::StartupReopen, now))
    }

    pub fn schedule_manual_rebuild(
        &mut self,
        now: Instant,
    ) -> Result<BuildRequestSnapshot, WorkspaceBuildError> {
        let Some(workspace) = self.workspace.as_mut() else {
            self.build_status = AppBuildStatus::NoWorkspace;
            return Err(WorkspaceBuildError::NoWorkspacePath);
        };

        let changed = workspace
            .reload_source()
            .map_err(WorkspaceBuildError::Workspace)?;
        if changed
            && let Some(snapshot) = self.reactive.accept_external_source_reload(
                workspace.source_text(),
                EditOrigin::ManualReload,
                now,
            )
        {
            self.undo_redo.clear();
            return Ok(snapshot);
        }

        Ok(self.reactive.note_manual_rebuild(
            workspace.source_text(),
            EditOrigin::ManualReload,
            now,
        ))
    }

    pub fn note_file_event(&mut self, observed_at: Instant) {
        self.reactive.observe_file_event(observed_at);
    }

    pub fn drain_ready_file_event(&mut self, now: Instant) -> Option<usize> {
        self.reactive.drain_ready_file_event(now)
    }

    pub fn accept_external_reload(
        &mut self,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, WorkspaceBuildError> {
        let Some(workspace) = self.workspace.as_mut() else {
            self.build_status = AppBuildStatus::NoWorkspace;
            return Ok(None);
        };

        let changed = workspace
            .reload_source()
            .map_err(WorkspaceBuildError::Workspace)?;
        if !changed {
            let _ = self
                .reactive
                .clear_own_write_if_matches_current_source(workspace.source_text());
            return Ok(None);
        }

        self.undo_redo.clear();
        Ok(self.reactive.accept_external_source_reload(
            workspace.source_text(),
            EditOrigin::ExternalFile,
            now,
        ))
    }

    pub fn undo_redo_availability(&self) -> UndoRedoAvailability {
        self.undo_redo.availability()
    }

    pub fn apply_parameter_scalar_delta(
        &mut self,
        parameter: &ParamId,
        delta: f64,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let current_value = self
            .last_good_scene
            .as_ref()
            .and_then(|scene| scene.parameters().get(parameter))
            .map(|parameter_definition| parameter_definition.scalar_value())
            .ok_or_else(|| {
                AppEditError::Conflict("parameter is unavailable for editing".to_owned())
            })?;

        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!("Adjust parameter {}", parameter.as_str())),
            vec![WorkspaceOp::SetParameterScalar {
                id: OperationId::new(),
                parameter_id: parameter.clone(),
                value: current_value + delta,
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn set_parameter_scalar_value(
        &mut self,
        parameter: &ParamId,
        value: f64,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!("Set parameter {}", parameter.as_str())),
            vec![WorkspaceOp::SetParameterScalar {
                id: OperationId::new(),
                parameter_id: parameter.clone(),
                value,
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn parameter_scalar(&self, parameter: &str) -> Option<f64> {
        let parameter = ParamId::new(parameter).ok()?;
        self.last_good_scene
            .as_ref()
            .and_then(|scene| scene.parameters().get(&parameter))
            .map(|definition| definition.scalar_value())
    }

    pub fn frame_selected_bounds(&self, selection: &ViewportSelection) -> Option<Bounds> {
        let selected = selection.selected_node.as_ref()?;
        let scene = self.last_good_scene.as_ref()?;
        let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
        evaluator
            .evaluate_node(scene, selected)
            .ok()
            .map(|geometry| geometry.bounds)
    }

    pub fn scene_tree_model(&self) -> Option<SceneTreeModel> {
        self.last_good_scene
            .as_ref()
            .map(SceneTreeModel::from_scene)
    }

    pub fn delete_node_safety(&self, node: &NodeId) -> Option<DeleteNodeSafety> {
        let scene = self.last_good_scene.as_ref()?;
        let tree = SceneTreeModel::from_scene(scene);
        Some(DeleteNodeSafety {
            is_root: scene.root() == node,
            direct_dependents: tree.direct_dependents(node),
            transitive_dependents: tree.transitive_dependents(node),
        })
    }

    pub fn editing_disabled_reason(&self) -> Option<&str> {
        match &self.build_status {
            AppBuildStatus::SceneError(_) => Some(
                "editing is disabled while the current source is invalid; fix or reload the source first",
            ),
            AppBuildStatus::Conflict(message) => Some(message.as_str()),
            _ => None,
        }
    }

    pub fn preserve_selection(&self, selection: &mut ViewportSelection) {
        let Some(scene) = self.last_good_scene.as_ref() else {
            selection.selected_node = None;
            return;
        };
        if let Some(selected) = selection.selected_node.as_ref()
            && scene.nodes().contains_key(selected)
        {
            return;
        }
        selection.selected_node = Some(scene.root().clone());
    }

    pub fn set_selected_node_transform_literal(
        &mut self,
        node: &NodeId,
        property: TransformProperty,
        axis: Axis,
        value: f64,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!("Set {} {:?} {:?}", node.as_str(), property, axis)),
            vec![WorkspaceOp::SetTransformComponent {
                id: OperationId::new(),
                node_id: node.clone(),
                property,
                axis,
                value,
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn set_selected_node_primitive_literal(
        &mut self,
        node: &NodeId,
        field: PrimitiveScalarField,
        value: f64,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!("Set primitive field on {}", node.as_str())),
            vec![WorkspaceOp::SetPrimitiveScalar {
                id: OperationId::new(),
                node_id: node.clone(),
                field,
                value,
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn set_node_label(
        &mut self,
        node: &NodeId,
        label: Option<&str>,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!("Set label on {}", node.as_str())),
            vec![WorkspaceOp::SetNodeLabel {
                id: OperationId::new(),
                node_id: node.clone(),
                label: label.map(ToOwned::to_owned),
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn rename_node(
        &mut self,
        from: &NodeId,
        to: &NodeId,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!("Rename {} to {}", from.as_str(), to.as_str())),
            vec![WorkspaceOp::RenameNode {
                id: OperationId::new(),
                from: from.clone(),
                to: to.clone(),
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn duplicate_node(
        &mut self,
        source_node: &NodeId,
        duplicate: &NodeId,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!(
                "Duplicate {} to {}",
                source_node.as_str(),
                duplicate.as_str()
            )),
            vec![WorkspaceOp::DuplicateNode {
                id: OperationId::new(),
                source_node: source_node.clone(),
                duplicate: duplicate.clone(),
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn add_node(
        &mut self,
        node: &NodeId,
        draft: SceneNodeDraft,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!("Add node {}", node.as_str())),
            vec![WorkspaceOp::AddNode {
                id: OperationId::new(),
                node_id: node.clone(),
                draft,
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn set_root_node(
        &mut self,
        node: &NodeId,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!("Set root to {}", node.as_str())),
            vec![WorkspaceOp::SetRootNode {
                id: OperationId::new(),
                node_id: node.clone(),
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn set_composition_children(
        &mut self,
        node: &NodeId,
        children: &[NodeId],
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!("Update composition children for {}", node.as_str())),
            vec![WorkspaceOp::SetCompositionChildren {
                id: OperationId::new(),
                node_id: node.clone(),
                children: children.to_vec(),
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn delete_node(
        &mut self,
        node: &NodeId,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        let transaction = WorkspaceTransaction::new(
            transaction_actor_for_origin(origin),
            Some(format!("Delete node {}", node.as_str())),
            vec![WorkspaceOp::DeleteNode {
                id: OperationId::new(),
                node_id: node.clone(),
            }],
        )
        .expect("single operation transaction");
        self.apply_workspace_transaction(origin, now, &transaction)
    }

    pub fn node_source_location(&self, node: &NodeId) -> Option<SourceLocation> {
        let workspace = self.workspace.as_ref()?;
        let source = geom_scene::SceneSource::parse(workspace.source_text()).ok()?;
        source.node_source_location(node)
    }

    pub fn parameter_source_location(&self, parameter: &ParamId) -> Option<SourceLocation> {
        let workspace = self.workspace.as_ref()?;
        let source = geom_scene::SceneSource::parse(workspace.source_text()).ok()?;
        source.parameter_source_location(parameter)
    }

    pub fn undo(
        &mut self,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        self.apply_undo_redo(origin, now, true)
    }

    pub fn redo(
        &mut self,
        origin: EditOrigin,
        now: Instant,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        self.apply_undo_redo(origin, now, false)
    }

    pub fn accept_build_outcome(&mut self, outcome: BuildOutcome) -> BuildApplicationAction {
        match self.reactive.accept_build_outcome(outcome) {
            BuildAcceptance::IgnoredStale => BuildApplicationAction::none(),
            BuildAcceptance::Accepted(outcome) => match outcome.kind {
                BuildOutcomeKind::Success(success) => self.accept_success(
                    outcome.source_revision,
                    outcome.generation,
                    outcome.requested_at,
                    *success,
                ),
                BuildOutcomeKind::Failure {
                    stage,
                    report,
                    timings,
                } => {
                    self.accept_failure(
                        stage,
                        report,
                        outcome.source_revision,
                        outcome.generation,
                        timings,
                    );
                    BuildApplicationAction::none()
                }
            },
        }
    }

    pub fn note_mesh_upload_complete(&mut self, upload_millis: f64, total_millis: f64) {
        self.reactive
            .note_mesh_upload_complete(upload_millis, total_millis);
        if let AppBuildStatus::Success(success) = &mut self.build_status {
            success.timings.mesh_upload_millis = upload_millis;
            success.timings.total_millis = total_millis;
        }
    }

    pub fn ui_status_snapshot(
        &self,
        geometry_revision: DisplayGeometryRevision,
        selection: &ViewportSelection,
    ) -> UiStatusSnapshot {
        let workspace_summary = self.workspace_summary();
        let reactive = self.reactive.status_snapshot();
        let current_output = self
            .displayed_geometry
            .as_ref()
            .map(|geometry| geometry.requested_output.to_string());
        let selection_label = selection.selected_node.as_ref().map(ToString::to_string);

        let (build_kind, build_label, error_message, diagnostics) = match &self.build_status {
            AppBuildStatus::NoWorkspace => (
                BuildStatusKind::NoWorkspace,
                "No workspace".to_owned(),
                None,
                Vec::new(),
            ),
            AppBuildStatus::WorkspaceError(message) => (
                BuildStatusKind::WorkspaceError,
                "Workspace error".to_owned(),
                Some(message.clone()),
                Vec::new(),
            ),
            AppBuildStatus::Conflict(message) => (
                BuildStatusKind::Conflict,
                "Edit conflict".to_owned(),
                Some(message.clone()),
                Vec::new(),
            ),
            AppBuildStatus::SceneError(diagnostic) => (
                BuildStatusKind::SceneError,
                "Scene error".to_owned(),
                diagnostic.report.primary_message().map(ToOwned::to_owned),
                diagnostic.report.diagnostics.clone(),
            ),
            AppBuildStatus::GeometryError(diagnostic) => (
                BuildStatusKind::GeometryError,
                "Geometry error".to_owned(),
                diagnostic.report.primary_message().map(ToOwned::to_owned),
                diagnostic.report.diagnostics.clone(),
            ),
            AppBuildStatus::Success(success) => {
                let kind = if success.displayed_geometry_revision == DisplayGeometryRevision::ZERO {
                    BuildStatusKind::EmptyMesh
                } else {
                    BuildStatusKind::Success
                };
                (kind, "Success".to_owned(), None, Vec::new())
            }
        };

        UiStatusSnapshot {
            workspace_name: workspace_summary
                .as_ref()
                .map(|summary| summary.name().to_owned()),
            workspace_path: self.workspace_path.clone(),
            workspace_revision: workspace_summary
                .as_ref()
                .map(WorkspaceSummary::revision)
                .unwrap_or(Revision::ZERO),
            workspace_dirty: workspace_summary
                .as_ref()
                .map(WorkspaceSummary::is_dirty)
                .unwrap_or(false),
            source_revision: reactive.source_revision,
            geometry_revision,
            build_generation: reactive.newest_generation,
            last_successful_generation: reactive.last_successful_generation,
            current_error_generation: reactive.current_error_generation,
            watching: reactive.watching,
            build_in_progress: reactive.build_in_progress,
            build_kind,
            build_label,
            error_message,
            diagnostics,
            displaying_last_good_geometry: matches!(
                self.build_status,
                AppBuildStatus::SceneError(_) | AppBuildStatus::GeometryError(_)
            ) && self.displayed_geometry.is_some(),
            timings: reactive.timings,
            current_output,
            selection: selection_label,
        }
    }

    fn accept_success(
        &mut self,
        source_revision: SourceRevision,
        generation: BuildGeneration,
        requested_at: Instant,
        success: ReactiveBuildSuccess,
    ) -> BuildApplicationAction {
        self.last_good_scene = Some(success.scene.clone());
        let mut refresh_displayed_geometry = false;
        let displayed_geometry_revision = if let Some(geometry) = success.geometry.clone() {
            let geometry_revision = DisplayGeometryRevision::new(geometry.geometry_revision);
            self.displayed_geometry = Some(geometry);
            refresh_displayed_geometry = true;
            geometry_revision
        } else {
            self.displayed_geometry
                .as_ref()
                .map(|geometry| DisplayGeometryRevision::new(geometry.geometry_revision))
                .unwrap_or(DisplayGeometryRevision::ZERO)
        };

        let requested_output = self
            .displayed_geometry
            .as_ref()
            .map(|geometry| geometry.requested_output.clone());

        self.build_status = AppBuildStatus::Success(Box::new(BuildSuccess {
            workspace_summary: self.workspace_summary(),
            source_revision,
            generation,
            requested_output,
            semantic_scene_changed: success.semantic_scene_changed,
            changed_node_ids: success.changed_node_ids,
            changed_parameter_ids: success.changed_parameter_ids,
            displayed_geometry_revision,
            timings: success.timings,
        }));

        if refresh_displayed_geometry {
            BuildApplicationAction {
                refresh_displayed_geometry: true,
                requested_at: Some(requested_at),
            }
        } else {
            BuildApplicationAction::none()
        }
    }

    fn accept_failure(
        &mut self,
        stage: DiagnosticStage,
        report: DiagnosticReport,
        source_revision: SourceRevision,
        generation: BuildGeneration,
        _timings: ReactiveBuildTimings,
    ) {
        let diagnostic = Box::new(ReactiveDiagnostic {
            stage,
            source_revision,
            generation,
            report,
        });
        self.build_status = match stage {
            DiagnosticStage::Workspace => AppBuildStatus::WorkspaceError(
                diagnostic
                    .report
                    .primary_message()
                    .unwrap_or("workspace error")
                    .to_owned(),
            ),
            DiagnosticStage::Conflict => AppBuildStatus::Conflict(
                diagnostic
                    .report
                    .primary_message()
                    .unwrap_or("edit conflict")
                    .to_owned(),
            ),
            DiagnosticStage::Scene => AppBuildStatus::SceneError(diagnostic),
            DiagnosticStage::Geometry => AppBuildStatus::GeometryError(diagnostic),
        };
    }

    fn open_workspace_from_path(&mut self) -> Result<(), WorkspaceBuildError> {
        let path = self
            .workspace_path
            .clone()
            .ok_or(WorkspaceBuildError::NoWorkspacePath)?;
        let workspace = Workspace::open(&path).map_err(WorkspaceBuildError::Workspace)?;
        self.workspace = Some(workspace);
        Ok(())
    }

    fn apply_workspace_transaction(
        &mut self,
        origin: EditOrigin,
        now: Instant,
        transaction: &WorkspaceTransaction,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        if let Some(reason) = self.editing_disabled_reason() {
            return Err(AppEditError::Conflict(reason.to_owned()));
        }

        let base_fingerprint = self
            .reactive
            .current_source_fingerprint()
            .ok_or_else(|| AppEditError::Conflict("no current source fingerprint".to_owned()))?;
        let Some(workspace) = self.workspace.as_mut() else {
            self.build_status = AppBuildStatus::NoWorkspace;
            return Err(AppEditError::Conflict("no workspace is open".to_owned()));
        };

        let disk_source =
            fs::read_to_string(workspace.paths().source_file()).map_err(|source| {
                AppEditError::Workspace(WorkspaceError::Io {
                    path: workspace.paths().source_file(),
                    operation: "read current source for conflict check",
                    source,
                })
            })?;
        if SourceFingerprint::from_text(&disk_source) != base_fingerprint {
            let message =
                "external source changed since the GUI edit base; retry against the newest source"
                    .to_owned();
            self.build_status = AppBuildStatus::Conflict(message.clone());
            return Err(AppEditError::Conflict(message));
        }

        let revision_before = workspace.revision();
        let commit = workspace
            .apply_transaction(transaction)
            .map_err(AppEditError::Transaction)?;
        if workspace.revision() == revision_before {
            return Ok(None);
        }
        self.undo_redo.record_commit(&commit);

        Ok(Some(self.reactive.accept_internal_source_write(
            workspace.source_text(),
            origin,
            now,
        )))
    }

    fn apply_undo_redo(
        &mut self,
        origin: EditOrigin,
        now: Instant,
        is_undo: bool,
    ) -> Result<Option<BuildRequestSnapshot>, AppEditError> {
        if let Some(reason) = self.editing_disabled_reason() {
            return Err(AppEditError::Conflict(reason.to_owned()));
        }

        let base_fingerprint = self
            .reactive
            .current_source_fingerprint()
            .ok_or_else(|| AppEditError::Conflict("no current source fingerprint".to_owned()))?;
        let Some(workspace) = self.workspace.as_mut() else {
            self.build_status = AppBuildStatus::NoWorkspace;
            return Err(AppEditError::Conflict("no workspace is open".to_owned()));
        };

        let disk_source =
            fs::read_to_string(workspace.paths().source_file()).map_err(|source| {
                AppEditError::Workspace(WorkspaceError::Io {
                    path: workspace.paths().source_file(),
                    operation: "read current source for conflict check",
                    source,
                })
            })?;
        if SourceFingerprint::from_text(&disk_source) != base_fingerprint {
            let message =
                "external source changed since the GUI edit base; retry against the newest source"
                    .to_owned();
            self.build_status = AppBuildStatus::Conflict(message.clone());
            return Err(AppEditError::Conflict(message));
        }

        let commit = if is_undo {
            self.undo_redo
                .undo(workspace, transaction_actor_for_origin(origin))
        } else {
            self.undo_redo
                .redo(workspace, transaction_actor_for_origin(origin))
        }
        .map_err(AppEditError::Transaction)?;

        let Some(_commit) = commit else {
            return Ok(None);
        };

        Ok(Some(self.reactive.accept_internal_source_write(
            workspace.source_text(),
            origin,
            now,
        )))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppBuildStatus {
    NoWorkspace,
    WorkspaceError(String),
    Conflict(String),
    SceneError(Box<ReactiveDiagnostic>),
    GeometryError(Box<ReactiveDiagnostic>),
    Success(Box<BuildSuccess>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildSuccess {
    pub workspace_summary: Option<WorkspaceSummary>,
    pub source_revision: SourceRevision,
    pub generation: BuildGeneration,
    pub requested_output: Option<NodeId>,
    pub semantic_scene_changed: bool,
    pub changed_node_ids: Vec<NodeId>,
    pub changed_parameter_ids: Vec<ParamId>,
    pub displayed_geometry_revision: DisplayGeometryRevision,
    pub timings: ReactiveBuildTimings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayedGeometry {
    pub requested_output: NodeId,
    pub geometry_revision: u64,
    pub mesh: geom_geometry::Mesh,
    pub bounds: Bounds,
}

impl DisplayedGeometry {
    pub fn from_evaluated(geometry: geom_geometry::EvaluatedGeometry) -> Self {
        Self {
            requested_output: geometry.requested_output,
            geometry_revision: geometry.evaluation_revision,
            mesh: geometry.mesh,
            bounds: geometry.bounds,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportDisplayMode {
    Shaded,
    Wireframe,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewportSelection {
    pub selected_node: Option<NodeId>,
}

impl ViewportSelection {
    pub fn select(&mut self, node: Option<NodeId>) {
        self.selected_node = node;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildRequest {
    pub action: AppRebuildActionKind,
}

impl BuildRequest {
    pub fn rebuild() -> Self {
        Self {
            action: AppRebuildActionKind::Rebuild,
        }
    }

    pub fn reopen() -> Self {
        Self {
            action: AppRebuildActionKind::Reopen,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppRebuildActionKind {
    Rebuild,
    Reopen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStatusKind {
    NoWorkspace,
    WorkspaceError,
    Conflict,
    SceneError,
    GeometryError,
    EmptyMesh,
    Success,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiStatusSnapshot {
    pub workspace_name: Option<String>,
    pub workspace_path: Option<PathBuf>,
    pub workspace_revision: Revision,
    pub workspace_dirty: bool,
    pub source_revision: SourceRevision,
    pub geometry_revision: DisplayGeometryRevision,
    pub build_generation: BuildGeneration,
    pub last_successful_generation: Option<BuildGeneration>,
    pub current_error_generation: Option<BuildGeneration>,
    pub watching: bool,
    pub build_in_progress: bool,
    pub build_kind: BuildStatusKind,
    pub build_label: String,
    pub error_message: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub displaying_last_good_geometry: bool,
    pub timings: Option<ReactiveBuildTimings>,
    pub current_output: Option<String>,
    pub selection: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildApplicationAction {
    refresh_displayed_geometry: bool,
    requested_at: Option<Instant>,
}

impl BuildApplicationAction {
    pub fn none() -> Self {
        Self {
            refresh_displayed_geometry: false,
            requested_at: None,
        }
    }

    pub fn refresh_displayed_geometry(&self) -> bool {
        self.refresh_displayed_geometry
    }

    pub fn requested_at(&self) -> Option<Instant> {
        self.requested_at
    }
}

#[derive(Debug)]
pub enum AppEditError {
    Workspace(WorkspaceError),
    Scene(geom_scene::SceneError),
    Transaction(WorkspaceTransactionError),
    Conflict(String),
}

#[derive(Debug)]
pub enum WorkspaceBuildError {
    NoWorkspacePath,
    Workspace(WorkspaceError),
}

fn transaction_actor_for_origin(origin: EditOrigin) -> TransactionActor {
    match origin {
        EditOrigin::Gui => TransactionActor::User,
        EditOrigin::Programmatic => TransactionActor::CliAutomation,
        EditOrigin::StartupReopen | EditOrigin::ExternalFile | EditOrigin::ManualReload => {
            TransactionActor::SystemMigration
        }
    }
}
