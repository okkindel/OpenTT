//! DTS shapes prepared for instanced drawing: one vertex/index buffer for
//! all shapes, one texture array layer per image.
//!
//! Every material has a timeline of texture layers sampled at 30 ticks per
//! second, so animated `.ifl` materials and static ones are drawn alike.

use crate::scene::ShapeInstance;
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tt_assets::dts::{Material, Shape};

pub const IFL_TICKS_PER_SECOND: f32 = 30.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShapeVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Start of this material's frames in [`ShapeSet::timeline`].
    pub timeline_start: u32,
    pub timeline_len: u32,
    /// Bit 0: self-illuminating.
    pub flags: u32,
    /// Index of the DTS object this vertex belongs to (selects the per-object
    /// pose matrix of posed instances).
    pub object: u32,
    /// Skinned vertices ([`SKINNED`]): up to four shape nodes and weights
    /// (weights in 1/255).
    pub bones: [u8; 4],
    pub weights: [u8; 4],
}

pub const SELF_ILLUMINATING: u32 = 1;
/// Vertex of a billboard mesh (posed shapes only): `pos` is relative to the
/// object's node and the mesh turns to face the camera.
pub const BILLBOARD: u32 = 2;
/// Billboard that only turns about the world Z axis.
pub const BILLBOARD_Z: u32 = 4;
/// Skinned vertex (posed shapes only): deformed by its `bones`.
pub const SKINNED: u32 = 8;
/// The brain jar's glass (`Brainglass`), for the glass extra.
pub const GLASS: u32 = 16;
/// Bits 8..15 of the flags: material index of an IFL material (for
/// per-instance IFL times), [`NO_IFL_SLOT`] otherwise.
pub const IFL_SLOT_SHIFT: u32 = 8;
pub const NO_IFL_SLOT: u32 = 0xff;
/// Per-instance IFL times cover material indices below this.
pub const MAX_IFL_SLOTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Blend {
    Opaque,
    Translucent,
    /// Translucent, front faces only (`Material::SINGLE_SIDED`).
    Glass,
    Additive,
}

impl Blend {
    /// The blend mode of a material with `flags`.
    pub fn of(flags: u32) -> Blend {
        if flags & Material::ADDITIVE != 0 {
            Blend::Additive
        } else if flags & (Material::TRANSLUCENT | Material::SUBTRACTIVE) != 0 {
            if flags & Material::SINGLE_SIDED != 0 { Blend::Glass } else { Blend::Translucent }
        } else {
            Blend::Opaque
        }
    }
}

/// Draw call: index range of one shape's triangles at one detail level with
/// one blend mode.
#[derive(Clone)]
pub struct ShapeBatch {
    pub indices: std::ops::Range<u32>,
    pub instances: std::ops::Range<u32>,
    pub blend: Blend,
    /// The shape: index into [`ShapeSet::lods`].
    pub group: u32,
    /// The detail level this geometry belongs to (0 = most detailed).
    pub detail: u32,
    /// The shape has glass ([`GLASS`]) surfaces.
    pub glass: bool,
}

/// One shape of a [`ShapeSet`]: its detail-level table and its instances.
#[derive(Clone)]
pub struct LodGroup {
    pub table: crate::render::DetailTable,
    pub instances: std::ops::Range<u32>,
}

/// Per-instance vertex data: object-to-world matrix and lighting.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InstanceData {
    pub model: [[f32; 4]; 4],
    /// x: diffuse scale `k` of the sun; yzw unused.
    pub light: [f32; 4],
    /// x: first per-object pose matrix of this instance in the pose buffer,
    /// [`NO_POSE`] for the default pose; yzw unused.
    pub pose: [u32; 4],
    /// Projected shadow: receiving plane `n.xyz · p = w` (n = 0: no shadow).
    /// The shadow's opacity is in `light[1]`.
    pub shadow: [f32; 4],
}

