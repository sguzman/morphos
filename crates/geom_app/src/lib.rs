pub mod camera;
pub mod mesh_adapter;
pub mod model;
pub mod reactive;
pub mod scene_tree;
pub mod viewport;

use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::pbr::wireframe::{Wireframe, WireframePlugin};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use camera::{CameraFrame, OrbitCameraInputMap, OrbitCameraState};
use crossbeam_channel::{Receiver, Sender, unbounded};
use geom_geometry::Bounds;
use geom_scene::{Axis, NodeId, ParamId, PrimitiveScalarField, SceneNodeDraft, TransformProperty};
use mesh_adapter::adapt_morphos_mesh;
use model::{
    AppEditError, AppModel, BuildStatusKind, DeleteNodeSafety, DisplayedGeometry, UiStatusSnapshot,
    ViewportDisplayMode, ViewportSelection, WorkspaceBuildError,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use reactive::{BuildOutcome, BuildWorker, EditOrigin, WorkerCommand, WorkspaceSessionId};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;
use viewport::DisplayGeometryRevision;

/// Current Bevy version selected for M05.
pub const GEOM_APP_BEVY_VERSION: &str = "0.19.1";

/// Current bevy_egui version selected for M05.
pub const GEOM_APP_BEVY_EGUI_VERSION: &str = "0.41.1";

const GRID_EXTENT: i32 = 10;
const GRID_SPACING: f32 = 1.0;
const GEOMETRY_MATERIAL_COLOR: Color = Color::srgb(0.78, 0.82, 0.9);
const DEMO_PARAMETER_ID: &str = "arm_length";

/// Parses an optional workspace path from command-line arguments.
pub fn parse_workspace_path_from_args<I>(args: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let _program = args.next();
    args.next().map(PathBuf::from)
}

/// Builds the desktop Morphos viewport application.
pub fn build_app(workspace_path: Option<PathBuf>) -> App {
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.08)))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 450.0,
            ..default()
        })
        .insert_resource(AppModel::new(workspace_path))
        .insert_resource(ViewportRuntimeState::default())
        .insert_resource(UiPointerCapture::default())
        .insert_resource(SceneEditorUiState::default())
        .insert_resource(ReactiveRuntimeState::default())
        .add_message::<AppCommand>()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Morphos".to_owned(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_plugins(WireframePlugin::default())
        .add_systems(Startup, (setup_scene, startup_build_request))
        .add_systems(EguiPrimaryContextPass, ui_system)
        .add_systems(
            Update,
            (
                camera_input_system,
                synchronize_camera_transform_system,
                process_app_commands_system,
                poll_watcher_events_system,
                poll_build_results_system,
                apply_geometry_refresh_system,
                synchronize_display_mode_system,
                draw_viewport_gizmos_system,
            ),
        );
    app
}

/// Runs the desktop Morphos viewport shell.
pub fn run_app(workspace_path: Option<PathBuf>) {
    build_app(workspace_path).run();
}

#[derive(Debug, Clone, Message)]
#[allow(dead_code)]
enum AppCommand {
    Rebuild,
    Reopen,
    Undo,
    Redo,
    FrameAll,
    FrameSelected,
    SetDisplayMode(ViewportDisplayMode),
    SetMeshVisibility(bool),
    SelectNode(Option<NodeId>),
    AdjustParameterScalar(ParamId, f64),
    SetParameterScalar(ParamId, f64),
    SetTransformComponent(NodeId, TransformProperty, Axis, f64),
    SetPrimitiveScalar(NodeId, PrimitiveScalarField, f64),
    SetNodeLabel(NodeId, Option<String>),
    RenameNode(NodeId, NodeId),
    DuplicateNode(NodeId, NodeId),
    DeleteNode(NodeId),
    SetRootNode(NodeId),
    AddNode(NodeId, SceneNodeDraft),
    SetCompositionChildren(NodeId, Vec<NodeId>),
}

#[derive(Debug, Default, Resource)]
struct UiPointerCapture {
    wants_pointer_input: bool,
}

#[derive(Debug, Resource)]
struct ViewportRuntimeState {
    camera: OrbitCameraState,
    selection: ViewportSelection,
    display_mode: ViewportDisplayMode,
    displayed_geometry_revision: DisplayGeometryRevision,
    displayed_output: Option<String>,
    displayed_bounds: Option<Bounds>,
    mesh_visible: bool,
    render_entity: Option<Entity>,
    mesh_handle: Option<Handle<Mesh>>,
    material_handle: Option<Handle<StandardMaterial>>,
}

impl Default for ViewportRuntimeState {
    fn default() -> Self {
        Self {
            camera: OrbitCameraState::default(),
            selection: ViewportSelection::default(),
            display_mode: ViewportDisplayMode::Shaded,
            displayed_geometry_revision: DisplayGeometryRevision::ZERO,
            displayed_output: None,
            displayed_bounds: None,
            mesh_visible: true,
            render_entity: None,
            mesh_handle: None,
            material_handle: None,
        }
    }
}

impl ViewportRuntimeState {
    fn accept_displayed_geometry(&mut self, geometry: &DisplayedGeometry) {
        self.displayed_geometry_revision = DisplayGeometryRevision::new(geometry.geometry_revision);
        self.displayed_output = Some(geometry.requested_output.to_string());
        self.displayed_bounds = Some(geometry.bounds.clone());
    }

    fn frame_all(&mut self, aspect_ratio: f32) {
        if let Some(bounds) = &self.displayed_bounds {
            self.camera.frame_bounds(bounds, aspect_ratio);
        }
    }
}

#[derive(Default, Resource)]
struct ReactiveRuntimeState {
    watcher: Option<WatcherSession>,
    worker: Option<BuildWorkerSession>,
    pending_mesh_upload_requested_at: Option<Instant>,
}

struct WatcherSession {
    session_id: WorkspaceSessionId,
    scene_path: PathBuf,
    receiver: Receiver<WatchedSourceEvent>,
    _watcher: RecommendedWatcher,
}

struct BuildWorkerSession {
    sender: Sender<WorkerCommand>,
    receiver: Receiver<BuildOutcome>,
}

#[derive(Debug, Clone)]
struct WatchedSourceEvent {
    session_id: WorkspaceSessionId,
    paths: Vec<PathBuf>,
    observed_at: Instant,
}

#[derive(Debug, Component)]
struct MorphosCamera;

#[derive(Debug, Component)]
struct MorphosGeometryEntity;

#[derive(Debug, Resource)]
#[allow(dead_code)]
struct SceneEditorUiState {
    search_query: String,
    label_buffer: String,
    rename_buffer: String,
    duplicate_buffer: String,
    add_node_id: String,
    add_kind: AddNodeKind,
    selected_node_cache: Option<NodeId>,
}

impl Default for SceneEditorUiState {
    fn default() -> Self {
        Self {
            search_query: String::new(),
            label_buffer: String::new(),
            rename_buffer: String::new(),
            duplicate_buffer: String::new(),
            add_node_id: "new_node".to_owned(),
            add_kind: AddNodeKind::Box,
            selected_node_cache: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum AddNodeKind {
    Box,
    Sphere,
    Cylinder,
    Capsule,
    Plane,
    Profile,
}

impl AddNodeKind {
    fn all() -> [Self; 6] {
        [
            Self::Box,
            Self::Sphere,
            Self::Cylinder,
            Self::Capsule,
            Self::Plane,
            Self::Profile,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Sphere => "sphere",
            Self::Cylinder => "cylinder",
            Self::Capsule => "capsule",
            Self::Plane => "plane",
            Self::Profile => "profile",
        }
    }

    fn draft(self) -> SceneNodeDraft {
        match self {
            Self::Box => SceneNodeDraft::Box,
            Self::Sphere => SceneNodeDraft::Sphere,
            Self::Cylinder => SceneNodeDraft::Cylinder,
            Self::Capsule => SceneNodeDraft::Capsule,
            Self::Plane => SceneNodeDraft::Plane,
            Self::Profile => SceneNodeDraft::Profile,
        }
    }
}

fn setup_scene(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Msaa::Sample4,
        Transform::from_xyz(-7.0, 6.0, 9.0).looking_at(Vec3::ZERO, Vec3::Y),
        MorphosCamera,
        Name::new("Morphos Camera"),
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 25_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.6, 0.0)),
        Name::new("Sun Light"),
    ));
}

fn startup_build_request(mut commands: MessageWriter<AppCommand>) {
    commands.write(AppCommand::Reopen);
}

