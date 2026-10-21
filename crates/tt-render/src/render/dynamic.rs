//! Moving shapes: their instances, layers, shadows and loading.

use super::meshes::{pose_group, ShapeMesh};
use super::shadows::{shadow_light_dir, sun_shadow_dir, ShadowCaster, MAX_CASTERS, SHADOW_FADE_HEIGHT, SHADOW_LENGTH_FACTOR, SHADOW_OPACITY, SOFT_SHADOW_OPACITY};
use super::upload::NORMAL_SIZE;
use super::Renderer;
use crate::pose::Rig;
use crate::shapes::{InstanceData, ShapeSet};
use glam::{Mat4, Vec2, Vec3};
use std::collections::HashMap;
use std::path::PathBuf;

/// A moving shape drawn on top of the static scene (tank, projectile,
/// power-up, flag...). It is not part of the baked lightmap.
#[derive(Clone, Debug)]
pub struct DynamicInstance {
    /// `.dts` path, looked up case-insensitively like the statics.
    pub shape: PathBuf,
    pub transform: Mat4,
    /// Sun diffuse factor `k`, normally [`Scene::shape_light`] at the object.
    pub light: f32,
    /// Local transform overrides by node name (case-insensitive), replacing
    /// the node's default transform relative to its parent; children follow.
    /// See [`crate::pose`] for sequence poses and node defaults.
    pub nodes: Vec<(String, Mat4)>,
    /// Objects to hide / to show although hidden by default (by object name,
    /// case-insensitive; `hidden` wins).
    pub hidden: Vec<String>,
    pub shown: Vec<String>,
}

impl Default for DynamicInstance {
    fn default() -> DynamicInstance {
        DynamicInstance { shape: PathBuf::new(), transform: Mat4::IDENTITY, light: 1.0, nodes: Vec::new(), hidden: Vec::new(), shown: Vec::new() }
    }
}

/// Extra per-instance settings for [`Renderer::set_dynamic_with`].
#[derive(Clone, Debug, Default)]
pub struct InstanceOptions {
    /// Cast the dynamic shadow onto the terrain: the shape's posed
    /// silhouette projected along the sun onto the ground plane under it,
    /// fading with height and with the instance's `light`.
    pub shadow: bool,
    /// IFL material times in seconds (material name, with or without
    /// `.ifl`, case-insensitive). Listed IFLs show the frame at that time;
    /// the others run on the global clock. ThinkTanks drives the tread
    /// IFLs `LTread`/`Rtread` through the `LTread`/`RTread` sequences, see
    /// [`crate::pose::sequence_ifl`].
    pub ifl: Vec<(String, f32)>,
}

/// One set of moving shapes with its instance and pose buffers, reused
/// (grown as needed) across frames.
#[derive(Default)]
pub(super) struct Layer {
    pub(super) instances: Option<wgpu::Buffer>,
    pub(super) capacity: usize,
    /// Per-object pose matrices of all instances, and their bind group.
    pub(super) poses: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    pub(super) pose_capacity: usize,
    /// Mesh key and instance range per draw group.
    pub(super) draws: Vec<(PathBuf, std::ops::Range<u32>)>,
    /// Some instance casts a projected shadow.
    pub(super) shadows: bool,
    /// World position per instance (for sorting blended surfaces).
    pub(super) positions: Vec<Vec3>,
    /// Object-to-world transform per instance (detail selection).
    pub(super) transforms: Vec<Mat4>,
    /// Shadow casters of this layer: (instance index, caster record).
    pub(super) casters: Vec<(u32, ShadowCaster)>,
    /// Terrain index ranges under each caster's box: (caster, indices).
    pub(super) shadow_draws: Vec<(u32, std::ops::Range<u32>)>,
}

/// Moving shapes: meshes loaded on first use and cached by path; layer 0 is
/// [`Renderer::set_dynamic`], layer 1 the shapes of [`Renderer::set_effects`].
#[derive(Default)]
pub(super) struct Dynamic {
    /// Shape path as given -> mesh (None: failed to load; not retried).
    pub(super) meshes: HashMap<PathBuf, Option<(ShapeMesh, Rig)>>,
    pub(super) layers: [Layer; 2],
}

