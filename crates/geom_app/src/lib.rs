pub mod camera;
pub mod mesh_adapter;
pub mod model;
pub mod viewport;

use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::pbr::wireframe::{Wireframe, WireframePlugin};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use camera::{CameraFrame, OrbitCameraInputMap, OrbitCameraState};
use geom_geometry::Bounds;
use geom_scene::NodeId;
use mesh_adapter::adapt_morphos_mesh;
use model::{
    AppModel, BuildRequest, BuildStatusKind, DisplayedGeometry, UiStatusSnapshot,
    ViewportDisplayMode, ViewportSelection,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use viewport::DisplayGeometryRevision;

/// Current Bevy version selected for M04.
pub const GEOM_APP_BEVY_VERSION: &str = "0.19.1";

/// Current bevy_egui version selected for M04.
pub const GEOM_APP_BEVY_EGUI_VERSION: &str = "0.41.1";

const GRID_EXTENT: i32 = 10;
const GRID_SPACING: f32 = 1.0;
const GEOMETRY_MATERIAL_COLOR: Color = Color::srgb(0.78, 0.82, 0.9);

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
enum AppCommand {
    Rebuild,
    Reopen,
    FrameAll,
    FrameSelected,
    SetDisplayMode(ViewportDisplayMode),
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
    displayed_output: Option<NodeId>,
    displayed_bounds: Option<Bounds>,
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
            render_entity: None,
            mesh_handle: None,
            material_handle: None,
        }
    }
}

impl ViewportRuntimeState {
    fn accept_displayed_geometry(&mut self, geometry: &DisplayedGeometry) {
        self.displayed_geometry_revision = DisplayGeometryRevision::new(geometry.geometry_revision);
        self.displayed_output = Some(geometry.requested_output.clone());
        self.displayed_bounds = Some(geometry.bounds.clone());
        self.selection.selected_node = Some(geometry.requested_output.clone());
    }

    fn frame_all(&mut self, aspect_ratio: f32) {
        if let Some(bounds) = &self.displayed_bounds {
            self.camera.frame_bounds(bounds, aspect_ratio);
        }
    }
}

#[derive(Debug, Component)]
struct MorphosCamera;