fn ui_system(
    mut contexts: EguiContexts,
    app_model: Res<AppModel>,
    viewport: Res<ViewportRuntimeState>,
    mut editor_state: ResMut<SceneEditorUiState>,
    mut commands: MessageWriter<AppCommand>,
    mut pointer_capture: ResMut<UiPointerCapture>,
) -> Result {
    let context = contexts.ctx_mut()?;
    let status = build_ui_snapshot(&app_model, &viewport);
    synchronize_editor_buffers(&app_model, &viewport, &mut editor_state);

    egui::Window::new("Morphos Status")
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(12.0, 12.0))
        .collapsible(false)
        .resizable(false)
        .default_width(980.0)
        .show(context, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "Workspace: {}",
                    status
                        .workspace_name
                        .as_deref()
                        .unwrap_or("No workspace opened")
                ));
                if let Some(path) = status.workspace_path.as_deref() {
                    ui.label(path.display().to_string());
                }
                ui.separator();
                ui.label(format!(
                    "Workspace Rev: {}",
                    status.workspace_revision.get()
                ));
                ui.separator();
                ui.label(if status.workspace_dirty {
                    "Dirty"
                } else {
                    "Clean"
                });
                ui.separator();
                ui.label(format!(
                    "Watching: {}",
                    if status.watching { "yes" } else { "no" }
                ));
                ui.separator();
                ui.label(format!("Source Rev: {}", status.source_revision.get()));
                ui.separator();
                ui.label(format!("Build Gen: {}", status.build_generation.get()));
                ui.separator();
                ui.label(format!("Geometry Rev: {}", status.geometry_revision.get()));
                ui.separator();
                ui.label(format!("Build: {}", status.build_label));
                if status.build_in_progress {
                    ui.separator();
                    ui.label("Rebuilding");
                }
                if let Some(generation) = status.current_error_generation {
                    ui.separator();
                    ui.label(format!("Error Gen: {}", generation.get()));
                }
                if let Some(output) = status.current_output.as_deref() {
                    ui.separator();
                    ui.label(format!("Output: {output}"));
                }
                if let Some(selection) = status.selection.as_deref() {
                    ui.separator();
                    ui.label(format!("Selection: {selection}"));
                }
            });

            if let Some(timings) = status.timings {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Parse: {:.2} ms", timings.parse_millis));
                    ui.separator();
                    ui.label(format!("Eval: {:.2} ms", timings.evaluation_millis));
                    ui.separator();
                    ui.label(format!("Mesh: {:.2} ms", timings.mesh_upload_millis));
                    ui.separator();
                    ui.label(format!("Total: {:.2} ms", timings.total_millis));
                });
            }

            ui.horizontal_wrapped(|ui| {
                if ui.button("Reload / Rebuild").clicked() {
                    commands.write(AppCommand::Rebuild);
                }
                if ui.button("Reopen Current Workspace").clicked() {
                    commands.write(AppCommand::Reopen);
                }
                let availability = app_model.undo_redo_availability();
                if ui
                    .add_enabled(availability.can_undo, egui::Button::new("Undo"))
                    .clicked()
                {
                    commands.write(AppCommand::Undo);
                }
                if ui
                    .add_enabled(availability.can_redo, egui::Button::new("Redo"))
                    .clicked()
                {
                    commands.write(AppCommand::Redo);
                }
                if ui.button("Frame All").clicked() {
                    commands.write(AppCommand::FrameAll);
                }
                if ui.button("Frame Selected").clicked() {
                    commands.write(AppCommand::FrameSelected);
                }

                if let Some(value) = app_model.parameter_scalar(DEMO_PARAMETER_ID) {
                    ui.separator();
                    ui.label(format!("{DEMO_PARAMETER_ID}: {value:.2}"));
                    if ui.small_button("-0.25").clicked()
                        && let Ok(parameter) = ParamId::new(DEMO_PARAMETER_ID)
                    {
                        commands.write(AppCommand::AdjustParameterScalar(parameter, -0.25));
                    }
                    if ui.small_button("+0.25").clicked()
                        && let Ok(parameter) = ParamId::new(DEMO_PARAMETER_ID)
                    {
                        commands.write(AppCommand::AdjustParameterScalar(parameter, 0.25));
                    }
                }

                ui.separator();
                let shaded_selected = viewport.display_mode == ViewportDisplayMode::Shaded;
                if ui.selectable_label(shaded_selected, "Shaded").clicked() {
                    commands.write(AppCommand::SetDisplayMode(ViewportDisplayMode::Shaded));
                }
                let wireframe_selected = viewport.display_mode == ViewportDisplayMode::Wireframe;
                if ui
                    .selectable_label(wireframe_selected, "Wireframe")
                    .clicked()
                {
                    commands.write(AppCommand::SetDisplayMode(ViewportDisplayMode::Wireframe));
                }

                let mut mesh_visible = viewport.mesh_visible;
                if ui.checkbox(&mut mesh_visible, "Preview Visible").changed() {
                    commands.write(AppCommand::SetMeshVisibility(mesh_visible));
                }
            });
        });

    egui::Window::new("Scene Tree")
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(12.0, 116.0))
        .default_width(300.0)
        .default_height(620.0)
        .show(context, |ui| {
            render_scene_tree_panel(ui, &app_model, &viewport, &mut editor_state, &mut commands);
        });

    egui::Window::new("Inspector")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 116.0))
        .default_width(380.0)
        .default_height(680.0)
        .show(context, |ui| {
            render_inspector_panel(ui, &app_model, &viewport, &mut editor_state, &mut commands);
        });

    let overlay_message = match status.build_kind {
        BuildStatusKind::NoWorkspace => {
            Some("No workspace opened. Launch Morphos with a workspace path.")
        }
        BuildStatusKind::WorkspaceError
        | BuildStatusKind::Conflict
        | BuildStatusKind::SceneError
        | BuildStatusKind::GeometryError => status.error_message.as_deref(),
        BuildStatusKind::EmptyMesh => Some("Geometry evaluated to an empty mesh."),
        BuildStatusKind::Success => None,
    };

    if let Some(message) = overlay_message {
        egui::Area::new("viewport_overlay".into())
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                egui::Frame::window(ui.style())
                    .fill(egui::Color32::from_rgba_unmultiplied(18, 20, 26, 220))
                    .show(ui, |ui| {
                        ui.label(message);
                    });
            });
    }

    pointer_capture.wants_pointer_input = context.egui_wants_pointer_input();
    Ok(())
}

fn synchronize_editor_buffers(
    app_model: &AppModel,
    viewport: &ViewportRuntimeState,
    editor_state: &mut SceneEditorUiState,
) {
    let selected = viewport.selection.selected_node.clone();
    if editor_state.selected_node_cache == selected {
        return;
    }

    editor_state.selected_node_cache = selected.clone();
    if let Some(scene) = app_model.current_scene()
        && let Some(selected) = selected.as_ref()
        && let Some(node) = scene.nodes().get(selected)
    {
        editor_state.label_buffer = node.label().unwrap_or_default().to_owned();
        editor_state.rename_buffer = node.id().as_str().to_owned();
        editor_state.duplicate_buffer = next_suffixed_id(scene, node.id(), "_copy");
    } else {
        editor_state.label_buffer.clear();
        editor_state.rename_buffer.clear();
        editor_state.duplicate_buffer.clear();
    }
}

fn render_scene_tree_panel(
    ui: &mut egui::Ui,
    app_model: &AppModel,
    viewport: &ViewportRuntimeState,
    editor_state: &mut SceneEditorUiState,
    commands: &mut MessageWriter<AppCommand>,
) {
    ui.heading("Scene Tree");
    ui.add(
        egui::TextEdit::singleline(&mut editor_state.search_query)
            .hint_text("Search by NodeId or label"),
    );
    ui.separator();

    let Some(scene) = app_model.current_scene() else {
        ui.label("No scene loaded.");
        return;
    };
    let tree = app_model
        .scene_tree_model()
        .expect("tree exists when current scene exists");
    let matches = tree.filtered_matches(&editor_state.search_query);

    egui::ScrollArea::vertical().show(ui, |ui| {
        for root in tree.roots() {
            render_tree_entry(ui, &tree, root, &matches, viewport, commands);
        }

        if !tree.unreferenced().is_empty() {
            ui.separator();
            egui::CollapsingHeader::new("Unreferenced Nodes")
                .default_open(true)
                .show(ui, |ui| {
                    for node in tree.unreferenced() {
                        render_tree_entry(ui, &tree, node, &matches, viewport, commands);
                    }
                });
        }

        ui.separator();
        ui.heading("Add Primitive");
        ui.horizontal(|ui| {
            ui.label("Id");
            ui.text_edit_singleline(&mut editor_state.add_node_id);
        });
        egui::ComboBox::from_label("Kind")
            .selected_text(editor_state.add_kind.label())
            .show_ui(ui, |ui| {
                for kind in AddNodeKind::all() {
                    ui.selectable_value(&mut editor_state.add_kind, kind, kind.label());
                }
            });
        if ui.button("Add Unreferenced Node").clicked()
            && let Ok(node_id) = NodeId::new(editor_state.add_node_id.trim())
        {
            commands.write(AppCommand::AddNode(node_id, editor_state.add_kind.draft()));
        }

        if scene.nodes().contains_key(scene.root()) {
            ui.small(format!("Root output: {}", scene.root()));
        }
    });
}

