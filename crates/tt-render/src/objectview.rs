//! Shape preview without a mission: ThinkTanks' `GuiObjectViewCtrl` (tank
//! selection screen) and similar shell views.
//!
//! - Script API: `load(file, mount, mountNode)`, `set/getCameraDistance`,
//!   `set/getCameraRotation`, `set/getModelOffset`, `clear()`; persistent
//!   fields `cameraDistance` (default 1), `cameraRotation` (Point3F, default
//!   `0 0 2.7`) and `modelOffset` (default `0 0 0`).
//! - The models are tank01/02/03 (Light/Medium/ Heavy) and tank05 (Baby Boss) with
//!   brainjar01/02/03/04.
//! - The mounted shape is drawn with the main shape's `mountNode` transform;
//!   both in their default pose (no sequence thread is started).
//! - Camera: every rendered frame `rotation.z += 0.002` rad (a per-frame step, so the
//!   speed depends on the frame rate: 0.12 rad/s at 60 fps). The orientation is
//!   `Rz(rot.z)·Ry(rot.y)` — the `Rx(rot.x)` product is computed but overwritten, so
//!   pitch has no effect. The eye is at `shape.center + modelOffset − forward·distance`
//!   (forward = the matrix's Y column), looking horizontally at the shape centre.
//!   Projection: near 0.1, far 2100, horizontal fov 0.8976 rad (51.4°).
//! - Rendering: depth is cleared but not colour (the GUI bitmap behind shows through),
//!   fog off, no lights installed — the shapes are drawn unlit (full texture colour).
//!   [`ObjectLight::default`] reproduces that; a directional light can be used instead.

use crate::render::{Camera, DynamicInstance, Renderer, View};
use glam::{Mat4, Vec3};
use std::path::Path;

pub const SPIN_PER_FRAME: f32 = 0.002;
pub const HFOV: f32 = 0.8976;

/// Light for an object view. Colours are display-space; `k` per instance is
/// [`DynamicInstance::light`].
#[derive(Clone, Copy, Debug)]
pub struct ObjectLight {
    /// Direction the light travels (like `Sun.direction`).
    pub direction: Vec3,
    pub color: [f32; 3],
    pub ambient: [f32; 3],
}

impl Default for ObjectLight {
    fn default() -> ObjectLight {
        ObjectLight { direction: Vec3::new(0.0, std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2), color: [0.0; 3], ambient: [1.0; 3] }
    }
}

/// What is behind the shapes inside the viewport.
#[derive(Clone, Copy, Debug, Default)]
pub enum Background {
    /// Keep the target's pixels.
    #[default]
    Keep,
    /// Fill with a display-space RGBA colour (alpha 0 gives a transparent
    /// texture when rendering into an offscreen target).
    Color([f32; 4]),
}

/// The control's camera state.
#[derive(Clone, Copy, Debug)]
pub struct OrbitCamera {
    pub rotation: Vec3,
    /// `cameraDistance`.
    pub distance: f32,
    /// `modelOffset`.
    pub offset: Vec3,
}

impl Default for OrbitCamera {
    fn default() -> OrbitCamera {
        OrbitCamera { rotation: Vec3::new(0.0, 0.0, 2.7), distance: 1.0, offset: Vec3::ZERO }
    }
}

impl OrbitCamera {
    /// One rendered frame of the game's automatic turn.
    pub fn spin_frame(&mut self) {
        self.rotation.z += SPIN_PER_FRAME;
    }

    /// Eye looking at `center` (normally [`shape_center`] of the main shape).
    /// Roll (`rotation.y`) is not represented.
    pub fn camera(&self, center: Vec3) -> Camera {
        let forward = Vec3::new(self.rotation.z.sin(), self.rotation.z.cos(), 0.0);
        Camera { pos: center + self.offset - forward * self.distance.max(0.0), yaw: self.rotation.z, pitch: 0.0 }
    }
}

/// `TSShape::center` of a shape file (bounds centre stored in the shape).
pub fn shape_center(shape: &Path) -> Option<Vec3> {
    crate::pose::shape(shape).map(|s| Vec3::from(s.center))
}

/// Instance of `mount` attached at `node` of `base` (posed with
/// `base.nodes`), like the control's mounted shape.
pub fn mounted(base: &DynamicInstance, mount: &Path, node: &str) -> Option<DynamicInstance> {
    let shape = crate::pose::shape(&base.shape)?;
    let m = crate::pose::node_transform(&shape, &base.nodes, node)?;
    Some(DynamicInstance { shape: mount.to_path_buf(), transform: base.transform * m, light: base.light, ..Default::default() })
}

/// Draws dynamic shapes into a viewport without a mission scene.
pub struct ObjectView {
    renderer: Renderer,
    /// Background inside the viewport.
    pub background: Background,
    /// Horizontal field of view in radians (default: the game's).
    pub hfov: f32,
    pub near: f32,
    pub far: f32,
    /// Seconds, for IFL texture animation.
    pub time: f32,
}

impl ObjectView {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> ObjectView {
        ObjectView { renderer: Renderer::objects_only(device, queue, format), background: Background::Keep, hfov: HFOV, near: 0.1, far: 2100.0, time: 0.0 }
    }

    /// Renders `instances` into `viewport` (`[x, y, w, h]` of the
    /// `width`×`height` target; None = all of it). Shapes are loaded on first
    /// use and cached by path, as for [`Renderer::set_dynamic`].
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
        viewport: Option<[u32; 4]>,
        instances: &[DynamicInstance],
        camera: &Camera,
        light: &ObjectLight,
    ) {
        let (clear, bg) = match self.background {
            Background::Keep => (false, [0.0; 4]),
            Background::Color(c) => (true, [crate::srgb_to_linear(c[0]), crate::srgb_to_linear(c[1]), crate::srgb_to_linear(c[2]), c[3].clamp(0.0, 1.0)]),
        };
        self.renderer.set_view_params(-light.direction, light.color, light.ambient, bg, self.near, self.far);
        self.renderer.set_dynamic(device, queue, instances);
        let [w, h] = viewport.map_or([width, height], |v| [v[2], v[3]]);
        let aspect = w.max(1) as f32 / h.max(1) as f32;
        let view = View { camera: *camera, fov_deg: crate::render::vertical_fov(self.hfov.to_degrees(), aspect), viewport, clear };
        self.renderer.render_view(device, queue, target, width, height, &view, self.time);
    }

    /// The tank-selection preview: `tank` with `brain` mounted at
    /// `codeSeat`, both lit by `light` factor 1.
    pub fn tank_preview(tank: &Path, brain: &Path) -> Vec<DynamicInstance> {
        let base = DynamicInstance { shape: tank.to_path_buf(), transform: Mat4::IDENTITY, light: 1.0, ..Default::default() };
        let mut out = vec![base.clone()];
        out.extend(mounted(&base, brain, "codeSeat"));
        out
    }
}
