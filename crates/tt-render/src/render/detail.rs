//! Which detail level of a shape is drawn.

use super::dynamic::Layer;
use super::meshes::ShapeMesh;
use super::Renderer;
use glam::{Mat4, Vec3};

/// What the game's detail prefs change in the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detail {
    /// Scales the projected pixel size that picks a shape's detail level
    /// ([`select_detail`]).
    pub ts_detail_adjust: f32,
    /// The highest detail levels skipped.
    pub skip_render_dls: u32,
    /// Dynamic (tank) shadows are drawn only at 0.2 and above.
    pub shadow_detail: f32,
    /// Mip levels dropped from shape textures. Fixed at creation.
    pub mip_reduction: u32,
    /// The same for the sky. Fixed at creation.
    pub sky_mip_reduction: u32,
    /// Mip levels dropped from the terrain textures. Fixed at creation.
    pub terrain_tex_detail: u32,
}

impl Default for Detail {
    fn default() -> Detail {
        Detail { ts_detail_adjust: 1.0, skip_render_dls: 0, shadow_detail: 1.0, mip_reduction: 0, sky_mip_reduction: 0, terrain_tex_detail: 0 }
    }
}

impl Detail {
    /// The settings fixed when the renderer is created (texture sampling).
    pub fn textures(&self) -> (u32, u32, u32) {
        (self.mip_reduction, self.sky_mip_reduction, self.terrain_tex_detail)
    }

    /// Draws only if `0.2 <= ShadowDetail`.
    pub fn shadows(&self) -> bool {
        self.shadow_detail >= 0.2
    }
}

pub fn detail_pixel_scale(viewport: (u32, u32), vfov_deg: f32) -> f32 {
    let (w, h) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
    (h * 0.5) / (vfov_deg.to_radians() * 0.5).tan().max(1e-4) * (w / 640.0)
}

/// Detail-level table of one shape for [`select_detail`].
#[derive(Clone, Debug, Default)]
pub struct DetailTable {
    /// `TSShape::radius`.
    pub radius: f32,
    /// `size` of each detail level (negative: collision/shadow details).
    pub sizes: Vec<f32>,
    pub smallest_visible_size: f32,
    pub smallest_visible_dl: i32,
}

impl DetailTable {
    pub fn of(shape: &tt_assets::dts::Shape) -> DetailTable {
        DetailTable {
            radius: shape.radius,
            sizes: shape.detail_levels.iter().map(|d| d.size).collect(),
            smallest_visible_size: shape.smallest_visible_size as f32,
            smallest_visible_dl: shape.smallest_visible_dl,
        }
    }

    /// Detail levels that can be selected: 0 up to the smallest visible one.
    pub fn visible_levels(&self) -> usize {
        if self.sizes.is_empty() {
            return 0;
        }
        let last = self.sizes.iter().position(|&s| s < 0.0).unwrap_or(self.sizes.len());
        ((self.smallest_visible_dl.max(0) as usize) + 1).min(last.max(1))
    }
}

/// [`select_detail`] for an object at `model` seen from `cam`: the distance
/// to the object's origin divided by its largest scale.
fn instance_detail(t: &DetailTable, model: Mat4, cam: Vec3, pixel_scale: f32, d: &Detail) -> usize {
    let scale = model.x_axis.truncate().length().max(model.y_axis.truncate().length()).max(model.z_axis.truncate().length()).max(1e-3);
    let dist = model.w_axis.truncate().distance(cam) / scale;
    select_detail(t, dist, pixel_scale, d.ts_detail_adjust, d.skip_render_dls)
}

/// - pixel radius = radius / dist · `pixel_scale` · `detailAdjust`; below
///   the smallest visible size it is raised to just above it, so a shape
///   never disappears through this path (its smallest level may be empty);
/// - the first level `i` with `size[i] < pixel radius`, or whose next level
///   is a collision/shadow level (negative size), or the last;
/// - levels below `min(skipRenderDLs, smallestVisibleDL)` are skipped.
pub fn select_detail(t: &DetailTable, dist: f32, pixel_scale: f32, detail_adjust: f32, skip_render_dls: u32) -> usize {
    let n = t.sizes.len();
    if n == 0 {
        return 0;
    }
    let mut pixel = t.radius / dist.max(1e-3) * pixel_scale * detail_adjust;
    if pixel <= t.smallest_visible_size {
        pixel = t.smallest_visible_size + 0.01;
    }
    let mut dl = n - 1;
    for i in 0..n {
        if t.sizes[i] < pixel || i + 1 >= n || t.sizes[i + 1] < 0.0 {
            dl = i;
            break;
        }
    }
    let skip = (skip_render_dls as i32).min(t.smallest_visible_dl).max(0) as usize;
    dl.max(skip).min(t.visible_levels().saturating_sub(1))
}