fn render_tree_entry(
    ui: &mut egui::Ui,
    tree: &scene_tree::SceneTreeModel,
    node_id: &NodeId,
    matches: &std::collections::BTreeSet<NodeId>,
    viewport: &ViewportRuntimeState,
    commands: &mut MessageWriter<AppCommand>,
) {
    if !matches.contains(node_id) {
        let Some(entry) = tree.entry(node_id) else {
            return;
        };
        if !entry
            .dependency_ids
            .iter()
            .any(|child| matches.contains(child))
        {
            return;
        }
    }

    let Some(entry) = tree.entry(node_id) else {
        return;
    };
    let mut title = entry.node_id.to_string();
    if let Some(label) = &entry.label {
        let _ = write!(title, " ({label})");
    }
    let _ = write!(title, " [{}]", entry.kind_label);
    if entry.is_root {
        title.push_str(" root");
    }
    if entry.is_shared() {
        title.push_str(" shared");
    }

    let selected = viewport.selection.selected_node.as_ref() == Some(node_id);
    if entry.dependency_ids.is_empty() {
        if ui.selectable_label(selected, title).clicked() {
            commands.write(AppCommand::SelectNode(Some(node_id.clone())));
        }
        return;
    }

    egui::CollapsingHeader::new(title)
        .default_open(entry.is_root)
        .show(ui, |ui| {
            if ui
                .selectable_label(selected, format!("Select {}", entry.node_id))
                .clicked()
            {
                commands.write(AppCommand::SelectNode(Some(node_id.clone())));
            }
            for child in &entry.dependency_ids {
                render_tree_entry(ui, tree, child, matches, viewport, commands);
            }
        });
}

fn render_inspector_panel(
    ui: &mut egui::Ui,
    app_model: &AppModel,
    viewport: &ViewportRuntimeState,
    editor_state: &mut SceneEditorUiState,
    commands: &mut MessageWriter<AppCommand>,
) {
    ui.heading("Inspector");
    let disabled_reason = app_model.editing_disabled_reason().map(str::to_owned);
    if let Some(reason) = disabled_reason.as_deref() {
        ui.colored_label(egui::Color32::LIGHT_YELLOW, reason);
        ui.separator();
    }

    let Some(scene) = app_model.current_scene() else {
        ui.label("No scene loaded.");
        return;
    };
    let Some(selected) = viewport.selection.selected_node.as_ref() else {
        ui.label("Select a node from the scene tree.");
        render_parameter_panel(ui, app_model, disabled_reason.is_none(), commands);
        return;
    };
    let Some(node) = scene.nodes().get(selected) else {
        ui.label("Selected node is no longer present.");
        render_parameter_panel(ui, app_model, disabled_reason.is_none(), commands);
        return;
    };

    ui.label(format!("NodeId: {}", node.id()));
    ui.label(format!("Kind: {}", node_kind_label(node.kind())));
    ui.label(if scene.root() == node.id() {
        "Root Output: yes"
    } else {
        "Root Output: no"
    });
    if let Some(location) = app_model.node_source_location(node.id()) {
        ui.label(format!(
            "Source: source/scene.toml line {}, column {}",
            location.line, location.column
        ));
        if ui.button("Copy Source Location").clicked() {
            ui.ctx().copy_text(format!(
                "source/scene.toml:{}:{}",
                location.line, location.column
            ));
        }
    }
    ui.separator();

    let editing_enabled = disabled_reason.is_none();
    ui.horizontal(|ui| {
        ui.label("Label");
        ui.add_enabled(
            editing_enabled,
            egui::TextEdit::singleline(&mut editor_state.label_buffer),
        );
        if ui
            .add_enabled(editing_enabled, egui::Button::new("Apply"))
            .clicked()
        {
            let label = editor_state.label_buffer.trim();
            commands.write(AppCommand::SetNodeLabel(
                node.id().clone(),
                if label.is_empty() {
                    None
                } else {
                    Some(label.to_owned())
                },
            ));
        }
    });

    render_transform_editor(ui, node.id(), node.transform(), editing_enabled, commands);
    ui.separator();
    render_kind_specific_editor(ui, node, editing_enabled, scene, commands);
    ui.separator();
    render_structural_editor(
        ui,
        app_model,
        node.id(),
        editing_enabled,
        editor_state,
        commands,
    );
    ui.separator();
    render_parameter_panel(ui, app_model, editing_enabled, commands);
}

fn render_transform_editor(
    ui: &mut egui::Ui,
    node_id: &NodeId,
    transform: &geom_scene::Transform,
    editing_enabled: bool,
    commands: &mut MessageWriter<AppCommand>,
) {
    ui.heading("Transform");
    render_scalar_expr_triplet(
        ui,
        node_id,
        TransformProperty::Translation,
        "Translate",
        &transform.translation,
        editing_enabled,
        commands,
    );
    render_scalar_expr_triplet(
        ui,
        node_id,
        TransformProperty::RotationDegrees,
        "Rotate Deg",
        &transform.rotation_deg,
        editing_enabled,
        commands,
    );
    render_scalar_expr_triplet(
        ui,
        node_id,
        TransformProperty::Scale,
        "Scale",
        &transform.scale,
        editing_enabled,
        commands,
    );
}

fn render_scalar_expr_triplet(
    ui: &mut egui::Ui,
    node_id: &NodeId,
    property: TransformProperty,
    label: &str,
    vector: &geom_scene::Vector3Expr,
    editing_enabled: bool,
    commands: &mut MessageWriter<AppCommand>,
) {
    ui.label(label);
    ui.horizontal(|ui| {
        render_scalar_expr_component(
            ui,
            node_id,
            property,
            Axis::X,
            "X",
            &vector.x,
            editing_enabled,
            commands,
        );
        render_scalar_expr_component(
            ui,
            node_id,
            property,
            Axis::Y,
            "Y",
            &vector.y,
            editing_enabled,
            commands,
        );
        render_scalar_expr_component(
            ui,
            node_id,
            property,
            Axis::Z,
            "Z",
            &vector.z,
            editing_enabled,
            commands,
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn render_scalar_expr_component(
    ui: &mut egui::Ui,
    node_id: &NodeId,
    property: TransformProperty,
    axis: Axis,
    axis_label: &str,
    expr: &geom_scene::ScalarExpr,
    editing_enabled: bool,
    commands: &mut MessageWriter<AppCommand>,
) {
    ui.vertical(|ui| {
        ui.label(axis_label);
        match expr {
            geom_scene::ScalarExpr::Literal(value) => {
                let mut pending = *value;
                let response = ui.add_enabled(
                    editing_enabled,
                    egui::DragValue::new(&mut pending)
                        .speed(0.1)
                        .range(-10_000.0..=10_000.0),
                );
                if response.changed() {
                    commands.write(AppCommand::SetTransformComponent(
                        node_id.clone(),
                        property,
                        axis,
                        pending,
                    ));
                }
            }
            geom_scene::ScalarExpr::Parameter(parameter) => {
                ui.label(format!("param: {}", parameter.target()));
            }
        }
    });
}

fn render_kind_specific_editor(
    ui: &mut egui::Ui,
    node: &geom_scene::Node,
    editing_enabled: bool,
    scene: &geom_scene::SceneDocument,
    commands: &mut MessageWriter<AppCommand>,
) {
    ui.heading("Node Properties");
    match node.kind() {
        geom_scene::NodeKind::Box(primitive) => {
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::BoxX,
                "Size X",
                &primitive.size.x,
                editing_enabled,
                commands,
            );
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::BoxY,
                "Size Y",
                &primitive.size.y,
                editing_enabled,
                commands,
            );
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::BoxZ,
                "Size Z",
                &primitive.size.z,
                editing_enabled,
                commands,
            );
        }
        geom_scene::NodeKind::Sphere(primitive) => render_literal_or_param_scalar(
            ui,
            node.id(),
            PrimitiveScalarField::SphereRadius,
            "Radius",
            &primitive.radius,
            editing_enabled,
            commands,
        ),
        geom_scene::NodeKind::Cylinder(primitive) => {
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::CylinderRadius,
                "Radius",
                &primitive.radius,
                editing_enabled,
                commands,
            );
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::CylinderHeight,
                "Height",
                &primitive.height,
                editing_enabled,
                commands,
            );
        }
        geom_scene::NodeKind::Capsule(primitive) => {
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::CapsuleRadius,
                "Radius",
                &primitive.radius,
                editing_enabled,
                commands,
            );
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::CapsuleHeight,
                "Height",
                &primitive.height,
                editing_enabled,
                commands,
            );
        }
        geom_scene::NodeKind::Plane(primitive) => {
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::PlaneWidth,
                "Width",
                &primitive.width,
                editing_enabled,
                commands,
            );
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::PlaneDepth,
                "Depth",
                &primitive.depth,
                editing_enabled,
                commands,
            );
        }
        geom_scene::NodeKind::Profile(primitive) => {
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::ProfileWidth,
                "Width",
                &primitive.width,
                editing_enabled,
                commands,
            );
            render_literal_or_param_scalar(
                ui,
                node.id(),
                PrimitiveScalarField::ProfileHeight,
                "Height",
                &primitive.height,
                editing_enabled,
                commands,
            );
        }
        geom_scene::NodeKind::Union(composition)
        | geom_scene::NodeKind::Difference(composition)
        | geom_scene::NodeKind::Intersection(composition) => {
            ui.label("Ordered children");
            let node_ids = scene.nodes().keys().cloned().collect::<Vec<_>>();
            let current_children = composition
                .children
                .iter()
                .map(|child| child.target().clone())
                .collect::<Vec<_>>();
            for (index, child) in current_children.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}", index + 1));
                    if ui.button("Up").clicked() && index > 0 && editing_enabled {
                        let mut reordered = current_children.clone();
                        reordered.swap(index, index - 1);
                        commands.write(AppCommand::SetCompositionChildren(
                            node.id().clone(),
                            reordered,
                        ));
                    }
                    if ui.button("Down").clicked()
                        && index + 1 < current_children.len()
                        && editing_enabled
                    {
                        let mut reordered = current_children.clone();
                        reordered.swap(index, index + 1);
                        commands.write(AppCommand::SetCompositionChildren(
                            node.id().clone(),
                            reordered,
                        ));
                    }
                    let mut replacement = child.clone();
                    egui::ComboBox::from_id_salt(("child", node.id().as_str(), index))
                        .selected_text(replacement.as_str())
                        .show_ui(ui, |ui| {
                            for candidate in &node_ids {
                                if candidate != node.id() {
                                    ui.selectable_value(
                                        &mut replacement,
                                        candidate.clone(),
                                        candidate.as_str(),
                                    );
                                }
                            }
                        });
                    if replacement != *child && editing_enabled {
                        let mut replaced = current_children.clone();
                        replaced[index] = replacement;
                        commands.write(AppCommand::SetCompositionChildren(
                            node.id().clone(),
                            replaced,
                        ));
                    }
                    if ui.button("Remove").clicked()
                        && editing_enabled
                        && current_children.len() > 2
                    {
                        let mut removed = current_children.clone();
                        removed.remove(index);
                        commands.write(AppCommand::SetCompositionChildren(
                            node.id().clone(),
                            removed,
                        ));
                    }
                });
            }

            if editing_enabled {
                ui.horizontal(|ui| {
                    ui.label("Add child");
                    let mut candidate = node_ids
                        .iter()
                        .find(|candidate| *candidate != node.id())
                        .cloned()
                        .unwrap_or_else(|| node.id().clone());
                    egui::ComboBox::from_id_salt(("add_child", node.id().as_str()))
                        .selected_text(candidate.as_str())
                        .show_ui(ui, |ui| {
                            for node_id in &node_ids {
                                if node_id != node.id() {
                                    ui.selectable_value(
                                        &mut candidate,
                                        node_id.clone(),
                                        node_id.as_str(),
                                    );
                                }
                            }
                        });
                    if ui.button("Append").clicked() && candidate != *node.id() {
                        let mut appended = current_children.clone();
                        appended.push(candidate);
                        commands.write(AppCommand::SetCompositionChildren(
                            node.id().clone(),
                            appended,
                        ));
                    }
                });
            }
        }
    }
}