/// [`InstanceData::pose`] value for instances drawn in the default pose.
pub const NO_POSE: u32 = u32::MAX;

/// Shape-space geometry used for lighting.
pub struct ShapeGeometry {
    /// Bounds of the visible (detail 0) geometry.
    pub min: Vec3,
    pub max: Vec3,
    /// Triangles that cast the static shadow: the shape's `Shadow*` detail if
    /// it has one (trees ship a dedicated `Shadow-99` hull), else its
    /// collision detail, else detail 0. The engine builds its shadow volumes
    /// from the object's polygon list; which detail that list comes from is
    /// not recorded in the notes, so this order is our choice.
    pub shadow: Vec<[Vec3; 3]>,
}

/// Where an input [`ShapeInstance`] ended up.
#[derive(Clone, Copy)]
pub struct Placed {
    /// Index into [`ShapeSet::instances`].
    pub slot: u32,
    /// Index into [`ShapeSet::geometry`].
    pub geometry: usize,
}

pub struct ShapeSet {
    pub vertices: Vec<ShapeVertex>,
    pub indices: Vec<u32>,
    /// RGBA8 `textures::size()`^2 layers; layer 0 is plain white.
    pub layers: Vec<std::sync::Arc<Vec<u8>>>,
    /// The image file of each layer (None: the white layer).
    pub layer_files: Vec<Option<PathBuf>>,
    /// Texture layer per material tick.
    pub timeline: Vec<u32>,
    pub instances: Vec<InstanceData>,
    /// Geometry of every visible detail level of every shape.
    pub batches: Vec<ShapeBatch>,
    /// Per shape (in load order): detail table and instance range.
    pub lods: Vec<LodGroup>,
    pub geometry: Vec<ShapeGeometry>,
    /// Per input instance (same order as passed to [`ShapeSet::load`]);
    /// None if its shape failed to load.
    pub placed: Vec<Option<Placed>>,
    /// Posed shapes only ([`ShapeSet::load_posable`]): index range of the
    /// geometry that casts the dynamic shadow (not drawn visibly), and the
    /// shape's radius (`TSShape::radius`).
    pub shadow_indices: std::ops::Range<u32>,
    pub radius: f32,
    /// `TSShape::center` and half the bounds diagonal (posed shapes).
    pub center: [f32; 3],
    pub bounds_radius: f32,
}

fn dynamic_shadow_detail(shape: &Shape) -> usize {
    let find = |name: &str| shape.detail_levels.iter().position(|d| d.name.eq_ignore_ascii_case(name));
    find("shadow-99").or_else(|| find("Collision-1")).unwrap_or(0)
}

fn shadow_detail(shape: &Shape) -> usize {
    let find = |prefix: &str| shape.detail_levels.iter().position(|d| d.name.to_ascii_lowercase().starts_with(prefix));
    find("shadow").or_else(|| find("collision")).unwrap_or(0)
}

struct MaterialInfo {
    timeline_start: u32,
    timeline_len: u32,
    blend: Blend,
    flags: u32,
}

/// Image `name` next to the shape (any case, optional extension). The
/// pixels are shared with the decode cache ([`crate::textures::layer`]).
pub(crate) fn load_image_shared(dir: &Path, name: &str) -> Option<std::sync::Arc<Vec<u8>>> {
    crate::textures::layer(&crate::textures::image_file(dir, name)?).ok()
}

/// Image files the materials of `shape` (in `dir`) use, IFL frames included.
fn texture_jobs(dir: &Path, shape: &Shape) -> Vec<crate::textures::Job> {
    let mut names = Vec::new();
    for m in &shape.materials {
        let ifl = m.is_ifl() || m.flags & Material::IFL_MATERIAL != 0;
        match if ifl { ifl_frames(dir, m.texture_name()) } else { None } {
            Some(frames) => names.extend(frames.into_iter().map(|(f, _)| f)),
            None => names.push(m.texture_name().to_string()),
        }
    }
    names.iter().filter_map(|n| crate::textures::image_file(dir, n)).collect()
}