impl Renderer {
    /// Picks every static shape's detail level for a view from `cam`
    /// ([`select_detail`]) and rewrites the static instance buffer grouped by
    /// (shape, level), recording the draws in `static_draws`.
    pub(super) fn select_static_details(&mut self, queue: &wgpu::Queue, cam: Vec3, pixel_scale: f32) {
        self.static_draws.clear();
        let Some((mesh, buffer)) = self.shapes.as_ref() else { return };
        let d = self.settings.detail;
        let mut sorted = Vec::with_capacity(self.static_instances.len());
        for (gi, g) in mesh.lods.iter().enumerate() {
            let levels = mesh.by_level[gi].len();
            let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); levels];
            for i in g.instances.clone() {
                let Some(inst) = self.static_instances.get(i as usize) else { continue };
                let m = Mat4::from_cols_array_2d(&inst.model);
                let level = instance_detail(&g.table, m, cam, pixel_scale, &d).min(levels - 1);
                buckets[level].push(i as usize);
            }
            for (level, list) in buckets.into_iter().enumerate() {
                if list.is_empty() {
                    continue;
                }
                let start = sorted.len() as u32;
                sorted.extend(list.iter().map(|&i| self.static_instances[i]));
                let range = start..sorted.len() as u32;
                self.static_draws.extend(mesh.by_level[gi][level].iter().map(|&b| (b, range.clone())));
            }
        }
        if !sorted.is_empty() {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&sorted));
        }
    }

    /// Detail level of a moving shape's instance for the current view.
    pub(super) fn dynamic_detail(&self, mesh: &ShapeMesh, l: &Layer, i: u32, cam: Vec3, pixel_scale: f32) -> u32 {
        let (Some(g), Some(m)) = (mesh.lods.first(), l.transforms.get(i as usize)) else { return 0 };
        instance_detail(&g.table, *m, cam, pixel_scale, &self.settings.detail) as u32
    }
}

#[cfg(test)]
mod detail_tests {
    use super::*;
    use crate::render::vertical_fov;

    /// `treeblue01.dts`: detail100..detail2, then shadow details.
    fn tree() -> DetailTable {
        DetailTable { radius: 6.77, sizes: vec![100.0, 80.0, 60.0, 40.0, 20.0, 10.0, 2.0, -1.0, -2.0, -3.0], smallest_visible_size: 2.0, smallest_visible_dl: 6 }
    }

    #[test]
    fn detail_follows_the_projected_size() {
        let t = tree();
        assert_eq!(t.visible_levels(), 7);
        // 640x480 at 90° horizontal FOV: 320 px per unit at distance 1.
        let ps = detail_pixel_scale((640, 480), vertical_fov(90.0, 640.0 / 480.0));
        assert!((ps - 320.0).abs() < 0.5, "{ps}");
        // Pixel radius 6.77·320/d: above 100 nearer than 21.7 m.
        assert_eq!(select_detail(&t, 20.0, ps, 1.0, 0), 0);
        assert_eq!(select_detail(&t, 25.0, ps, 1.0, 0), 1);
        assert_eq!(select_detail(&t, 50.0, ps, 1.0, 0), 3);
        // Far away: the smallest visible level, never past it.
        assert_eq!(select_detail(&t, 5000.0, ps, 1.0, 0), 6);
        // detailAdjust 0.8 drops a level sooner; skipRenderDLs 1 never draws detail 0.
        assert_eq!(select_detail(&t, 20.0, ps, 0.8, 0), 1);
        assert_eq!(select_detail(&t, 1.0, ps, 1.0, 1), 1);
        // The pixel scale grows with the square of the width.
        let hd = detail_pixel_scale((1280, 960), vertical_fov(90.0, 1280.0 / 960.0));
        assert!((hd - 4.0 * ps).abs() < 1.0);
    }

    #[test]
    fn shadows_need_shadow_detail_0_2() {
        assert!(Detail::default().shadows());
        assert!(Detail { shadow_detail: 0.5, ..Detail::default() }.shadows());
        assert!(!Detail { shadow_detail: 0.0, ..Detail::default() }.shadows());
    }
}