fn render_literal_or_param_scalar(
    ui: &mut egui::Ui,
    node_id: &NodeId,
    field: PrimitiveScalarField,
    label: &str,
    expr: &geom_scene::ScalarExpr,
    editing_enabled: bool,
    commands: &mut MessageWriter<AppCommand>,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        match expr {
            geom_scene::ScalarExpr::Literal(value) => {
                let mut pending = *value;
                let response = ui.add_enabled(
                    editing_enabled,
                    egui::DragValue::new(&mut pending)
                        .speed(0.1)
                        .range(0.0001..=10_000.0),
                );
                if response.changed() {
                    commands.write(AppCommand::SetPrimitiveScalar(
                        node_id.clone(),
                        field,
                        pending,
                    ));
                }
            }
            geom_scene::ScalarExpr::Parameter(parameter) => {
                ui.label(format!("parameter-driven: {}", parameter.target()));
            }
        }
    });
}

fn render_structural_editor(
    ui: &mut egui::Ui,
    app_model: &AppModel,
    node_id: &NodeId,
    editing_enabled: bool,
    editor_state: &mut SceneEditorUiState,
    commands: &mut MessageWriter<AppCommand>,
) {
    ui.heading("Structure");
    ui.horizontal(|ui| {
        ui.label("Rename");
        ui.add_enabled(
            editing_enabled,
            egui::TextEdit::singleline(&mut editor_state.rename_buffer),
        );
        if ui
            .add_enabled(editing_enabled, egui::Button::new("Apply"))
            .clicked()
            && let Ok(next) = NodeId::new(editor_state.rename_buffer.trim())
        {
            commands.write(AppCommand::RenameNode(node_id.clone(), next));
        }
    });
    ui.horizontal(|ui| {
        ui.label("Duplicate As");
        ui.add_enabled(
            editing_enabled,
            egui::TextEdit::singleline(&mut editor_state.duplicate_buffer),
        );
        if ui
            .add_enabled(editing_enabled, egui::Button::new("Duplicate"))
            .clicked()
            && let Ok(next) = NodeId::new(editor_state.duplicate_buffer.trim())
        {
            commands.write(AppCommand::DuplicateNode(node_id.clone(), next));
        }
    });
    if ui
        .add_enabled(editing_enabled, egui::Button::new("Set As Root Output"))
        .clicked()
    {
        commands.write(AppCommand::SetRootNode(node_id.clone()));
    }

    if let Some(safety) = app_model.delete_node_safety(node_id) {
        render_delete_safety(ui, &safety);
        let can_delete = editing_enabled && !safety.is_root && safety.direct_dependents.is_empty();
        if ui
            .add_enabled(can_delete, egui::Button::new("Delete Node"))
            .clicked()
        {
            commands.write(AppCommand::DeleteNode(node_id.clone()));
        }
    }
}

fn render_delete_safety(ui: &mut egui::Ui, safety: &DeleteNodeSafety) {
    if safety.is_root {
        ui.colored_label(
            egui::Color32::LIGHT_RED,
            "Deletion blocked: selected node is the root output.",
        );
    } else if !safety.direct_dependents.is_empty() {
        ui.colored_label(
            egui::Color32::LIGHT_RED,
            format!(
                "Deletion blocked: directly referenced by {}",
                join_node_ids(&safety.direct_dependents)
            ),
        );
    } else {
        ui.label("Deletion safe: node is currently unreferenced.");
    }
    if !safety.transitive_dependents.is_empty() {
        ui.small(format!(
            "Transitive dependents: {}",
            join_node_ids(&safety.transitive_dependents)
        ));
    }
}

fn render_parameter_panel(
    ui: &mut egui::Ui,
    app_model: &AppModel,
    editing_enabled: bool,
    commands: &mut MessageWriter<AppCommand>,
) {
    ui.heading("Parameters");
    let Some(scene) = app_model.current_scene() else {
        return;
    };
    let tree = app_model.scene_tree_model();
    for (parameter_id, definition) in scene.parameters() {
        ui.separator();
        ui.label(parameter_id.as_str());
        if let Some(location) = app_model.parameter_source_location(parameter_id) {
            ui.small(format!(
                "source/scene.toml:{}:{}",
                location.line, location.column
            ));
        }
        let mut value = definition.scalar_value();
        if ui
            .add_enabled(
                editing_enabled,
                egui::DragValue::new(&mut value)
                    .speed(0.1)
                    .range(-10_000.0..=10_000.0),
            )
            .changed()
        {
            commands.write(AppCommand::SetParameterScalar(parameter_id.clone(), value));
        }
        if let Some(units) = definition
            .extensions()
            .entries()
            .get("units")
            .and_then(|value| match value {
                geom_scene::ExtensionValue::String(text) => Some(text.as_str()),
                _ => None,
            })
        {
            ui.small(format!("Units: {units}"));
        }
        if let Some(tree) = &tree {
            let direct = tree.parameter_dependents(parameter_id);
            let transitive = tree.transitive_parameter_dependents(parameter_id);
            ui.small(format!("Direct dependents: {}", join_node_ids(&direct)));
            ui.small(format!(
                "Transitive dependents: {}",
                join_node_ids(&transitive)
            ));
        }
    }
}