impl Renderer {
    /// Replaces the set of moving shapes drawn by the following frames.
    /// Shapes load on first use and stay cached by path.
    pub fn set_dynamic(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, instances: &[DynamicInstance]) {
        self.set_layer(device, queue, 0, instances, &[]);
    }

    /// [`Renderer::set_dynamic`] with per-instance [`InstanceOptions`]
    /// (`options[i]` belongs to `instances[i]`; missing = default).
    pub fn set_dynamic_with(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, instances: &[DynamicInstance], options: &[InstanceOptions]) {
        self.set_layer(device, queue, 0, instances, options);
    }

    /// Camera-facing sprites drawn by the following frames (e.g. `.png`
    /// projectiles: `Sprite { texture, pos, size: sprite_scale, ..Default::default() }`).
    pub fn set_sprites(&mut self, sprites: &[crate::sprites::Sprite]) {
        self.sprite_layers[0] = sprites.to_vec();
    }

    /// Takes over the current state of `fx` (particles, explosion sprites and
    /// shapes) for the following frames. Call after [`crate::fx::Effects::advance`].
    pub fn set_effects(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, fx: &crate::fx::Effects) {
        self.sprite_layers[1] = fx.sprites();
        let (shapes, options): (Vec<DynamicInstance>, Vec<InstanceOptions>) =
            fx.shapes_with_ifl().into_iter().map(|(inst, ifl)| (inst, InstanceOptions { shadow: false, ifl })).unzip();
        self.set_layer(device, queue, 1, &shapes, &options);
    }

    /// Takes over the moving shapes and sprite textures that `old` (a
    /// renderer of the same device) has on the GPU, when they were made with
    /// the same texture settings, so a new mission's renderer does not load
    /// and upload them again (bind group layouts of equal descriptors are
    /// one and the same in wgpu, so the old bind groups fit the new
    /// pipelines).
    pub fn reuse_assets(&mut self, old: Renderer) {
        let (a, b) = (&self.settings, &old.settings);
        if self.format != old.format || a.anisotropy != b.anisotropy || a.detail.textures() != b.detail.textures() || a.extras.fixed() != b.extras.fixed() {
            return;
        }
        let Renderer { dynamic, sprites, .. } = old;
        for (path, mesh) in dynamic.meshes {
            self.dynamic.meshes.entry(path).or_insert(mesh);
        }
        self.sprites.reuse_textures(sprites);
    }