/// The image file of a layer named `name` (normal maps sit next to it).
pub(crate) fn image_file(dir: &Path, name: &str) -> Option<PathBuf> {
    crate::textures::image_file(dir, name)
}

/// Frames of an image file list: `file [ticks]` per line, ticks default 1.
fn ifl_frames(dir: &Path, name: &str) -> Option<Vec<(String, u32)>> {
    let list = tt_assets::find_ci(&dir.join(name)).or_else(|| tt_assets::find_ci(&dir.join(format!("{name}.ifl"))))?;
    let text: String = std::fs::read(list).ok()?.iter().map(|&c| c as char).collect();
    let frames: Vec<(String, u32)> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let file = it.next()?.to_string();
            let ticks = it.next().and_then(|t| t.parse().ok()).unwrap_or(1u32).max(1);
            Some((file, ticks))
        })
        .collect();
    (!frames.is_empty()).then_some(frames)
}

/// The image shown at each IFL tick of the IFL material `material` (name as
/// in the shape, with or without `.ifl`, any case) of the shape at `path`:
/// the timeline the renderer plays, indexed by `tick % len` with
/// `tick = floor(time * IFL_TICKS_PER_SECOND)`. None if it is no IFL.
pub fn ifl_timeline(path: &Path, material: &str) -> Option<Vec<PathBuf>> {
    let file = tt_assets::find_ci(path)?;
    let dir = file.parent()?;
    let shape = crate::pose::shape(path)?;
    let want = material.to_ascii_lowercase();
    let want = want.strip_suffix(".ifl").unwrap_or(&want);
    // The IFL one: health.dts also has a plain `health` material.
    let m = shape.materials.iter().find(|m| {
        let n = m.name.to_ascii_lowercase();
        n.strip_suffix(".ifl").unwrap_or(&n) == want && (m.is_ifl() || m.flags & Material::IFL_MATERIAL != 0)
    })?;
    let mut out = Vec::new();
    for (name, ticks) in ifl_frames(dir, m.texture_name())? {
        let img = image_file(dir, &name)?;
        out.extend(std::iter::repeat_n(img, ticks as usize));
    }
    Some(out)
}

struct Loader {
    set: ShapeSet,
    /// Image file (lowercased path) -> layer.
    layer_cache: HashMap<PathBuf, u32>,
}

impl Loader {
    fn layer(&mut self, dir: &Path, name: &str) -> Option<u32> {
        let key = dir.join(name.to_lowercase());
        if let Some(&l) = self.layer_cache.get(&key) {
            return Some(l);
        }
        let px = load_image_shared(dir, name)?;
        self.set.layers.push(px);
        self.set.layer_files.push(image_file(dir, name));
        let l = self.set.layers.len() as u32 - 1;
        self.layer_cache.insert(key, l);
        Some(l)
    }

    fn material(&mut self, dir: &Path, shape_path: &Path, index: u32, m: &Material) -> MaterialInfo {
        let timeline_start = self.set.timeline.len() as u32;
        let ifl = m.is_ifl() || m.flags & Material::IFL_MATERIAL != 0;
        let frames = if ifl { ifl_frames(dir, m.texture_name()) } else { None };
        match frames {
            Some(frames) => {
                for (file, ticks) in frames {
                    let layer = self.layer(dir, &file).unwrap_or(0);
                    self.set.timeline.extend(std::iter::repeat_n(layer, ticks as usize));
                }
            }
            None => {
                let layer = self.layer(dir, m.texture_name()).unwrap_or_else(|| {
                    eprintln!("warning: {}: texture {} not found", shape_path.display(), m.name);
                    0
                });
                self.set.timeline.push(layer);
            }
        }
        let blend = Blend::of(m.flags);
        MaterialInfo {
            timeline_start,
            timeline_len: self.set.timeline.len() as u32 - timeline_start,
            blend,
            flags: (if m.flags & Material::SELF_ILLUMINATING != 0 { SELF_ILLUMINATING } else { 0 })
                | (if m.name.eq_ignore_ascii_case("brainglass") { GLASS } else { 0 })
                | (if ifl && (index as usize) < MAX_IFL_SLOTS { index } else { NO_IFL_SLOT }) << IFL_SLOT_SHIFT,
        }
    }
}

