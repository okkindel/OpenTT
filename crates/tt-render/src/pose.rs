//! DTS node poses: sequence playback and per-object matrices for posed
//! dynamic instances.
//!
//! Node transforms here are *local* (relative to the parent node), as glam
//! matrices in shape units.

use glam::{Mat4, Vec3, Vec4};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tt_assets::dts::{quat_to_matrix, Sequence, Shape, Transform};

pub fn to_mat4(t: &Transform) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(t[0][0], t[1][0], t[2][0], 0.0),
        Vec4::new(t[0][1], t[1][1], t[2][1], 0.0),
        Vec4::new(t[0][2], t[1][2], t[2][2], 0.0),
        Vec4::new(t[0][3], t[1][3], t[2][3], 1.0),
    )
}

fn local(q: [f32; 4], t: [f32; 3]) -> Mat4 {
    let mut m = to_mat4(&quat_to_matrix(q));
    m.w_axis = Vec3::from(t).extend(1.0);
    m
}

/// Parsed shape by path (case-insensitive lookup), cached for the process.
pub fn shape(path: &Path) -> Option<Arc<Shape>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<Arc<Shape>>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
    // Looked up every frame per drawn shape: no key copy on a hit.
    if let Some(s) = cache.get(path) {
        return s.clone();
    }
    cache
        .entry(path.to_path_buf())
        .or_insert_with(|| {
            let file = tt_assets::find_ci(path)?;
            Shape::parse(&std::fs::read(file).ok()?).ok().map(Arc::new)
        })
        .clone()
}

/// Default local transform of a node (by name, case-insensitive).
pub fn node_default(shape: &Shape, node: &str) -> Option<Mat4> {
    let n = shape.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(node))?;
    Some(local(n.rotation, n.translation))
}

fn find_sequence<'a>(shape: &'a Shape, name: &str) -> Option<&'a Sequence> {
    shape.sequences.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// Keyframe pair and blend for time `t`: cyclic sequences wrap and blend
/// the last keyframe back into the first; others clamp to their end.
fn keys(seq: &Sequence, t: f32) -> (usize, usize, f32) {
    let n = seq.num_keyframes.max(1) as usize;
    let d = seq.duration.max(1e-6);
    if seq.flags & Sequence::CYCLIC != 0 {
        let pos = (t / d).rem_euclid(1.0) * n as f32;
        let k1 = (pos.floor() as usize).min(n - 1);
        (k1, (k1 + 1) % n, pos - k1 as f32)
    } else {
        let pos = (t / d).clamp(0.0, 1.0) * (n - 1) as f32;
        let k1 = (pos.floor() as usize).min(n - 1);
        (k1, (k1 + 1).min(n - 1), pos - k1 as f32)
    }
}

fn nlerp(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 4] {
    let dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
    let s = if dot < 0.0 { -1.0 } else { 1.0 };
    let q: [f32; 4] = std::array::from_fn(|i| a[i] * (1.0 - f) + b[i] * s * f);
    let len = q.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    q.map(|x| x / len)
}

