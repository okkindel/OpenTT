//! The camera and the per-view uniforms.

use glam::{Mat4, Vec3};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Globals {
    pub(super) view_proj: [[f32; 4]; 4],
    pub(super) camera: [f32; 4],
    pub(super) sun_dir: [f32; 4],
    pub(super) sun_color: [f32; 4],
    pub(super) ambient: [f32; 4],
    pub(super) fog_color: [f32; 4],
    pub(super) params: [f32; 4],
    // x: seconds, y: IFL ticks per second
    pub(super) time: [f32; 4],
    // x: terrain grid origin (world, both axes), y: metres per square, z: samples per edge
    pub(super) terrain: [f32; 4],
    // Height fog bands: x visible distance (0 = unused), y min height, z max height.
    pub(super) fog_volumes: [[f32; 4]; 3],
    // Camera basis for billboards (world space).
    pub(super) cam_right: [f32; 4],
    pub(super) cam_up: [f32; 4],
    pub(super) cam_forward: [f32; 4],
    // Graphics extras: x normal-map strength, y glass, z emissive boost, w sun glow.
    pub(super) extras: [f32; 4],
    // Dynamic lights (graphics extra): x count.
    pub(super) light_info: [f32; 4],
    // Per light: xyz position, w radius; rgb colour (linear, × intensity).
    pub(super) lights: [[f32; 4]; 2 * MAX_POINT_LIGHTS],
}

/// Most dynamic point lights a view is lit by (the nearest are kept).
pub const MAX_POINT_LIGHTS: usize = 16;

/// A short-lived coloured point light (explosions, shots, projectiles; the
/// "Dynamic lights" graphics extra). Lights terrain and shapes within
/// `radius` with a smooth falloff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
    pub pos: Vec3,
    pub radius: f32,
    /// Linear RGB times intensity (1 = a white light as bright as full sun
    /// at the centre).
    pub color: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: Vec3,
    /// Radians; 0 looks along +Y (north), increasing turns towards +X.
    pub yaw: f32,
    pub pitch: f32,
}

impl Camera {
    /// Roll is not represented and is dropped (the view stays upright).
    pub fn from_matrix(m: Mat4) -> Camera {
        let pos = m.w_axis.truncate();
        let f = m.y_axis.truncate().try_normalize().unwrap_or(Vec3::Y);
        Camera { pos, yaw: f.x.atan2(f.y), pitch: f.z.clamp(-1.0, 1.0).asin() }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.yaw.cos() * self.pitch.cos(), self.pitch.sin())
    }

    /// `fov_deg`: vertical field of view.
    pub fn view_proj_fov(&self, aspect: f32, far: f32, fov_deg: f32) -> Mat4 {
        self.view_proj_near(aspect, 0.5, far, fov_deg)
    }

    /// [`Camera::view_proj_fov`] with an explicit near plane.
    pub fn view_proj_near(&self, aspect: f32, near: f32, far: f32, fov_deg: f32) -> Mat4 {
        let view = Mat4::look_to_rh(self.pos, self.forward(), Vec3::Z);
        Mat4::perspective_rh(fov_deg.clamp(1.0, 179.0).to_radians(), aspect, near.max(1e-3), far) * view
    }
}

pub fn vertical_fov(horizontal_deg: f32, aspect: f32) -> f32 {
    (2.0 * ((horizontal_deg.to_radians() * 0.5).tan() / aspect.max(1e-3)).atan()).to_degrees()
}

/// How to draw one frame.
pub struct View {
    pub camera: Camera,
    /// Vertical field of view in degrees (see [`vertical_fov`]).
    pub fov_deg: f32,
    /// Sub-rectangle `[x, y, w, h]` of the target in pixels; None = whole target.
    /// Pixels outside it are left untouched.
    pub viewport: Option<[u32; 4]>,
    /// Fill the viewport with the fog colour first; false keeps the existing
    /// colour (depth is always cleared).
    pub clear: bool,
}