fn join_node_ids(node_ids: &[NodeId]) -> String {
    if node_ids.is_empty() {
        "none".to_owned()
    } else {
        node_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn next_suffixed_id(scene: &geom_scene::SceneDocument, base: &NodeId, suffix: &str) -> String {
    let mut index = 1usize;
    loop {
        let candidate = format!("{}{suffix}{index}", base.as_str());
        if !scene
            .nodes()
            .keys()
            .any(|existing| existing.as_str() == candidate)
        {
            return candidate;
        }
        index += 1;
    }
}

fn node_kind_label(kind: &geom_scene::NodeKind) -> &'static str {
    match kind {
        geom_scene::NodeKind::Box(_) => "box",
        geom_scene::NodeKind::Sphere(_) => "sphere",
        geom_scene::NodeKind::Cylinder(_) => "cylinder",
        geom_scene::NodeKind::Capsule(_) => "capsule",
        geom_scene::NodeKind::Plane(_) => "plane",
        geom_scene::NodeKind::Profile(_) => "profile",
        geom_scene::NodeKind::Union(_) => "union",
        geom_scene::NodeKind::Difference(_) => "difference",
        geom_scene::NodeKind::Intersection(_) => "intersection",
    }
}

fn camera_input_system(
    windows: Single<&Window>,
    mut viewport: ResMut<ViewportRuntimeState>,
    pointer_capture: Res<UiPointerCapture>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut motion_reader: MessageReader<MouseMotion>,
    mut wheel_reader: MessageReader<MouseWheel>,
) {
    if pointer_capture.wants_pointer_input {
        motion_reader.clear();
        wheel_reader.clear();
        return;
    }

    let mut orbit_delta = Vec2::ZERO;
    let mut pan_delta = Vec2::ZERO;

    for event in motion_reader.read() {
        if buttons.pressed(OrbitCameraInputMap::default().orbit_button) {
            orbit_delta += event.delta;
        }
        if buttons.pressed(OrbitCameraInputMap::default().pan_button) {
            pan_delta += event.delta;
        }
    }

    if orbit_delta != Vec2::ZERO {
        viewport
            .camera
            .orbit(orbit_delta, OrbitCameraInputMap::default());
    }
    if pan_delta != Vec2::ZERO {
        viewport
            .camera
            .pan(pan_delta, OrbitCameraInputMap::default());
    }

    let scroll_delta: f32 = wheel_reader.read().map(|event| event.y).sum();
    if scroll_delta != 0.0 {
        viewport
            .camera
            .zoom(scroll_delta, OrbitCameraInputMap::default());
    }

    viewport.camera.aspect_ratio = windows.width() / windows.height().max(1.0);
}

fn synchronize_camera_transform_system(
    viewport: Res<ViewportRuntimeState>,
    mut camera_query: Single<&mut Transform, With<MorphosCamera>>,
) {
    **camera_query = viewport.camera.transform();
}

fn process_app_commands_system(
    mut commands: MessageReader<AppCommand>,
    windows: Single<&Window>,
    mut app_model: ResMut<AppModel>,
    mut runtime: ResMut<ReactiveRuntimeState>,
    mut viewport: ResMut<ViewportRuntimeState>,
) {
    for command in commands.read() {
        match command {
            AppCommand::Rebuild => match app_model.schedule_manual_rebuild(Instant::now()) {
                Ok(request) => dispatch_build_request(&mut runtime, request),
                Err(error) => apply_workspace_error(&mut app_model, error),
            },
            AppCommand::Reopen => match app_model.prepare_reopen(Instant::now()) {
                Ok(request) => {
                    if let Some(workspace) = app_model.workspace_mut()
                        && let Err(error) = configure_runtime_for_workspace(
                            &mut runtime,
                            workspace.paths().source_file(),
                            request.session_id,
                        )
                    {
                        error!("failed to configure watcher: {error}");
                    }
                    dispatch_build_request(&mut runtime, request);
                }
                Err(error) => apply_workspace_error(&mut app_model, error),
            },
            AppCommand::Undo => match app_model.undo(EditOrigin::Gui, Instant::now()) {
                Ok(Some(request)) => dispatch_build_request(&mut runtime, request),
                Ok(None) => {}
                Err(error) => warn!("undo failed: {error:?}"),
            },
            AppCommand::Redo => match app_model.redo(EditOrigin::Gui, Instant::now()) {
                Ok(Some(request)) => dispatch_build_request(&mut runtime, request),
                Ok(None) => {}
                Err(error) => warn!("redo failed: {error:?}"),
            },
            AppCommand::FrameAll => {
                viewport.frame_all(windows.width() / windows.height().max(1.0));
            }
            AppCommand::FrameSelected => {
                if let Some(bounds) = app_model.frame_selected_bounds(&viewport.selection) {
                    let aspect_ratio = viewport.camera.aspect_ratio;
                    viewport
                        .camera
                        .apply_frame(CameraFrame::from_bounds(&bounds, aspect_ratio));
                }
            }
            AppCommand::SetDisplayMode(mode) => {
                viewport.display_mode = *mode;
            }
            AppCommand::SetMeshVisibility(visible) => {
                viewport.mesh_visible = *visible;
            }
            AppCommand::SelectNode(node) => {
                viewport.selection.select(node.clone());
            }
            AppCommand::AdjustParameterScalar(parameter, delta) => match app_model
                .apply_parameter_scalar_delta(parameter, *delta, EditOrigin::Gui, Instant::now())
            {
                Ok(Some(request)) => dispatch_build_request(&mut runtime, request),
                Ok(None) => {}
                Err(AppEditError::Workspace(error)) => {
                    error!("workspace edit failed: {error}");
                }
                Err(AppEditError::Scene(error)) => {
                    error!("scene edit failed: {error}");
                }
                Err(AppEditError::Transaction(error)) => {
                    error!("transaction edit failed: {error}");
                }
                Err(AppEditError::Conflict(error)) => {
                    warn!("edit conflict: {error}");
                }
            },
            AppCommand::SetParameterScalar(parameter, value) => match app_model
                .set_parameter_scalar_value(parameter, *value, EditOrigin::Gui, Instant::now())
            {
                Ok(Some(request)) => dispatch_build_request(&mut runtime, request),
                Ok(None) => {}
                Err(error) => warn!("parameter edit failed: {error:?}"),
            },
            AppCommand::SetTransformComponent(node, property, axis, value) => match app_model
                .set_selected_node_transform_literal(
                    node,
                    *property,
                    *axis,
                    *value,
                    EditOrigin::Gui,
                    Instant::now(),
                ) {
                Ok(Some(request)) => dispatch_build_request(&mut runtime, request),
                Ok(None) => {}
                Err(error) => warn!("transform edit failed: {error:?}"),
            },
            AppCommand::SetPrimitiveScalar(node, field, value) => match app_model
                .set_selected_node_primitive_literal(
                    node,
                    *field,
                    *value,
                    EditOrigin::Gui,
                    Instant::now(),
                ) {
                Ok(Some(request)) => dispatch_build_request(&mut runtime, request),
                Ok(None) => {}
                Err(error) => warn!("primitive edit failed: {error:?}"),
            },
            AppCommand::SetNodeLabel(node, label) => match app_model.set_node_label(
                node,
                label.as_deref(),
                EditOrigin::Gui,
                Instant::now(),
            ) {
                Ok(Some(request)) => dispatch_build_request(&mut runtime, request),
                Ok(None) => {}
                Err(error) => warn!("label edit failed: {error:?}"),
            },
            AppCommand::RenameNode(from, to) => {
                match app_model.rename_node(from, to, EditOrigin::Gui, Instant::now()) {
                    Ok(Some(request)) => {
                        viewport.selection.select(Some(to.clone()));
                        dispatch_build_request(&mut runtime, request);
                    }
                    Ok(None) => {}
                    Err(error) => warn!("rename failed: {error:?}"),
                }
            }
            AppCommand::DuplicateNode(source, duplicate) => {
                match app_model.duplicate_node(source, duplicate, EditOrigin::Gui, Instant::now()) {
                    Ok(Some(request)) => {
                        viewport.selection.select(Some(duplicate.clone()));
                        dispatch_build_request(&mut runtime, request);
                    }
                    Ok(None) => {}
                    Err(error) => warn!("duplicate failed: {error:?}"),
                }
            }
            AppCommand::DeleteNode(node) => {
                match app_model.delete_node(node, EditOrigin::Gui, Instant::now()) {
                    Ok(Some(request)) => {
                        viewport.selection.select(None);
                        dispatch_build_request(&mut runtime, request);
                    }
                    Ok(None) => {}
                    Err(error) => warn!("delete failed: {error:?}"),
                }
            }
            AppCommand::SetRootNode(node) => {
                match app_model.set_root_node(node, EditOrigin::Gui, Instant::now()) {
                    Ok(Some(request)) => dispatch_build_request(&mut runtime, request),
                    Ok(None) => {}
                    Err(error) => warn!("set root failed: {error:?}"),
                }
            }
            AppCommand::AddNode(node, draft) => {
                match app_model.add_node(node, draft.clone(), EditOrigin::Gui, Instant::now()) {
                    Ok(Some(request)) => {
                        viewport.selection.select(Some(node.clone()));
                        dispatch_build_request(&mut runtime, request);
                    }
                    Ok(None) => {}
                    Err(error) => warn!("add node failed: {error:?}"),
                }
            }
            AppCommand::SetCompositionChildren(node, children) => match app_model
                .set_composition_children(node, children, EditOrigin::Gui, Instant::now())
            {
                Ok(Some(request)) => dispatch_build_request(&mut runtime, request),
                Ok(None) => {}
                Err(error) => warn!("composition edit failed: {error:?}"),
            },
        }
    }
}

fn poll_watcher_events_system(
    mut app_model: ResMut<AppModel>,
    mut runtime: ResMut<ReactiveRuntimeState>,
) {
    let mut saw_relevant_event = false;
    if let Some(watcher) = &runtime.watcher {
        while let Ok(event) = watcher.receiver.try_recv() {
            if event.session_id != watcher.session_id {
                continue;
            }
            if event.paths.iter().any(|path| path == &watcher.scene_path) {
                app_model.note_file_event(event.observed_at);
                saw_relevant_event = true;
            }
        }
    }

    if !saw_relevant_event && app_model.drain_ready_file_event(Instant::now()).is_none() {
        return;
    }

    if app_model.drain_ready_file_event(Instant::now()).is_some() {
        match app_model.accept_external_reload(Instant::now()) {
            Ok(Some(request)) => dispatch_build_request(&mut runtime, request),
            Ok(None) => {}
            Err(error) => apply_workspace_error(&mut app_model, error),
        }
    }
}

fn poll_build_results_system(
    mut app_model: ResMut<AppModel>,
    mut runtime: ResMut<ReactiveRuntimeState>,
    mut viewport: ResMut<ViewportRuntimeState>,
) {
    let Some(receiver) = runtime
        .worker
        .as_ref()
        .map(|worker| worker.receiver.clone())
    else {
        return;
    };

    while let Ok(outcome) = receiver.try_recv() {
        let action = app_model.accept_build_outcome(outcome);
        if action.refresh_displayed_geometry() {
            if let Some(displayed) = app_model.displayed_geometry() {
                viewport.accept_displayed_geometry(displayed);
            }
            app_model.preserve_selection(&mut viewport.selection);
            runtime.pending_mesh_upload_requested_at = action.requested_at();
        }
    }
}

fn apply_geometry_refresh_system(
    mut commands: Commands,
    mut app_model: ResMut<AppModel>,
    mut runtime: ResMut<ReactiveRuntimeState>,
    mut viewport: ResMut<ViewportRuntimeState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(displayed) = app_model.displayed_geometry() else {
        return;
    };
    let current_revision = DisplayGeometryRevision::new(displayed.geometry_revision);
    if viewport.displayed_geometry_revision == current_revision {
        return;
    }

    let upload_started = Instant::now();
    let bevy_mesh = match adapt_morphos_mesh(&displayed.mesh) {
        Ok(mesh) => mesh,
        Err(error) => {
            warn!("failed to adapt Morphos mesh for rendering: {error}");
            return;
        }
    };

    let mesh_handle = meshes.add(bevy_mesh);
    let material_handle = viewport.material_handle.clone().unwrap_or_else(|| {
        materials.add(StandardMaterial {
            base_color: GEOMETRY_MATERIAL_COLOR,
            perceptual_roughness: 0.68,
            metallic: 0.02,
            ..default()
        })
    });

    if let Some(entity) = viewport.render_entity {
        commands.entity(entity).insert((
            Mesh3d(mesh_handle.clone()),
            MeshMaterial3d(material_handle.clone()),
        ));
    } else {
        let entity = commands
            .spawn((
                Mesh3d(mesh_handle.clone()),
                MeshMaterial3d(material_handle.clone()),
                Transform::default(),
                Visibility::default(),
                MorphosGeometryEntity,
                Name::new("Morphos Evaluated Geometry"),
            ))
            .id();
        viewport.render_entity = Some(entity);
    }

    viewport.mesh_handle = Some(mesh_handle);
    viewport.material_handle = Some(material_handle);
    viewport.accept_displayed_geometry(displayed);

    if let Some(requested_at) = runtime.pending_mesh_upload_requested_at.take() {
        let upload_millis = Instant::now()
            .saturating_duration_since(upload_started)
            .as_secs_f64()
            * 1_000.0;
        let total_millis = Instant::now()
            .saturating_duration_since(requested_at)
            .as_secs_f64()
            * 1_000.0;
        app_model.note_mesh_upload_complete(upload_millis, total_millis);
    }
}

fn synchronize_display_mode_system(
    mut commands: Commands,
    viewport: Res<ViewportRuntimeState>,
    geometry_entities: Query<Entity, With<MorphosGeometryEntity>>,
) {
    if !viewport.is_changed() {
        return;
    }

    for entity in &geometry_entities {
        match viewport.display_mode {
            ViewportDisplayMode::Shaded => {
                commands.entity(entity).remove::<Wireframe>();
            }
            ViewportDisplayMode::Wireframe => {
                commands.entity(entity).insert(Wireframe);
            }
        }
        commands.entity(entity).insert(if viewport.mesh_visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
    }
}

fn draw_viewport_gizmos_system(mut gizmos: Gizmos) {
    for step in -GRID_EXTENT..=GRID_EXTENT {
        let offset = step as f32 * GRID_SPACING;
        let accent = if step == 0 { 0.35 } else { 0.16 };
        let color = Color::srgba(accent, accent, accent, 0.8);
        gizmos.line(
            Vec3::new(offset, 0.0, -GRID_EXTENT as f32 * GRID_SPACING),
            Vec3::new(offset, 0.0, GRID_EXTENT as f32 * GRID_SPACING),
            color,
        );
        gizmos.line(
            Vec3::new(-GRID_EXTENT as f32 * GRID_SPACING, 0.0, offset),
            Vec3::new(GRID_EXTENT as f32 * GRID_SPACING, 0.0, offset),
            color,
        );
    }

    gizmos.line(
        Vec3::ZERO,
        Vec3::new(2.5, 0.0, 0.0),
        Color::srgb(0.95, 0.25, 0.25),
    );
    gizmos.line(
        Vec3::ZERO,
        Vec3::new(0.0, 2.5, 0.0),
        Color::srgb(0.3, 0.95, 0.35),
    );
    gizmos.line(
        Vec3::ZERO,
        Vec3::new(0.0, 0.0, 2.5),
        Color::srgb(0.25, 0.45, 0.95),
    );
}

fn build_ui_snapshot(app_model: &AppModel, viewport: &ViewportRuntimeState) -> UiStatusSnapshot {
    app_model.ui_status_snapshot(viewport.displayed_geometry_revision, &viewport.selection)
}

fn configure_runtime_for_workspace(
    runtime: &mut ReactiveRuntimeState,
    scene_path: PathBuf,
    session_id: WorkspaceSessionId,
) -> notify::Result<()> {
    runtime.worker = Some(spawn_build_worker());
    runtime.watcher = Some(spawn_watcher(scene_path, session_id)?);
    Ok(())
}

fn dispatch_build_request(
    runtime: &mut ReactiveRuntimeState,
    request: reactive::BuildRequestSnapshot,
) {
    if runtime.worker.is_none() {
        runtime.worker = Some(spawn_build_worker());
    }
    if let Some(worker) = &runtime.worker {
        let _ = worker.sender.send(WorkerCommand::Build(request));
    }
}

fn apply_workspace_error(_app_model: &mut AppModel, error: WorkspaceBuildError) {
    match error {
        WorkspaceBuildError::NoWorkspacePath => {}
        WorkspaceBuildError::Workspace(error) => {
            error!("workspace operation failed: {error}");
        }
    }
}

fn spawn_build_worker() -> BuildWorkerSession {
    let (command_sender, command_receiver) = unbounded::<WorkerCommand>();
    let (result_sender, result_receiver) = unbounded::<BuildOutcome>();
    thread::spawn(move || {
        let mut worker = BuildWorker::new();
        while let Ok(command) = command_receiver.recv() {
            match command {
                WorkerCommand::Shutdown => break,
                WorkerCommand::Build(mut request) => {
                    while let Ok(next_command) = command_receiver.try_recv() {
                        match next_command {
                            WorkerCommand::Shutdown => return,
                            WorkerCommand::Build(next_request) => {
                                request = next_request;
                            }
                        }
                    }
                    let outcome = worker.process(request);
                    if result_sender.send(outcome).is_err() {
                        break;
                    }
                }
            }
        }
    });

    BuildWorkerSession {
        sender: command_sender,
        receiver: result_receiver,
    }
}

fn spawn_watcher(
    scene_path: PathBuf,
    session_id: WorkspaceSessionId,
) -> notify::Result<WatcherSession> {
    let source_dir = scene_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| scene_path.clone());
    let (sender, receiver) = unbounded::<WatchedSourceEvent>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event {
            let _ = sender.send(WatchedSourceEvent {
                session_id,
                paths: event.paths,
                observed_at: Instant::now(),
            });
        }
    })?;
    watcher.watch(&source_dir, RecursiveMode::NonRecursive)?;
    Ok(WatcherSession {
        session_id,
        scene_path,
        receiver,
        _watcher: watcher,
    })
}