/// Local transforms of the nodes a sequence animates, at `t` seconds, for
/// [`DynamicInstance::nodes`](crate::DynamicInstance). Rotation, translation
/// and scale keyframes are interpolated (local = T·R·S); a node animated in
/// only some of them keeps its default for the others. Blend sequences are applied as
/// absolute poses (not added on top of another sequence).
pub fn sequence_pose_of(shape: &Shape, seq_name: &str, t: f32) -> Vec<(String, Mat4)> {
    let Some(seq) = find_sequence(shape, seq_name) else { return Vec::new() };
    let (k1, k2, f) = keys(seq, t);
    let nk = seq.num_keyframes.max(0) as usize;
    // Per animated node: rotation, translation, scale (local = T·R·S).
    let mut nodes: HashMap<usize, ([f32; 4], [f32; 3], Mat4)> = HashMap::new();
    for (j, &node) in seq.rotation_matters.iter().enumerate() {
        let base = seq.base_rotation.max(0) as usize + j * nk;
        let (Some(a), Some(b), Some(n)) = (shape.node_rotations.get(base + k1), shape.node_rotations.get(base + k2), shape.nodes.get(node)) else {
            continue;
        };
        nodes.entry(node).or_insert((n.rotation, n.translation, Mat4::IDENTITY)).0 = nlerp(*a, *b, f);
    }
    for (j, &node) in seq.translation_matters.iter().enumerate() {
        let base = seq.base_translation.max(0) as usize + j * nk;
        let (Some(a), Some(b), Some(n)) = (shape.node_translations.get(base + k1), shape.node_translations.get(base + k2), shape.nodes.get(node)) else {
            continue;
        };
        nodes.entry(node).or_insert((n.rotation, n.translation, Mat4::IDENTITY)).1 = std::array::from_fn(|i| a[i] * (1.0 - f) + b[i] * f);
    }
    // Scale tracks: the sequence flag says which keyframe array holds
    // them. Explosion shapes grow this way.
    let lerp3 = |a: [f32; 3], b: [f32; 3]| Vec3::from(a).lerp(Vec3::from(b), f);
    for (j, &node) in seq.scale_matters.iter().enumerate() {
        let k = seq.base_scale.max(0) as usize + j * nk;
        let scale = if seq.flags & Sequence::UNIFORM_SCALE != 0 {
            match (shape.node_uniform_scales.get(k + k1), shape.node_uniform_scales.get(k + k2)) {
                (Some(a), Some(b)) => Some(Mat4::from_scale(Vec3::splat(a + (b - a) * f))),
                _ => None,
            }
        } else if seq.flags & Sequence::ALIGNED_SCALE != 0 {
            match (shape.node_aligned_scales.get(k + k1), shape.node_aligned_scales.get(k + k2)) {
                (Some(a), Some(b)) => Some(Mat4::from_scale(lerp3(*a, *b))),
                _ => None,
            }
        } else if seq.flags & Sequence::ARBITRARY_SCALE != 0 {
            let fs = (shape.node_arbitrary_scale_factors.get(k + k1), shape.node_arbitrary_scale_factors.get(k + k2));
            let rs = (shape.node_arbitrary_scale_rotations.get(k + k1), shape.node_arbitrary_scale_rotations.get(k + k2));
            match (fs, rs) {
                ((Some(a), Some(b)), (Some(qa), Some(qb))) => {
                    let r = to_mat4(&quat_to_matrix(nlerp(*qa, *qb, f)));
                    Some(r * Mat4::from_scale(lerp3(*a, *b)) * r.transpose())
                }
                _ => None,
            }
        } else {
            None
        };
        if let (Some(scale), Some(n)) = (scale, shape.nodes.get(node)) {
            nodes.entry(node).or_insert((n.rotation, n.translation, Mat4::IDENTITY)).2 = scale;
        }
    }
    let mut out: Vec<(String, Mat4)> = nodes.into_iter().map(|(n, (q, tr, sc))| (shape.nodes[n].name.clone(), local(q, tr) * sc)).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// [`sequence_pose_of`] for a shape file (parsed once and cached).
pub fn sequence_pose(shape: &Path, seq_name: &str, t: f32) -> Vec<(String, Mat4)> {
    self::shape(shape).map(|s| sequence_pose_of(&s, seq_name, t)).unwrap_or_default()
}

/// IFL times a sequence sets at `t` seconds: `(IFL material name, time)`
/// for the IFL materials it controls, for [`crate::InstanceOptions::ifl`].
pub fn sequence_ifl_of(shape: &Shape, seq_name: &str, t: f32) -> Vec<(String, f32)> {
    let Some(seq) = find_sequence(shape, seq_name) else { return Vec::new() };
    let d = seq.duration.max(1e-6);
    let pos_time = if seq.flags & Sequence::CYCLIC != 0 { t.rem_euclid(d) } else { t.clamp(0.0, d) };
    // AnimateIfls reads the IFL at `thread.pos * seq.duration +
    // seq.toolBegin`, not at the bare thread time: Think Tanks' IFLs are
    // laid out on the exporter's timeline (`RTread`'s frames sit at 0.333 s,
    // after an 11-tick hold).
    let time = pos_time + seq.tool_begin.max(0.0);
    seq.ifl_matters
        .iter()
        .filter_map(|&i| shape.ifl_materials.get(i))
        .filter_map(|ifl| shape.materials.get(ifl.material_slot).map(|m| (m.name.clone(), time)))
        .collect()
}

pub fn sequence_ifl(shape: &Path, seq_name: &str, t: f32) -> Vec<(String, f32)> {
    self::shape(shape).map(|s| sequence_ifl_of(&s, seq_name, t)).unwrap_or_default()
}

/// Object visibility a sequence sets at `t`: `(object name, visible)` for
/// every object whose visibility it animates (for `hidden` / `shown`).
pub fn sequence_visibility_of(shape: &Shape, seq_name: &str, t: f32) -> Vec<(String, bool)> {
    sequence_visibility_level_of(shape, seq_name, t).into_iter().map(|(n, v)| (n, v > 0.0)).collect()
}

/// Interpolated visibility (0..1) a sequence gives each object it animates
/// at `t`.
pub fn sequence_visibility_level_of(shape: &Shape, seq_name: &str, t: f32) -> Vec<(String, f32)> {
    let Some(seq) = find_sequence(shape, seq_name) else { return Vec::new() };
    let (k1, k2, f) = keys(seq, t);
    let nk = seq.num_keyframes.max(0) as usize;
    seq.vis_matters
        .iter()
        .enumerate()
        .filter_map(|(j, &obj)| {
            let base = seq.base_object_state.max(0) as usize + j * nk;
            let (a, b) = (shape.object_states.get(base + k1)?, shape.object_states.get(base + k2)?);
            Some((shape.objects.get(obj)?.name.clone(), a * (1.0 - f) + b * f))
        })
        .collect()
}

pub fn sequence_visibility(shape: &Path, seq_name: &str, t: f32) -> Vec<(String, bool)> {
    self::shape(shape).map(|s| sequence_visibility_of(&s, seq_name, t)).unwrap_or_default()
}

pub fn sequence_visibility_level(shape: &Path, seq_name: &str, t: f32) -> Vec<(String, f32)> {
    self::shape(shape).map(|s| sequence_visibility_level_of(&s, seq_name, t)).unwrap_or_default()
}

/// Shape-space transform of `node` with local overrides `nodes` applied
/// (e.g. a muzzle position: `instance.transform * node_transform(..)`).
pub fn node_transform(shape: &Shape, nodes: &[(String, Mat4)], node: &str) -> Option<Mat4> {
    let mut i = shape.nodes.iter().position(|n| n.name.eq_ignore_ascii_case(node))?;
    let mut m = Mat4::IDENTITY;
    for _ in 0..shape.nodes.len() {
        let n = &shape.nodes[i];
        let local = nodes.iter().rev().find(|(name, _)| name.eq_ignore_ascii_case(&n.name)).map_or_else(|| local(n.rotation, n.translation), |o| o.1);
        m = local * m;
        match n.parent {
            Some(p) if p < shape.nodes.len() && p != i => i = p,
            _ => break,
        }
    }
    Some(m)
}

/// Per-shape data for building per-object pose matrices quickly.
pub(crate) struct Rig {
    names: Vec<String>,
    parents: Vec<Option<usize>>,
    /// Default local transforms.
    locals: Vec<Mat4>,
    /// Nodes ordered so that parents come before children.
    order: Vec<usize>,
    /// Inverse default world transform per node.
    inv_world: Vec<Mat4>,
    /// Node and default visibility per object (None node: skinned).
    objects: Vec<(Option<usize>, bool, String)>,
    /// Objects whose detail-0 mesh is a billboard (vertices node-local).
    billboard: Vec<bool>,
    /// Per-object matrices of the default pose (hidden objects collapsed).
    pub(crate) default_objects: Vec<Mat4>,
    /// Lower-case material names (for IFL times by name).
    materials: Vec<String>,
}

impl Rig {
    pub(crate) fn new(shape: &Shape) -> Rig {
        let n = shape.nodes.len();
        let names = shape.nodes.iter().map(|n| n.name.to_ascii_lowercase()).collect();
        let parents: Vec<Option<usize>> = shape.nodes.iter().enumerate().map(|(i, node)| node.parent.filter(|&p| p < n && p != i)).collect();
        let locals = shape.nodes.iter().map(|n| local(n.rotation, n.translation)).collect();
        // Depth-first order from the roots; nodes in a parent cycle are
        // appended as roots.
        let mut order = Vec::with_capacity(n);
        let mut seen = vec![false; n];
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, p) in parents.iter().enumerate() {
            if let Some(p) = p {
                children[*p].push(i);
            }
        }
        let mut stack: Vec<usize> = (0..n).filter(|&i| parents[i].is_none()).rev().collect();
        while let Some(i) = stack.pop() {
            if seen[i] {
                continue;
            }
            seen[i] = true;
            order.push(i);
            stack.extend(children[i].iter().rev());
        }
        order.extend((0..n).filter(|&i| !seen[i]));
        let inv_world = shape.node_world_transforms().iter().map(|t| to_mat4(t).inverse()).collect();
        let objects = shape.objects.iter().map(|o| (o.node.filter(|&i| i < n), o.default_visibility > 0.0, o.name.to_ascii_lowercase())).collect();
        let billboard = (0..shape.objects.len())
            .map(|o| shape.detail_mesh(0, o).and_then(|m| shape.meshes.get(m)).is_some_and(|m| m.flags & tt_assets::dts::Mesh::BILLBOARD != 0))
            .collect();
        let materials = shape.materials.iter().map(|m| m.name.to_ascii_lowercase()).collect();
        let mut rig = Rig { names, parents, locals, order, inv_world, objects, billboard, default_objects: Vec::new(), materials };
        rig.default_objects = rig.instance_matrices(&[], &[], &[], &[]);
        rig
    }

    /// Per-object matrices taking the default-pose shape-space geometry to
    /// the posed shape space; hidden objects get a zero matrix.
    pub(crate) fn object_count(&self) -> usize {
        self.objects.len()
    }

    pub(crate) fn node_count(&self) -> usize {
        self.locals.len()
    }

    /// Everything a posed instance needs in the pose buffer: one matrix per
    /// object (node pose relative to the default; zero when hidden), one delta per node (default to
    /// posed, for skinned vertices), then one matrix holding 16 IFL tick
    /// values by material index (-1: the global clock).
    pub(crate) fn instance_matrices(&self, nodes: &[(String, Mat4)], hidden: &[String], shown: &[String], ifl: &[(String, f32)]) -> Vec<Mat4> {
        let world = self.world(nodes);
        let mut out = self.objects_from(&world, hidden, shown);
        out.extend(world.iter().zip(&self.inv_world).map(|(w, inv)| *w * *inv));
        let mut ticks = [-1.0f32; crate::shapes::MAX_IFL_SLOTS];
        for (name, t) in ifl {
            let name = name.to_ascii_lowercase();
            let name = name.strip_suffix(".ifl").unwrap_or(&name);
            // Every material of that name (health.dts has a plain `health`
            // before its IFL `health`; only IFL slots read the value).
            for (i, m) in self.materials.iter().enumerate().take(ticks.len()) {
                if m.strip_suffix(".ifl").unwrap_or(m) == name {
                    ticks[i] = (t.max(0.0) * crate::shapes::IFL_TICKS_PER_SECOND).floor();
                }
            }
        }
        out.push(Mat4::from_cols_array(&ticks));
        out
    }

    fn world(&self, nodes: &[(String, Mat4)]) -> Vec<Mat4> {
        let mut locals = self.locals.clone();
        for (name, m) in nodes {
            if let Some(i) = self.names.iter().position(|n| n.eq_ignore_ascii_case(name)) {
                locals[i] = *m;
            }
        }
        let mut world = vec![Mat4::IDENTITY; locals.len()];
        for &i in &self.order {
            world[i] = match self.parents[i] {
                Some(p) => world[p] * locals[i],
                None => locals[i],
            };
        }
        world
    }

    fn objects_from(&self, world: &[Mat4], hidden: &[String], shown: &[String]) -> Vec<Mat4> {
        self.objects
            .iter()
            .zip(&self.billboard)
            .map(|((node, vis, name), &billboard)| {
                let named = |list: &[String]| list.iter().any(|h| h.eq_ignore_ascii_case(name));
                let visible = if named(hidden) { false } else { *vis || named(shown) };
                if !visible {
                    Mat4::ZERO
                } else {
                    // Billboard vertices are node-local: give the node itself.
                    node.map_or(Mat4::IDENTITY, |n| if billboard { world[n] } else { world[n] * self.inv_world[n] })
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyclic_keys_wrap() {
        let seq = Sequence {
            name: String::new(),
            flags: Sequence::CYCLIC,
            num_keyframes: 4,
            duration: 1.0,
            priority: 0,
            first_ground_frame: 0,
            num_ground_frames: 0,
            base_rotation: 0,
            base_translation: 0,
            base_scale: 0,
            base_object_state: 0,
            base_decal_state: 0,
            first_trigger: 0,
            num_triggers: 0,
            tool_begin: 0.0,
            rotation_matters: Vec::new(),
            translation_matters: Vec::new(),
            scale_matters: Vec::new(),
            vis_matters: Vec::new(),
            ifl_matters: Vec::new(),
            frame_matters: Vec::new(),
            mat_frame_matters: Vec::new(),
        };
        let (a, b, f) = keys(&seq, 0.875);
        assert_eq!((a, b), (3, 0));
        assert!((f - 0.5).abs() < 1e-5);
        let (a, b, _) = keys(&seq, 1.25);
        assert_eq!((a, b), (1, 2));
    }

    /// Texture layer the shader picks for the vertices of IFL material
    /// `material` of a posed instance with IFL times `ifl` (vs_shape: the
    /// per-instance tick indexes the material's timeline).
    fn ifl_layer(set: &crate::shapes::ShapeSet, rig: &Rig, material: &str, ifl: &[(String, f32)]) -> u32 {
        use crate::shapes::{IFL_SLOT_SHIFT, NO_IFL_SLOT};
        let slot = rig.materials.iter().position(|m| m.strip_suffix(".ifl").unwrap_or(m) == material).expect("material") as u32;
        let v = set.vertices.iter().find(|v| (v.flags >> IFL_SLOT_SHIFT) & 0xff == slot).expect("vertex of the IFL material");
        assert_ne!(slot, NO_IFL_SLOT);
        let ticks = rig.instance_matrices(&[], &[], &[], ifl).last().unwrap().to_cols_array();
        let tick = ticks[slot as usize];
        assert!(tick >= 0.0, "per-instance IFL tick set");
        set.timeline[(v.timeline_start + tick as u32 % v.timeline_len.max(1)) as usize]
    }

    /// Tank treads: the `LTread`/`RTread` sequences drive the `LTread` /
    /// `Rtread` IFLs per instance; different thread positions show different
    /// tread images (`RTread` starts at its toolBegin, 0.333 s, so its first
    /// tick is not the IFL's 11-tick hold of frame 1).
    #[test]
    fn tread_ifl_frames_follow_the_sequence_time() {
        tt_assets::require_game_data!();
        let path = tt_assets::layout::default_dir().join("models/tanks/tank01.dts");
        let (set, shape) = crate::shapes::ShapeSet::load_posable(&path).expect("tank01");
        let rig = Rig::new(&shape);
        for (seq, mat) in [("LTread", "ltread"), ("RTread", "rtread")] {
            let d = shape.sequences.iter().find(|s| s.name == seq).unwrap().duration;
            // One IFL frame per 1/30 s of sequence time (7 frames per cycle).
            let layers: Vec<u32> = (0..7).map(|k| ifl_layer(&set, &rig, mat, &sequence_ifl_of(&shape, seq, (k as f32 + 0.5) / 7.0 * d))).collect();
            let mut distinct = layers.clone();
            distinct.sort();
            distinct.dedup();
            assert_eq!(distinct.len(), 7, "{seq}: seven tread images over one cycle, got {layers:?}");
            // The material name of the sequence's IFL matches either case.
            assert!(!sequence_ifl_of(&shape, seq, 0.0).is_empty());
        }
    }
}