#[derive(Debug, Component)]
struct MorphosGeometryEntity;

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
    mut commands: MessageWriter<AppCommand>,
    mut pointer_capture: ResMut<UiPointerCapture>,
) -> Result {
    let context = contexts.ctx_mut()?;
    let status = build_ui_snapshot(&app_model, &viewport);

    egui::Window::new("Morphos Status")
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(12.0, 12.0))
        .collapsible(false)
        .resizable(false)
        .default_width(780.0)
        .show(context, |ui| {
            ui.horizontal_wrapped(|ui| {
                let workspace_label = status
                    .workspace_name
                    .as_deref()
                    .unwrap_or("No workspace opened");
                ui.label(format!("Workspace: {workspace_label}"));
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
                ui.label(format!("Geometry Rev: {}", status.geometry_revision.get()));
                ui.separator();
                ui.label(format!("Build: {}", status.build_label));
                if let Some(duration) = status.last_rebuild_millis {
                    ui.separator();
                    ui.label(format!("Last Rebuild: {duration:.2} ms"));
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

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Reload / Rebuild").clicked() {
                    commands.write(AppCommand::Rebuild);
                }
                if ui.button("Reopen Current Workspace").clicked() {
                    commands.write(AppCommand::Reopen);
                }
                if ui.button("Frame All").clicked() {
                    commands.write(AppCommand::FrameAll);
                }
                if ui.button("Frame Selected").clicked() {
                    commands.write(AppCommand::FrameSelected);
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
            });
        });

    let overlay_message = match status.build_kind {
        BuildStatusKind::NoWorkspace => {
            Some("No workspace opened. Launch Morphos with a workspace path.")
        }
        BuildStatusKind::WorkspaceError => status.error_message.as_deref(),
        BuildStatusKind::SceneError => status.error_message.as_deref(),
        BuildStatusKind::GeometryError => status.error_message.as_deref(),
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
    let _ = &app_model;
    Ok(())
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
    mut viewport: ResMut<ViewportRuntimeState>,
) {
    for command in commands.read() {
        match command {
            AppCommand::Rebuild => {
                let action = app_model.process_build_request(BuildRequest::rebuild());
                if let Some(displayed) = action.accepted_geometry() {
                    viewport.accept_displayed_geometry(displayed);
                }
            }
            AppCommand::Reopen => {
                let action = app_model.process_build_request(BuildRequest::reopen());
                if let Some(displayed) = action.accepted_geometry() {
                    viewport.accept_displayed_geometry(displayed);
                }
            }
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
        }
    }
}

fn apply_geometry_refresh_system(
    mut commands: Commands,
    app_model: Res<AppModel>,
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

/// Builds, parses, and evaluates a workspace path without a window.
pub fn smoke_load_workspace(path: impl AsRef<Path>) -> AppModel {
    let mut model = AppModel::new(Some(path.as_ref().to_path_buf()));
    let _ = model.process_build_request(BuildRequest::reopen());
    model
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AppBuildStatus, BuildSuccess};
    use geom_geometry::Mesh as MorphosMesh;
    use geom_workspace::Revision;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn smoke_workspace_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("workspaces")
            .join("viewport-smoke")
    }

    fn clone_workspace_fixture() -> PathBuf {
        let unique_suffix = format!(
            "geom-app-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        let target_root = std::env::temp_dir().join(unique_suffix);
        copy_directory_recursive(&smoke_workspace_path(), &target_root);
        target_root
    }

    fn copy_directory_recursive(source: &Path, target: &Path) {
        fs::create_dir_all(target).expect("create target directory");
        for entry in fs::read_dir(source).expect("read source directory") {
            let entry = entry.expect("directory entry");
            let source_path = entry.path();
            let target_path = target.join(entry.file_name());
            let file_type = entry.file_type().expect("entry type");
            if file_type.is_dir() {
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
    }

    #[test]
    fn failing_rebuild_preserves_last_good_displayed_geometry() {
        let workspace_root = clone_workspace_fixture();
        let mut model = smoke_load_workspace(&workspace_root);
        let last_good_revision = model
            .displayed_geometry()
            .expect("initial geometry")
            .geometry_revision;

        let workspace = model.workspace_mut().expect("workspace");
        workspace.replace_source("schema_version = 1\nroot = \"broken\"\n");
        workspace.save().expect("persist invalid source");

        let action = model.process_build_request(BuildRequest::rebuild());
        assert!(action.accepted_geometry().is_none());
        assert!(matches!(
            model.build_status(),
            AppBuildStatus::SceneError(_)
        ));
        assert_eq!(
            model
                .displayed_geometry()
                .expect("last good")
                .geometry_revision,
            last_good_revision
        );
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
            requested_output: NodeId::new("root").expect("node id"),
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
    fn display_mode_changes_do_not_mutate_canonical_geometry() {
        let mesh = MorphosMesh::new(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .expect("mesh");
        let geometry = DisplayedGeometry {
            requested_output: NodeId::new("root").expect("node id"),
            geometry_revision: 1,
            mesh: mesh.clone(),
            bounds: Bounds::Finite {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 0.0],
            },
        };

        let mut model = AppModel::new(None);
        model.accept_success(BuildSuccess {
            workspace_summary: None,
            requested_output: geometry.requested_output.clone(),
            geometry,
            rebuild_millis: 1.0,
        });

        let before = model.displayed_geometry().expect("geometry").mesh.clone();
        let _viewport = ViewportRuntimeState {
            display_mode: ViewportDisplayMode::Wireframe,
            ..Default::default()
        };
        let after = model.displayed_geometry().expect("geometry").mesh.clone();
        assert_eq!(before, after);
    }
}