    /// Loads moving shapes and sprite textures ahead of use (nothing is
    /// drawn), so the first frame that shows them does not stall. Also
    /// loads the shapes and sprites of every explosion in `fx`.
    pub fn preload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, shapes: &[PathBuf], sprite_textures: &[PathBuf], fx: Option<&crate::fx::FxLibrary>) {
        let mut shapes: Vec<PathBuf> = shapes.to_vec();
        let mut textures: Vec<PathBuf> = sprite_textures.to_vec();
        if let Some(lib) = fx {
            for e in &lib.explosions {
                match &e.resource {
                    crate::fx::ExplosionResource::Shape(p) => shapes.push(p.clone()),
                    crate::fx::ExplosionResource::Sprite(p) => textures.push(p.clone()),
                    _ => {}
                }
            }
            textures.extend(lib.particles.iter().map(|p| p.texture.clone()));
        }
        let sp = crate::loadstats::span(|| format!("preload: {} shapes", shapes.len()));
        // Shapes parsed and their textures decoded on all cores, then uploaded.
        shapes.sort();
        shapes.dedup();
        shapes.retain(|p| !self.dynamic.meshes.contains_key(p));
        let normals = self.shape_pipelines.normals;
        let loaded = crate::textures::parallel_map(&shapes, |p| {
            let l = ShapeSet::load_posable(p);
            // Their mip levels and normal maps too, while on all cores.
            for px in l.iter().flat_map(|(set, _)| &set.layers) {
                crate::textures::srgb_mips(px);
            }
            if normals {
                for f in l.iter().flat_map(|(set, _)| &set.layer_files).flatten() {
                    if let Some(n) = crate::textures::normal_layer(f, NORMAL_SIZE) {
                        crate::textures::mips(&n, false);
                    }
                }
            }
            l
        });
        drop(sp);
        let sp = crate::loadstats::span(|| "preload: shape uploads".into());
        for (p, l) in shapes.into_iter().zip(loaded) {
            let mesh = l.map(|(set, shape)| (ShapeMesh::new(device, queue, &self.shape_pipelines, &set), Rig::new(&shape)));
            self.dynamic.meshes.insert(p, mesh);
        }
        drop(sp);
        let _sp = crate::loadstats::span(|| format!("preload: {} sprite textures", textures.len()));
        // Those the GPU has already (a renderer's reused textures) are not decoded again.
        textures.retain(|t| !self.sprites.has_texture(t));
        let jobs: Vec<crate::textures::Job> = textures
            .iter()
            .filter_map(|t| t.parent().zip(t.file_name().and_then(|n| n.to_str())))
            .filter_map(|(dir, name)| crate::textures::image_file(dir, name))
            .collect();
        crate::textures::prefetch(&jobs);
        for t in textures {
            self.sprites.texture(device, queue, &t);
        }
        crate::loadstats::note(|| {
            let (l, n, m) = crate::textures::cache_bytes();
            format!("texture caches: layers {} MB, normal maps {} MB, mips {} MB", l >> 20, n >> 20, m >> 20)
        });
    }

    fn set_layer(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, layer: usize, instances: &[DynamicInstance], options: &[InstanceOptions]) {
        let none = InstanceOptions::default();
        let d = &mut self.dynamic;
        // Group by shape, keeping first-seen order.
        let mut order: Vec<&PathBuf> = Vec::new();
        let mut groups: HashMap<&PathBuf, Vec<(&DynamicInstance, &InstanceOptions)>> = HashMap::new();
        for (i, inst) in instances.iter().enumerate() {
            let mesh = d.meshes.entry(inst.shape.clone()).or_insert_with(|| {
                let (set, shape) = ShapeSet::load_posable(&inst.shape)?;
                Some((ShapeMesh::new(device, queue, &self.shape_pipelines, &set), Rig::new(&shape)))
            });
            if mesh.is_none() {
                continue;
            }
            groups
                .entry(&inst.shape)
                .or_insert_with(|| {
                    order.push(&inst.shape);
                    Vec::new()
                })
                .push((inst, options.get(i).unwrap_or(&none)));
        }
        let l = &mut d.layers[layer];
        let mut data = Vec::with_capacity(instances.len());
        let mut poses: Vec<[f32; 16]> = Vec::new();
        l.draws.clear();
        l.shadows = false;
        l.positions.clear();
        l.transforms.clear();
        l.casters.clear();
        l.shadow_draws.clear();
        for path in order {
            let Some(Some((mesh, rig))) = d.meshes.get(path) else { continue };
            let start = data.len() as u32;
            for &(inst, opts) in &groups[path] {
                let shadow = opts.shadow && self.settings.detail.shadows();
                let base = poses.len() as u32;
                if inst.nodes.is_empty() && inst.hidden.is_empty() && inst.shown.is_empty() && opts.ifl.is_empty() {
                    poses.extend(rig.default_objects.iter().map(|m| m.to_cols_array()));
                } else {
                    poses.extend(rig.instance_matrices(&inst.nodes, &inst.hidden, &inst.shown, &opts.ifl).iter().map(|m| m.to_cols_array()));
                }
                let node_base = base + rig.object_count() as u32;
                let ifl_base = node_base + rig.node_count() as u32;
                if shadow && !mesh.shadow.is_empty() && layer == 0 && l.casters.len() < MAX_CASTERS {
                    // Shadow::prepare: box around the shape centre, 2r across
                    // the (fixed) light direction and 10·radius along it.
                    let c = inst.transform.transform_point3(mesh.center);
                    let scale = inst.transform.x_axis.truncate().length().max(1e-3);
                    let r = mesh.bounds_radius * scale;
                    let len = SHADOW_LENGTH_FACTOR * mesh.radius * scale;
                    let improved = self.settings.extras.shadows > 0;
                    let d = if improved { sun_shadow_dir(self.sun_dir) } else { shadow_light_dir() };
                    let u = if d.z.abs() <= 0.9 { d.cross(Vec3::Z) } else { d.cross(Vec3::X) }.normalize();
                    let v = u.cross(d);
                    let ground = self.ground.height(c.x, c.y);
                    let opacity = if improved { SOFT_SHADOW_OPACITY } else { SHADOW_OPACITY } * (1.0 - (c.z - ground).max(0.0) / SHADOW_FADE_HEIGHT).clamp(0.0, 1.0) * (inst.light * 2.0).clamp(0.0, 1.0);
                    if opacity > 0.0 {
                        // Terrain cells under the part of the box that can
                        // reach the ground (down to 4 m below the lowest
                        // terrain around), as rows of the terrain mesh.
                        let low = [(-r, 0.0), (r, 0.0), (0.0, -r), (0.0, r), (0.0, 0.0)]
                            .iter()
                            .map(|(dx, dy)| self.ground.height(c.x + dx, c.y + dy))
                            .fold(f32::MAX, f32::min);
                        let reach = ((c.z - low + 4.0 + r) / -d.z).clamp(0.0, len);
                        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
                        for along in [-r, reach] {
                            for (a, b) in [(-r, -r), (-r, r), (r, -r), (r, r)] {
                                let p = (c + d * along + u * a + v * b).truncate();
                                lo = lo.min(p);
                                hi = hi.max(p);
                            }
                        }
                        let (g0x, g0y, cells) = self.terrain_grid;
                        let cell = |w: f32| ((w - self.ground.origin) / self.ground.square).floor() as i64;
                        let (i0, i1) = ((cell(lo.x) - g0x).max(0), (cell(hi.x) - g0x).min(cells - 1));
                        let (j0, j1) = ((cell(lo.y) - g0y).max(0), (cell(hi.y) - g0y).min(cells - 1));
                        let caster = l.casters.len() as u32;
                        if i0 <= i1 {
                            for j in j0..=j1 {
                                l.shadow_draws.push((caster, ((j * cells + i0) * 6) as u32..((j * cells + i1 + 1) * 6) as u32));
                            }
                        }
                        let tile = l.casters.len() as f32;
                        l.casters.push((
                            data.len() as u32,
                            ShadowCaster { center: [c.x, c.y, c.z, r], dir: [d.x, d.y, d.z, len], u: [u.x, u.y, u.z, opacity], v: [v.x, v.y, v.z, tile] },
                        ));
                        l.shadows = true;
                    }
                }
                l.positions.push(inst.transform.w_axis.truncate());
                l.transforms.push(inst.transform);
                data.push(InstanceData {
                    model: inst.transform.to_cols_array_2d(),
                    light: [inst.light, 0.0, 0.0, 0.0],
                    pose: [base, l.casters.len().saturating_sub(1) as u32, node_base, ifl_base],
                    shadow: [0.0; 4],
                });
            }
            l.draws.push((path.clone(), start..data.len() as u32));
        }
        if !l.casters.is_empty() {
            let records: Vec<ShadowCaster> = l.casters.iter().map(|c| c.1).collect();
            queue.write_buffer(&self.casters, 0, bytemuck::cast_slice(&records));
        }
        if data.is_empty() {
            return;
        }
        if l.instances.is_none() || l.capacity < data.len() {
            l.capacity = data.len().next_power_of_two().max(64);
            l.instances = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dynamic instances"),
                size: (l.capacity * std::mem::size_of::<InstanceData>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        queue.write_buffer(l.instances.as_ref().unwrap(), 0, bytemuck::cast_slice(&data));
        if poses.is_empty() {
            poses.push(Mat4::IDENTITY.to_cols_array());
        }
        if l.poses.is_none() || l.pose_capacity < poses.len() {
            l.pose_capacity = poses.len().next_power_of_two().max(1024);
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dynamic poses"),
                size: (l.pose_capacity * 64) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = pose_group(device, &self.pose_layout, &buf);
            l.poses = Some((buf, group));
        }
        queue.write_buffer(&l.poses.as_ref().unwrap().0, 0, bytemuck::cast_slice(&poses));
    }

}