/// Builds, parses, and evaluates a workspace path without a window.
pub fn smoke_load_workspace(path: impl AsRef<Path>) -> AppModel {
    let mut model = AppModel::new(Some(path.as_ref().to_path_buf()));
    let request = model.prepare_reopen(Instant::now()).expect("reopen");
    let mut worker = BuildWorker::new();
    let action = model.accept_build_outcome(worker.process(request));
    assert!(!action.refresh_displayed_geometry() || model.displayed_geometry().is_some());
    model
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppBuildStatus;
    use crate::reactive::EditOrigin;
    use crossbeam_channel::unbounded;
    use geom_geometry::Mesh as MorphosMesh;
    use geom_workspace::Revision;
    use std::fs;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn smoke_workspace_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("workspaces")
            .join("viewport-smoke")
    }

    fn benchmark_workspace_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("workspaces")
            .join("benchmark-reactive")
    }

    fn authoring_smoke_workspace_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("workspaces")
            .join("authoring-smoke")
    }

    fn clone_workspace_fixture() -> PathBuf {
        clone_workspace_from(&smoke_workspace_path())
    }

    fn clone_workspace_from(source_root: &Path) -> PathBuf {
        let unique_suffix = format!(
            "geom-app-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        let target_root = std::env::temp_dir().join(unique_suffix);
        copy_directory_recursive(source_root, &target_root);
        target_root
    }

    fn copy_directory_recursive(source: &Path, target: &Path) {
        fs::create_dir_all(target).expect("create target directory");
        for entry in fs::read_dir(source).expect("read source directory") {
            let entry = entry.expect("directory entry");
            let source_path = entry.path();
            let target_path = target.join(entry.file_name());
            if entry.file_type().expect("type").is_dir() {
                copy_directory_recursive(&source_path, &target_path);
            } else {
                fs::copy(&source_path, &target_path).expect("copy file");
            }
        }
    }

    #[test]
    fn parse_workspace_path_uses_first_argument_after_program() {
        let path = parse_workspace_path_from_args([
            OsString::from("geom_app"),
            OsString::from("examples/workspaces/viewport-smoke"),
        ]);
        assert_eq!(
            path,
            Some(PathBuf::from("examples/workspaces/viewport-smoke"))
        );
    }

    #[test]
    fn successful_build_surfaces_workspace_and_geometry_revisions() {
        let model = smoke_load_workspace(smoke_workspace_path());
        let geometry = model.displayed_geometry().expect("geometry");
        assert_eq!(
            model.workspace_summary().expect("workspace").revision(),
            Revision::ZERO
        );
        assert_eq!(geometry.geometry_revision, 1);
        assert_eq!(geometry.requested_output.as_str(), "root");
        assert!(matches!(model.build_status(), AppBuildStatus::Success(_)));
        assert_eq!(model.current_source_revision().get(), 1);
    }

    #[test]
    fn failing_rebuild_preserves_last_good_displayed_geometry() {
        let workspace_root = clone_workspace_fixture();
        let mut model = smoke_load_workspace(&workspace_root);
        let last_good_revision = model
            .displayed_geometry()
            .expect("geometry")
            .geometry_revision;

        let workspace = model.workspace_mut().expect("workspace");
        workspace.replace_source("schema_version = 1\nroot = \"broken\"\n");
        workspace.save().expect("save");

        let request = model
            .schedule_manual_rebuild(Instant::now())
            .expect("request");
        let mut worker = BuildWorker::new();
        let action = model.accept_build_outcome(worker.process(request));
        assert!(!action.refresh_displayed_geometry());
        assert!(matches!(
            model.build_status(),
            AppBuildStatus::SceneError(_)
        ));
        assert_eq!(
            model
                .displayed_geometry()
                .expect("geometry")
                .geometry_revision,
            last_good_revision
        );
    }

    #[test]
    fn invalid_source_then_valid_source_recovers_automatically() {
        let workspace_root = clone_workspace_fixture();
        let mut model = smoke_load_workspace(&workspace_root);
        let original_text = model
            .workspace_mut()
            .expect("workspace")
            .source_text()
            .to_owned();
        let last_good_revision = model
            .displayed_geometry()
            .expect("geometry")
            .geometry_revision;
        let mut worker = BuildWorker::new();

        {
            let workspace = model.workspace_mut().expect("workspace");
            workspace.replace_source("schema_version = 1\nroot = \"broken\"\n");
            workspace.save().expect("save invalid");
        }
        let invalid_request = model
            .schedule_manual_rebuild(Instant::now())
            .expect("request");
        let invalid_action = model.accept_build_outcome(worker.process(invalid_request));
        assert!(!invalid_action.refresh_displayed_geometry());
        assert!(matches!(
            model.build_status(),
            AppBuildStatus::SceneError(_)
        ));
        assert_eq!(
            model
                .displayed_geometry()
                .expect("geometry")
                .geometry_revision,
            last_good_revision
        );

        {
            let workspace = model.workspace_mut().expect("workspace");
            workspace.replace_source(original_text);
            workspace.save().expect("save valid");
        }
        let valid_request = model
            .schedule_manual_rebuild(Instant::now())
            .expect("request");
        let valid_action = model.accept_build_outcome(worker.process(valid_request));
        assert!(valid_action.refresh_displayed_geometry());
        assert!(matches!(model.build_status(), AppBuildStatus::Success(_)));
        assert!(
            model
                .displayed_geometry()
                .expect("geometry")
                .geometry_revision
                >= last_good_revision
        );
    }

    #[test]
    fn programmatic_edit_persists_and_own_write_echo_is_suppressed() {
        let workspace_root = clone_workspace_fixture();
        let mut model = smoke_load_workspace(&workspace_root);
        let request = model
            .apply_parameter_scalar_delta(
                &ParamId::new(DEMO_PARAMETER_ID).expect("parameter id"),
                0.25,
                EditOrigin::Programmatic,
                Instant::now(),
            )
            .expect("edit")
            .expect("request");
        let mut worker = BuildWorker::new();
        let action = model.accept_build_outcome(worker.process(request));
        assert!(action.refresh_displayed_geometry());
        assert!(matches!(model.build_status(), AppBuildStatus::Success(_)));
        assert!(
            model
                .workspace_summary()
                .expect("workspace")
                .revision()
                .get()
                >= 2
        );
        let echo = model
            .accept_external_reload(Instant::now())
            .expect("reload");
        assert!(echo.is_none());
    }

    #[test]
    fn stale_result_from_previous_session_is_ignored_after_reopen() {
        let workspace_root = clone_workspace_fixture();
        let mut model = AppModel::new(Some(workspace_root));
        let old_request = model.prepare_reopen(Instant::now()).expect("first reopen");
        let new_request = model.prepare_reopen(Instant::now()).expect("second reopen");
        let mut old_worker = BuildWorker::new();
        let mut new_worker = BuildWorker::new();

        let stale = model.accept_build_outcome(old_worker.process(old_request));
        assert!(!stale.refresh_displayed_geometry());

        let current = model.accept_build_outcome(new_worker.process(new_request));
        assert!(current.refresh_displayed_geometry());
    }

    #[test]
    fn camera_state_survives_geometry_refresh() {
        let mut viewport = ViewportRuntimeState {
            camera: OrbitCameraState {
                target: Vec3::new(1.0, 2.0, 3.0),
                distance: 17.0,
                yaw: 0.7,
                pitch: -0.4,
                aspect_ratio: 1.8,
            },
            ..Default::default()
        };
        let camera_before = viewport.camera;
        let geometry = DisplayedGeometry {
            requested_output: geom_scene::NodeId::new("root").expect("node id"),
            geometry_revision: 4,
            mesh: MorphosMesh::new(
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                vec![[0, 1, 2]],
            )
            .expect("mesh"),
            bounds: Bounds::Finite {
                min: [-1.0, -1.0, -1.0],
                max: [1.0, 1.0, 1.0],
            },
        };
        viewport.accept_displayed_geometry(&geometry);
        assert_eq!(viewport.camera, camera_before);
    }

    #[test]
    fn frame_operation_uses_bounds_center_and_finite_distance() {
        let bounds = Bounds::Finite {
            min: [2.0, 3.0, 4.0],
            max: [6.0, 7.0, 10.0],
        };
        let frame = CameraFrame::from_bounds(&bounds, 16.0 / 9.0);
        assert_eq!(frame.target, Vec3::new(4.0, 5.0, 7.0));
        assert!(frame.distance.is_finite());
        assert!(frame.distance > 0.0);
    }

    #[test]
    fn watcher_events_route_through_debounce_state() {
        let scene_path = smoke_workspace_path().join("source").join("scene.toml");
        let (sender, receiver) = unbounded();
        let mut runtime = ReactiveRuntimeState {
            watcher: Some(WatcherSession {
                session_id: WorkspaceSessionId::ZERO,
                scene_path: scene_path.clone(),
                receiver,
                _watcher: notify::recommended_watcher(|_| {}).expect("watcher"),
            }),
            ..Default::default()
        };
        let mut model = AppModel::new(Some(smoke_workspace_path()));
        let request = model.prepare_reopen(Instant::now()).expect("reopen");
        dispatch_build_request(&mut runtime, request);

        sender
            .send(WatchedSourceEvent {
                session_id: WorkspaceSessionId::ZERO,
                paths: vec![scene_path],
                observed_at: Instant::now(),
            })
            .expect("send");
        model.note_file_event(Instant::now());
        assert!(
            model
                .drain_ready_file_event(Instant::now() + Duration::from_millis(75))
                .is_some()
        );
    }

    #[test]
    fn delete_safety_reports_root_and_referenced_nodes_conservatively() {
        let model = smoke_load_workspace(smoke_workspace_path());
        let root = model
            .delete_node_safety(&NodeId::new("root").expect("root"))
            .expect("root safety");
        assert!(root.is_root);

        let arm = model
            .delete_node_safety(&NodeId::new("arm").expect("arm"))
            .expect("arm safety");
        assert!(!arm.is_root);
        assert_eq!(
            arm.direct_dependents,
            vec![NodeId::new("union_shape").expect("dependent")]
        );
        assert!(
            arm.transitive_dependents
                .contains(&NodeId::new("root").expect("root dependent"))
        );
    }

    #[test]
    fn invalid_source_disables_canonical_gui_edits() {
        let workspace_root = clone_workspace_fixture();
        let mut model = smoke_load_workspace(&workspace_root);
        {
            let workspace = model.workspace_mut().expect("workspace");
            workspace.replace_source("schema_version = 1\nroot = \"broken\"\n");
            workspace.save().expect("save invalid");
        }
        let request = model
            .schedule_manual_rebuild(Instant::now())
            .expect("request");
        let mut worker = BuildWorker::new();
        let _ = model.accept_build_outcome(worker.process(request));

        let result = model.set_node_label(
            &NodeId::new("root").expect("id"),
            Some("blocked"),
            EditOrigin::Gui,
            Instant::now(),
        );
        assert!(matches!(result, Err(AppEditError::Conflict(_))));
    }

    #[test]
    fn selection_persists_across_unrelated_reload_and_falls_back_to_root_when_missing() {
        let workspace_root = clone_workspace_from(&authoring_smoke_workspace_path());
        let mut model = smoke_load_workspace(&workspace_root);
        let mut selection = ViewportSelection {
            selected_node: Some(NodeId::new("spare_sensor").expect("spare sensor")),
        };

        {
            let workspace = model.workspace_mut().expect("workspace");
            let updated = workspace
                .source_text()
                .replace("value = 1.6", "value = 1.9");
            workspace.replace_source(updated);
            workspace.save().expect("save unrelated edit");
        }
        let request = model
            .schedule_manual_rebuild(Instant::now())
            .expect("manual rebuild");
        let mut worker = BuildWorker::new();
        let _ = model.accept_build_outcome(worker.process(request));
        model.preserve_selection(&mut selection);
        assert_eq!(
            selection.selected_node,
            Some(NodeId::new("spare_sensor").expect("spare sensor"))
        );

        {
            let workspace = model.workspace_mut().expect("workspace");
            let source = workspace.source_text().to_owned();
            let mut scene = geom_scene::SceneSource::parse(&source).expect("parse scene");
            scene
                .delete_node(&NodeId::new("spare_sensor").expect("spare sensor"))
                .expect("delete spare sensor");
            workspace.replace_source(scene.into_text());
            workspace.save().expect("save deleted arm");
        }
        let request = model
            .schedule_manual_rebuild(Instant::now())
            .expect("manual rebuild");
        let _ = model.accept_build_outcome(worker.process(request));
        model.preserve_selection(&mut selection);
        assert_eq!(
            selection.selected_node,
            Some(NodeId::new("assembly").expect("assembly"))
        );
    }

    #[test]
    fn parameter_value_edit_preserves_parameter_reference_after_reopen() {
        let workspace_root = clone_workspace_fixture();
        let mut model = smoke_load_workspace(&workspace_root);
        let request = model
            .set_parameter_scalar_value(
                &ParamId::new("arm_length").expect("parameter"),
                3.4,
                EditOrigin::Gui,
                Instant::now(),
            )
            .expect("set parameter")
            .expect("request");
        let mut worker = BuildWorker::new();
        let _ = model.accept_build_outcome(worker.process(request));

        let reopened = geom_scene::parse_scene(
            &fs::read_to_string(workspace_root.join("source").join("scene.toml")).expect("source"),
        )
        .expect("reparse");
        match reopened.nodes()[&NodeId::new("arm").expect("arm")].kind() {
            geom_scene::NodeKind::Cylinder(cylinder) => {
                assert!(matches!(
                    cylinder.height,
                    geom_scene::ScalarExpr::Parameter(_)
                ));
            }
            other => panic!("unexpected arm kind: {other:?}"),
        }
        assert_eq!(
            reopened.parameters()[&ParamId::new("arm_length").expect("parameter")].scalar_value(),
            3.4
        );
    }

    #[test]
    fn gui_transform_edit_persists_through_transaction_and_reopen() {
        let workspace_root = clone_workspace_fixture();
        let mut model = smoke_load_workspace(&workspace_root);
        let request = model
            .set_selected_node_transform_literal(
                &NodeId::new("body").expect("body"),
                geom_scene::TransformProperty::Translation,
                geom_scene::Axis::X,
                2.75,
                EditOrigin::Gui,
                Instant::now(),
            )
            .expect("set transform")
            .expect("request");
        let mut worker = BuildWorker::new();
        let _ = model.accept_build_outcome(worker.process(request));

        assert!(
            model
                .workspace_summary()
                .expect("workspace")
                .revision()
                .get()
                >= 2,
            "workspace revision should advance through the persisted transaction"
        );

        let reopened = geom_scene::parse_scene(
            &fs::read_to_string(workspace_root.join("source").join("scene.toml")).expect("source"),
        )
        .expect("reparse");
        match &reopened.nodes()[&NodeId::new("body").expect("body")]
            .transform()
            .translation
            .x
        {
            geom_scene::ScalarExpr::Literal(value) => assert_eq!(*value, 2.75),
            other => panic!("unexpected translation expr: {other:?}"),
        }
    }

    #[test]
    fn undo_and_redo_flow_through_workspace_transaction_and_reopen() {
        let workspace_root = clone_workspace_fixture();
        let mut model = smoke_load_workspace(&workspace_root);

        let request = model
            .set_parameter_scalar_value(
                &ParamId::new("arm_length").expect("parameter"),
                3.8,
                EditOrigin::Gui,
                Instant::now(),
            )
            .expect("set parameter")
            .expect("request");
        let mut worker = BuildWorker::new();
        let _ = model.accept_build_outcome(worker.process(request));
        assert!(model.undo_redo_availability().can_undo);

        let request = model
            .undo(EditOrigin::Gui, Instant::now())
            .expect("undo")
            .expect("undo request");
        let _ = model.accept_build_outcome(worker.process(request));
        let reopened = geom_scene::parse_scene(
            &fs::read_to_string(workspace_root.join("source").join("scene.toml")).expect("source"),
        )
        .expect("reparse");
        assert_eq!(
            reopened.parameters()[&ParamId::new("arm_length").expect("parameter")].scalar_value(),
            2.6
        );
        assert!(model.undo_redo_availability().can_redo);

        let request = model
            .redo(EditOrigin::Gui, Instant::now())
            .expect("redo")
            .expect("redo request");
        let _ = model.accept_build_outcome(worker.process(request));
        let reopened = geom_scene::parse_scene(
            &fs::read_to_string(workspace_root.join("source").join("scene.toml")).expect("source"),
        )
        .expect("reparse");
        assert_eq!(
            reopened.parameters()[&ParamId::new("arm_length").expect("parameter")].scalar_value(),
            3.8
        );
    }

    #[test]
    fn external_reload_clears_undo_and_redo_availability() {
        let workspace_root = clone_workspace_fixture();
        let mut model = smoke_load_workspace(&workspace_root);
        let request = model
            .set_parameter_scalar_value(
                &ParamId::new("arm_length").expect("parameter"),
                3.1,
                EditOrigin::Gui,
                Instant::now(),
            )
            .expect("set parameter")
            .expect("request");
        let mut worker = BuildWorker::new();
        let _ = model.accept_build_outcome(worker.process(request));
        assert!(model.undo_redo_availability().can_undo);

        {
            let workspace = model.workspace_mut().expect("workspace");
            let updated = workspace
                .source_text()
                .replace("value = 3.1", "value = 4.4");
            fs::write(workspace.paths().source_file(), updated).expect("write external edit");
        }

        let request = model
            .accept_external_reload(Instant::now())
            .expect("reload")
            .expect("request");
        let _ = model.accept_build_outcome(worker.process(request));
        assert!(!model.undo_redo_availability().can_undo);
        assert!(!model.undo_redo_availability().can_redo);
    }

    #[test]
    #[ignore = "manual timing harness"]
    fn reactive_timing_harness() {
        let smoke = measure_programmatic_edit_latency(
            &clone_workspace_from(&smoke_workspace_path()),
            DEMO_PARAMETER_ID,
            0.15,
        );
        println!(
            "smoke parse={:.2}ms eval={:.2}ms mesh={:.2}ms total={:.2}ms",
            smoke.parse_millis,
            smoke.evaluation_millis,
            smoke.mesh_upload_millis,
            smoke.total_millis
        );

        let benchmark = measure_programmatic_edit_latency(
            &clone_workspace_from(&benchmark_workspace_path()),
            "left_width",
            0.1,
        );
        println!(
            "benchmark parse={:.2}ms eval={:.2}ms mesh={:.2}ms total={:.2}ms",
            benchmark.parse_millis,
            benchmark.evaluation_millis,
            benchmark.mesh_upload_millis,
            benchmark.total_millis
        );
    }

    fn measure_programmatic_edit_latency(
        workspace_root: &Path,
        parameter: &str,
        delta: f64,
    ) -> reactive::ReactiveBuildTimings {
        let mut model = AppModel::new(Some(workspace_root.to_path_buf()));
        let initial = model
            .prepare_reopen(Instant::now())
            .expect("initial reopen");
        let mut worker = BuildWorker::new();
        let _ = model.accept_build_outcome(worker.process(initial));

        let request = model
            .apply_parameter_scalar_delta(
                &ParamId::new(parameter).expect("parameter id"),
                delta,
                EditOrigin::Programmatic,
                Instant::now(),
            )
            .expect("edit")
            .expect("request");
        let action = model.accept_build_outcome(worker.process(request));
        if action.refresh_displayed_geometry() {
            let upload_started = Instant::now();
            let displayed = model.displayed_geometry().expect("geometry");
            let _mesh = adapt_morphos_mesh(&displayed.mesh).expect("adapt mesh");
            let mesh_upload_millis = Instant::now()
                .saturating_duration_since(upload_started)
                .as_secs_f64()
                * 1_000.0;
            let requested_at = action.requested_at().expect("requested at");
            let total_millis = Instant::now()
                .saturating_duration_since(requested_at)
                .as_secs_f64()
                * 1_000.0;
            model.note_mesh_upload_complete(mesh_upload_millis, total_millis);
        }

        match model.build_status() {
            AppBuildStatus::Success(success) => success.timings,
            status => panic!("expected success, found {status:?}"),
        }
    }
}
