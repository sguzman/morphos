use crate::viewport::DisplayGeometryRevision;
use bevy::prelude::Resource;
use geom_geometry::{
    BoolmeshBackend, Bounds, EvaluatedGeometry, GeometryEvaluator, Mesh as MorphosMesh,
};
use geom_scene::{NodeId, SceneDocument, parse_scene};
use geom_workspace::{Revision, Workspace, WorkspaceError, WorkspaceSummary};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Resource)]
pub struct AppModel {
    workspace_path: Option<PathBuf>,
    workspace: Option<Workspace>,
    latest_scene: Option<SceneDocument>,
    evaluator: GeometryEvaluator<BoolmeshBackend>,
    displayed_geometry: Option<DisplayedGeometry>,
    build_status: AppBuildStatus,
    last_rebuild_millis: Option<f64>,
}

impl AppModel {
    pub fn new(workspace_path: Option<PathBuf>) -> Self {
        Self {
            workspace_path,
            workspace: None,
            latest_scene: None,
            evaluator: GeometryEvaluator::new(BoolmeshBackend::new()),
            displayed_geometry: None,
            build_status: AppBuildStatus::NoWorkspace,
            last_rebuild_millis: None,
        }
    }

    pub fn build_status(&self) -> &AppBuildStatus {
        &self.build_status
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

    pub fn process_build_request(&mut self, request: BuildRequest) -> AppRebuildAction {
        let started = Instant::now();
        let result = match request.action {
            AppRebuildActionKind::Reopen => self.open_workspace_from_path(),
            AppRebuildActionKind::Rebuild => self.reload_or_open_workspace(),
        };

        let rebuild_millis = started.elapsed().as_secs_f64() * 1_000.0;
        self.last_rebuild_millis = Some(rebuild_millis);

        if let Err(error) = result {
            self.latest_scene = None;
            self.build_status = match error {
                WorkspaceBuildError::NoWorkspacePath => AppBuildStatus::NoWorkspace,
                WorkspaceBuildError::Workspace(error) => {
                    AppBuildStatus::WorkspaceError(error.to_string())
                }
            };
            return AppRebuildAction::none();
        }

        let Some(workspace) = self.workspace.as_ref() else {
            self.latest_scene = None;
            self.build_status = AppBuildStatus::NoWorkspace;
            return AppRebuildAction::none();
        };

        let scene = match parse_scene(workspace.source_text()) {
            Ok(scene) => scene,
            Err(error) => {
                self.latest_scene = None;
                self.build_status = AppBuildStatus::SceneError(error.to_string());
                return AppRebuildAction::none();
            }
        };

        let geometry = match self.evaluator.evaluate_root(&scene) {
            Ok(geometry) => geometry,
            Err(error) => {
                self.latest_scene = Some(scene);
                self.build_status = AppBuildStatus::GeometryError(error.to_string());
                return AppRebuildAction::none();
            }
        };

        self.latest_scene = Some(scene);
        let success = BuildSuccess {
            workspace_summary: Some(workspace.summary()),
            requested_output: geometry.requested_output.clone(),
            geometry: DisplayedGeometry::from_evaluated(geometry),
            rebuild_millis,
        };
        self.accept_success(success.clone());
        AppRebuildAction::accepted(success.geometry)
    }

    pub fn frame_selected_bounds(&mut self, selection: &ViewportSelection) -> Option<Bounds> {
        let selected = selection.selected_node.as_ref()?;
        let scene = self.latest_scene.as_ref()?;
        self.evaluator
            .evaluate_node(scene, selected)
            .ok()
            .map(|geometry| geometry.bounds)
    }

    pub fn ui_status_snapshot(
        &self,
        geometry_revision: DisplayGeometryRevision,
        selection: &ViewportSelection,
    ) -> UiStatusSnapshot {
        let workspace_summary = self.workspace_summary();
        let current_output = self
            .displayed_geometry
            .as_ref()
            .map(|geometry| geometry.requested_output.to_string());
        let selection_label = selection.selected_node.as_ref().map(ToString::to_string);

        let (build_kind, build_label, error_message) = match &self.build_status {
            AppBuildStatus::NoWorkspace => (
                BuildStatusKind::NoWorkspace,
                "No workspace".to_owned(),
                None,
            ),
            AppBuildStatus::WorkspaceError(message) => (
                BuildStatusKind::WorkspaceError,
                "Workspace error".to_owned(),
                Some(message.clone()),
            ),
            AppBuildStatus::SceneError(message) => (
                BuildStatusKind::SceneError,
                "Scene error".to_owned(),
                Some(message.clone()),
            ),
            AppBuildStatus::GeometryError(message) => (
                BuildStatusKind::GeometryError,
                "Geometry error".to_owned(),
                Some(message.clone()),
            ),
            AppBuildStatus::Success(success) => {
                let kind = if success.geometry.mesh.is_empty() {
                    BuildStatusKind::EmptyMesh
                } else {
                    BuildStatusKind::Success
                };
                (kind, "Success".to_owned(), None)
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
            geometry_revision,
            build_kind,
            build_label,
            error_message,
            last_rebuild_millis: self.last_rebuild_millis,
            current_output,
            selection: selection_label,
        }
    }

    pub fn accept_success(&mut self, success: BuildSuccess) {
        self.displayed_geometry = Some(success.geometry.clone());
        self.build_status = AppBuildStatus::Success(Box::new(success));
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

    fn reload_or_open_workspace(&mut self) -> Result<(), WorkspaceBuildError> {
        if let Some(workspace) = self.workspace.as_mut() {
            workspace
                .reload_source()
                .map_err(WorkspaceBuildError::Workspace)?;
            return Ok(());
        }
        self.open_workspace_from_path()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppBuildStatus {
    NoWorkspace,
    WorkspaceError(String),
    SceneError(String),
    GeometryError(String),
    Success(Box<BuildSuccess>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildSuccess {
    pub workspace_summary: Option<WorkspaceSummary>,
    pub requested_output: NodeId,
    pub geometry: DisplayedGeometry,
    pub rebuild_millis: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayedGeometry {
    pub requested_output: NodeId,
    pub geometry_revision: u64,
    pub mesh: MorphosMesh,
    pub bounds: Bounds,
}

impl DisplayedGeometry {
    pub fn from_evaluated(geometry: EvaluatedGeometry) -> Self {
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

#[derive(Debug, Clone, PartialEq)]
pub struct AppRebuildAction {
    accepted_geometry: Option<DisplayedGeometry>,
}

impl AppRebuildAction {
    pub fn none() -> Self {
        Self {
            accepted_geometry: None,
        }
    }

    pub fn accepted(geometry: DisplayedGeometry) -> Self {
        Self {
            accepted_geometry: Some(geometry),
        }
    }

    pub fn accepted_geometry(&self) -> Option<&DisplayedGeometry> {
        self.accepted_geometry.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStatusKind {
    NoWorkspace,
    WorkspaceError,
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
    pub geometry_revision: DisplayGeometryRevision,
    pub build_kind: BuildStatusKind,
    pub build_label: String,
    pub error_message: Option<String>,
    pub last_rebuild_millis: Option<f64>,
    pub current_output: Option<String>,
    pub selection: Option<String>,
}

#[derive(Debug)]
enum WorkspaceBuildError {
    NoWorkspacePath,
    Workspace(WorkspaceError),
}