impl ShapeSet {
    pub fn load(instances: &[ShapeInstance]) -> ShapeSet {
        Self::load_impl(instances, false, &mut Vec::new())
    }

    /// One shape for posed drawing: also includes objects hidden by default
    /// (their visibility is decided per instance). Returns the parsed shape
    /// too, for the node hierarchy.
    pub fn load_posable(path: &Path) -> Option<(ShapeSet, Shape)> {
        let inst = ShapeInstance { path: path.to_path_buf(), transform: Mat4::IDENTITY, casts_shadow: false, tree_lighting: false, light_boost: 0.0 };
        let mut shapes = Vec::new();
        let set = Self::load_impl(&[inst], true, &mut shapes);
        let shape = shapes.pop()?;
        (!set.batches.is_empty()).then_some((set, shape))
    }

    fn load_impl(instances_in: &[ShapeInstance], all_objects: bool, keep: &mut Vec<Shape>) -> ShapeSet {
        let mut by_path: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        let mut order = Vec::new();
        for (index, i) in instances_in.iter().enumerate() {
            let Some(p) = tt_assets::find_ci(&i.path) else {
                eprintln!("warning: shape not found: {}", i.path.display());
                continue;
            };
            by_path.entry(p.clone()).or_insert_with(|| {
                order.push(p.clone());
                Vec::new()
            });
            by_path.get_mut(&p).unwrap().push(index);
        }

        let mut ld = Loader {
            set: ShapeSet {
                vertices: Vec::new(),
                indices: Vec::new(),
                layers: vec![crate::textures::white()],
                layer_files: vec![None],
                timeline: vec![0],
                instances: Vec::new(),
                batches: Vec::new(),
                lods: Vec::new(),
                geometry: Vec::new(),
                placed: vec![None; instances_in.len()],
                shadow_indices: 0..0,
                radius: 0.0,
                center: [0.0; 3],
                bounds_radius: 0.0,
            },
            layer_cache: HashMap::new(),
        };
        // Parse every shape and decode all their textures in parallel first.
        let parsed: Vec<(PathBuf, Result<Shape, String>)> = crate::textures::parallel_map(&order, |path| {
            let shape = std::fs::read(path).map_err(|e| e.to_string()).and_then(|d| Shape::parse(&d).map_err(|e| e.to_string()));
            (path.clone(), shape)
        });
        let jobs: Vec<crate::textures::Job> = parsed.iter().filter_map(|(p, s)| Some(texture_jobs(p.parent()?, s.as_ref().ok()?))).flatten().collect();
        crate::textures::prefetch(&jobs);
        let untextured = MaterialInfo { timeline_start: 0, timeline_len: 1, blend: Blend::Opaque, flags: NO_IFL_SLOT << IFL_SLOT_SHIFT };
        for (path, shape) in parsed {
            let shape = match shape {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("warning: {}: {e}", path.display());
                    continue;
                }
            };
            let dir = path.parent().unwrap();
            let mats: Vec<MaterialInfo> = shape.materials.iter().enumerate().map(|(i, m)| ld.material(dir, &path, i as u32, m)).collect();
            let glass = mats.iter().any(|m| m.flags & GLASS != 0);

            let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let node_world = shape.node_world_transforms();
            let table = crate::render::DetailTable::of(&shape);
            let levels = table.visible_levels().max(1);
            let first_instance = ld.set.instances.len() as u32;
            let instance_count = by_path[&path].len() as u32;
            let instances = first_instance..first_instance + instance_count;
            // Index ranges per (detail level, blend) of this shape.
            let mut level_batches: Vec<Vec<(Blend, std::ops::Range<u32>)>> = Vec::with_capacity(levels);
            for detail in 0..levels {
            // Auto-billboard levels are not drawn as such: they reuse the
            // previous level.
            if detail > 0 && shape.detail_levels.get(detail).is_some_and(|d| d.sub_shape < 0) {
                let prev = level_batches[detail - 1].clone();
                level_batches.push(prev);
                continue;
            }
            let mut buckets: HashMap<Blend, Vec<u32>> = HashMap::new();
            let geometry = if all_objects { shape.detail_geometry_all(detail) } else { shape.detail_geometry(detail) };
            for rm in geometry {
                // Billboards (posed shapes only): keep node-local positions.
                let mesh_flags = shape.meshes.get(rm.mesh).map_or(0, |m| m.flags);
                let node = shape.objects.get(rm.object).and_then(|o| o.node).and_then(|n| node_world.get(n));
                let billboard = match node {
                    Some(w) if all_objects && mesh_flags & tt_assets::dts::Mesh::BILLBOARD != 0 => Some((
                        crate::pose::to_mat4(w).inverse(),
                        if mesh_flags & tt_assets::dts::Mesh::BILLBOARD_Z_AXIS != 0 { BILLBOARD | BILLBOARD_Z } else { BILLBOARD },
                    )),
                    _ => None,
                };
                if detail == 0 {
                    for p in &rm.positions {
                        min = min.min(Vec3::from(*p));
                        max = max.max(Vec3::from(*p));
                    }
                }
                let base = ld.set.vertices.len() as u32;
                let mat = rm.material.and_then(|m| mats.get(m)).unwrap_or(&untextured);
                // Skin influences (posed shapes): four strongest per vertex.
                let skin = shape.meshes.get(rm.mesh).and_then(|m| m.skin.as_ref()).filter(|_| all_objects).map(|sk| {
                    let mut inf: Vec<Vec<(u32, f32)>> = vec![Vec::new(); shape.meshes[rm.mesh].vertices.len()];
                    for ((&v, &b), &w) in sk.vertex_index.iter().zip(&sk.bone_index).zip(&sk.weight) {
                        if let (Some(list), Some(&node)) = (inf.get_mut(v as usize), sk.node_index.get(b as usize)) {
                            list.push((node, w));
                        }
                    }
                    inf
                });
                for k in 0..rm.positions.len() {
                    let (pos, mut bb) = match billboard {
                        Some((inv, f)) => (inv.transform_point3(Vec3::from(rm.positions[k])).into(), f),
                        None => (rm.positions[k], 0),
                    };
                    let (mut bones, mut weights) = ([0u8; 4], [0u8; 4]);
                    if let Some(list) = skin.as_ref().and_then(|inf| inf.get(rm.source[k] as usize)).filter(|l| !l.is_empty()) {
                        let mut list = list.clone();
                        list.sort_by(|a, b| b.1.total_cmp(&a.1));
                        list.truncate(4);
                        let sum: f32 = list.iter().map(|x| x.1).sum::<f32>().max(1e-6);
                        for (i, (node, w)) in list.iter().enumerate() {
                            bones[i] = (*node).min(255) as u8;
                            weights[i] = (w / sum * 255.0).round() as u8;
                        }
                        bb |= SKINNED;
                    }
                    ld.set.vertices.push(ShapeVertex {
                        pos,
                        normal: rm.normals.get(k).copied().unwrap_or([0.0, 0.0, 1.0]),
                        uv: rm.uvs.get(k).copied().unwrap_or([0.0, 0.0]),
                        timeline_start: mat.timeline_start,
                        timeline_len: mat.timeline_len,
                        flags: mat.flags | bb,
                        object: rm.object as u32,
                        bones,
                        weights,
                    });
                }
                buckets.entry(mat.blend).or_default().extend(rm.triangles.iter().flatten().map(|&i| base + i));
            }
            let mut ranges = Vec::new();
            for blend in [Blend::Opaque, Blend::Translucent, Blend::Glass, Blend::Additive] {
                if let Some(idx) = buckets.remove(&blend) {
                    let start = ld.set.indices.len() as u32;
                    ld.set.indices.extend(idx);
                    ranges.push((blend, start..ld.set.indices.len() as u32));
                }
            }
            level_batches.push(ranges);
            }
            if min.x > max.x {
                (min, max) = (Vec3::ZERO, Vec3::ZERO);
            }
            let shadow = shape
                .detail_geometry(shadow_detail(&shape))
                .iter()
                .flat_map(|rm| rm.triangles.iter().map(|t| t.map(|i| Vec3::from(rm.positions[i as usize]))))
                .collect();
            let geometry = ld.set.geometry.len();
            ld.set.geometry.push(ShapeGeometry { min, max, shadow });
            for &index in &by_path[&path] {
                let model: Mat4 = instances_in[index].transform;
                ld.set.placed[index] = Some(Placed { slot: ld.set.instances.len() as u32, geometry });
                ld.set.instances.push(InstanceData { model: model.to_cols_array_2d(), light: [1.0, 0.0, 0.0, 0.0], pose: [NO_POSE, 0, 0, 0], shadow: [0.0; 4] });
            }
            let group = ld.set.lods.len() as u32;
            for (detail, ranges) in level_batches.into_iter().enumerate() {
                for (blend, indices) in ranges {
                    ld.set.batches.push(ShapeBatch { indices, instances: instances.clone(), blend, group, detail: detail as u32, glass });
                }
            }
            ld.set.lods.push(LodGroup { table, instances: instances.clone() });
            if all_objects {
                // Dynamic-shadow geometry: vertices tagged with their object
                // (posed like the rest), indices outside every batch.
                let start = ld.set.indices.len() as u32;
                for rm in shape.detail_geometry(dynamic_shadow_detail(&shape)) {
                    let base = ld.set.vertices.len() as u32;
                    for &p in &rm.positions {
                        ld.set.vertices.push(ShapeVertex {
                            pos: p,
                            normal: [0.0, 0.0, 1.0],
                            uv: [0.0, 0.0],
                            timeline_start: 0,
                            timeline_len: 1,
                            flags: NO_IFL_SLOT << IFL_SLOT_SHIFT,
                            object: rm.object as u32,
                            bones: [0; 4],
                            weights: [0; 4],
                        });
                    }
                    ld.set.indices.extend(rm.triangles.iter().flatten().map(|&i| base + i));
                }
                ld.set.shadow_indices = start..ld.set.indices.len() as u32;
                ld.set.radius = shape.radius;
                ld.set.center = shape.center;
                ld.set.bounds_radius = (Vec3::from(shape.bounds.max) - Vec3::from(shape.bounds.min)).length() * 0.5;
                keep.push(shape);
            }
        }
        ld.set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A translucent material is drawn from both sides, unless it is
    /// single-sided; additive wins over both.
    #[test]
    fn blend_modes_of_material_flags() {
        assert_eq!(Blend::of(0), Blend::Opaque);
        assert_eq!(Blend::of(Material::SINGLE_SIDED), Blend::Opaque);
        assert_eq!(Blend::of(Material::TRANSLUCENT), Blend::Translucent);
        assert_eq!(Blend::of(Material::TRANSLUCENT | Material::SINGLE_SIDED), Blend::Glass);
        assert_eq!(Blend::of(Material::ADDITIVE | Material::TRANSLUCENT | Material::SINGLE_SIDED), Blend::Additive);
    }
}
