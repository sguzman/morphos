use bevy::prelude::*;
use geom_geometry::Bounds;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitCameraState {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub aspect_ratio: f32,
}

impl Default for OrbitCameraState {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: 12.0,
            yaw: 0.65,
            pitch: -0.45,
            aspect_ratio: 16.0 / 9.0,
        }
    }
}

impl OrbitCameraState {
    pub fn orbit(&mut self, delta: Vec2, input: OrbitCameraInputMap) {
        self.yaw -= delta.x * input.orbit_sensitivity;
        self.pitch = (self.pitch - delta.y * input.orbit_sensitivity).clamp(-1.5, 1.5);
    }

    pub fn pan(&mut self, delta: Vec2, input: OrbitCameraInputMap) {
        let right = self.right();
        let up = self.up();
        let speed = self.distance * input.pan_speed;
        let translation = (-delta.x * speed * right) + (delta.y * speed * up);
        self.target += translation;
    }

    pub fn zoom(&mut self, scroll_delta: f32, input: OrbitCameraInputMap) {
        let factor = (1.0 - scroll_delta * input.zoom_speed).clamp(0.1, 10.0);
        self.distance = (self.distance * factor).clamp(0.05, 5_000.0);
    }

    pub fn transform(&self) -> Transform {
        let rotation = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0);
        let offset = rotation * Vec3::new(0.0, 0.0, self.distance);
        Transform::from_translation(self.target + offset).looking_at(self.target, Vec3::Y)
    }

    pub fn apply_frame(&mut self, frame: CameraFrame) {
        self.target = frame.target;
        self.distance = frame.distance.max(0.05);
    }

    pub fn frame_bounds(&mut self, bounds: &Bounds, aspect_ratio: f32) {
        self.aspect_ratio = aspect_ratio.max(0.1);
        self.apply_frame(CameraFrame::from_bounds(bounds, self.aspect_ratio));
    }

    fn right(&self) -> Vec3 {
        let rotation = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0);
        rotation * Vec3::X
    }

    fn up(&self) -> Vec3 {
        let rotation = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0);
        rotation * Vec3::Y
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitCameraInputMap {
    pub orbit_button: MouseButton,
    pub pan_button: MouseButton,
    pub orbit_sensitivity: f32,
    pub pan_speed: f32,
    pub zoom_speed: f32,
}

impl Default for OrbitCameraInputMap {
    fn default() -> Self {
        Self {
            orbit_button: MouseButton::Right,
            pan_button: MouseButton::Middle,
            orbit_sensitivity: 0.01,
            pan_speed: 0.002,
            zoom_speed: 0.12,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraFrame {
    pub target: Vec3,
    pub distance: f32,
}

impl CameraFrame {
    pub fn from_bounds(bounds: &Bounds, aspect_ratio: f32) -> Self {
        let (target, extent) = match bounds {
            Bounds::Empty => (Vec3::ZERO, Vec3::splat(0.5)),
            Bounds::Finite { min, max } => {
                let min = Vec3::new(min[0] as f32, min[1] as f32, min[2] as f32);
                let max = Vec3::new(max[0] as f32, max[1] as f32, max[2] as f32);
                let size = (max - min).max(Vec3::splat(0.001));
                ((min + max) * 0.5, size * 0.5)
            }
        };

        let radius = extent.length().max(0.2);
        let vertical_fov = 45.0_f32.to_radians();
        let horizontal_fov = 2.0 * ((vertical_fov * 0.5).tan() * aspect_ratio.max(0.1)).atan();
        let distance_y = radius / (vertical_fov * 0.5).tan();
        let distance_x = radius / (horizontal_fov * 0.5).tan();
        let distance = distance_x.max(distance_y) * 1.35;

        Self { target, distance }
    }
}

#[derive(Debug, Clone, Copy, Component)]
pub struct OrbitCameraController;
