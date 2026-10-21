//! All values are little endian.
//! ```text
//! u16 version, u16 exporter_version
//! u32 size_mem_buffer           in 32-bit words
//! u32 start_u16, u32 start_u8   word offsets of the 16- and 8-bit streams
//! u32 mem_buffer[size_mem_buffer]
//! u32 num_sequences, Sequence[num_sequences]
//! MaterialList
//! ```
//! The memory buffer holds three interleaved streams: 32-bit words
//! `[0, start_u16)`, 16-bit halves `[start_u16, start_u8)` and bytes
//! `[start_u8, size)`. Every logical value is read from the stream of its
//! width. At fixed points a "guard" is read from each of the three streams;
//! guard *n* must equal *n* in all of them (0, 1, 2, ... with independent
//! counters), which makes the parse self-checking.
//!
//! Stream contents, `G` marks a guard:
//! ```text
//! 32: num_nodes, num_objects, num_decals, num_sub_shapes, num_ifl_materials,
//!     num_node_rots, num_node_trans, num_uniform_scales, num_aligned_scales,
//!     num_arbitrary_scales, num_ground_frames, num_object_states,
//!     num_decal_states, num_triggers, num_details, num_meshes, num_names,
//!     smallest_visible_size (int), smallest_visible_dl                  G
//! 32: f32 radius, tube_radius, center[3], bounds min[3] max[3]          G
//! 32: Node[num_nodes] {name, parent, first_object, first_child, next_sibling} G
//! 32: Object[num_objects] {name, num_meshes, start_mesh, node, next_sibling,
//!                          first_decal}                                 G
//! 32: Decal[num_decals] (5 words, deprecated)                           G
//! 32: IflMaterial[n] {name, material_slot, first_frame, time, num_frames} G
//! 32: sub_shape first_node[], first_object[], first_decal[]             G
//! 32: sub_shape num_nodes[], num_objects[], num_decals[]                G
//! 16: default rotations Quat16[num_nodes] (x,y,z,w / 32767)
//! 32: default translations [f32;3][num_nodes]
//! 32: node_translations[num_node_trans]   16: node_rotations Quat16[..]  G
//! 32: uniform scales, aligned scales[3], arbitrary scale factors[3]
//! 16: arbitrary scale rotations Quat16                                   G
//! 32: ground translations[3]  16: ground rotations Quat16               G
//! 32: ObjectState[num_object_states] {f32 vis, frame, mat_frame}       G
//! 32: decal states[num_decal_states]                                    G
//! 32: triggers[num_triggers] {state, f32 pos}                           G
//! 32: Detail[num_details] {name, sub_shape, object_detail, f32 size,
//!                          f32 avg_error, f32 max_error, poly_count}    G
//! num_meshes x { 32: mesh type; Mesh (see below) }                     G
//!  8: num_names zero-terminated strings                                 G
//! ```
//! Mesh (TSMesh::assemble), types 0 standard, 1 skin, 2 decal, 3 sorted, 4 null
//! (null meshes have no data at all):
//! ```text
//! G 32: num_frames, num_mat_frames, parent_mesh, f32 bounds[6], f32 center[3],
//!       f32 radius
//! 32: num_verts, [f32;3][num_verts]*        * only when parent_mesh < 0,
//! 32: num_tverts, [f32;2][num_tverts]*        otherwise shared with the parent
//! 32: [f32;3][num_verts]* normals   8: u8[num_verts]* encoded normals
//! 32: num_prims; 16: {start, num_elements}[num_prims]; 32: mat[num_prims]
//! 32: num_indices; 16: u16[num_indices]
//! 32: num_merge; 16: merge indices[num_merge] (deprecated)
//! 32: verts_per_frame, flags                                            G
//! skin: 32: n, initial verts[3*n]*, normals[3*n]*; 8: encoded[n]*
//!       32: n, initial transforms [f32;16][n]*
//!       32: n, vertex_index[n]*, bone_index[n]*, f32 weight[n]*
//!       32: n, node_index[n]*                                           G
//! sorted: 32: clusters (8 words each), start_cluster, first_verts,
//!         num_verts, first_tverts (each count-prefixed), always_write_depth G
//! decal: 32: n; {16: start, count; 32: mat}[n]; 32: m; 16: idx[m];
//!        32: k, start_prim[k], texgen_s[4k], texgen_t[4k], material    G
//! ```
//! Version 25 additionally stores one extra (ignored) u32 per material.

pub use crate::bytes::Error;
use crate::bytes::Reader;

fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

pub type Transform = [[f32; 4]; 3];

pub const IDENTITY: Transform = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    pub const EMPTY: Bounds = Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };

    pub fn extend(&mut self, p: [f32; 3]) {
        for (i, c) in p.into_iter().enumerate() {
            self.min[i] = self.min[i].min(c);
            self.max[i] = self.max[i].max(c);
        }
    }

    pub fn is_empty(&self) -> bool {
        (0..3).any(|i| self.min[i] > self.max[i])
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    /// Default translation relative to the parent node.
    pub translation: [f32; 3],
    /// Default rotation relative to the parent, `[x, y, z, w]`.
    pub rotation: [f32; 4],
}

impl Node {
    /// Local transform (relative to the parent) in the default pose.
    pub fn local_transform(&self) -> Transform {
        let mut m = quat_to_matrix(self.rotation);
        for (row, t) in m.iter_mut().zip(self.translation) {
            row[3] = t;
        }
        m
    }
}

#[derive(Debug, Clone)]
pub struct Object {
    pub name: String,
    /// Node the object's meshes are attached to (None for skinned objects).
    pub node: Option<usize>,
    /// Meshes `first_mesh .. first_mesh + num_meshes`, one per object detail.
    pub first_mesh: usize,
    pub num_meshes: usize,
    /// Visibility in the default state (objects with 0 are hidden).
    pub default_visibility: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct SubShape {
    pub first_node: usize,
    pub num_nodes: usize,
    pub first_object: usize,
    pub num_objects: usize,
}

#[derive(Debug, Clone)]
pub struct DetailLevel {
    pub name: String,
    /// Negative for auto-billboard details.
    pub sub_shape: i32,
    /// Index into each object's mesh list (or packed billboard options).
    pub object_detail: i32,
    /// Pixel size at which this detail kicks in; negative for collision etc.
    pub size: f32,
    pub average_error: f32,
    pub max_error: f32,
    pub poly_count: i32,
}

#[derive(Debug, Clone)]
pub struct IflMaterial {
    pub name: String,
    pub material_slot: usize,
    pub first_frame: i32,
    pub time: i32,
    pub num_frames: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshKind {
    Standard,
    Skin,
    Decal,
    Sorted,
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveKind {
    Triangles,
    Strip,
    Fan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Primitive {
    pub start: u32,
    pub num_elements: u32,
    /// Material index, None when the NoMaterial flag is set.
    pub material: Option<u32>,
    pub kind: PrimitiveKind,
    /// Raw `matIndex` word (material | flags).
    pub raw: u32,
}

impl Primitive {
    pub const STRIP: u32 = 1 << 30;
    pub const FAN: u32 = 2 << 30;
    pub const TYPE_MASK: u32 = 3 << 30;
    pub const INDEXED: u32 = 1 << 29;
    pub const NO_MATERIAL: u32 = 1 << 28;
    pub const MATERIAL_MASK: u32 = (1 << 28) - 1;

    fn from_raw(start: u32, num_elements: u32, raw: u32) -> Primitive {
        let kind = match raw & Self::TYPE_MASK {
            0 => PrimitiveKind::Triangles,
            Self::STRIP => PrimitiveKind::Strip,
            Self::FAN => PrimitiveKind::Fan,
            _ => PrimitiveKind::Strip,
        };
        let material = (raw & Self::NO_MATERIAL == 0).then_some(raw & Self::MATERIAL_MASK);
        Primitive {
            start,
            num_elements,
            material,
            kind,
            raw,
        }
    }

    /// Appends this primitive's triangles.
    pub fn triangles_into(&self, indices: &[u32], out: &mut Vec<[u32; 3]>) {
        let s = self.start as usize;
        let Some(idx) = indices.get(s..s + self.num_elements as usize) else {
            return;
        };
        match self.kind {
            PrimitiveKind::Triangles => {
                out.extend_from_slice(idx.as_chunks::<3>().0);
            }
            PrimitiveKind::Strip => {
                for k in 0..idx.len().saturating_sub(2) {
                    let t = if k % 2 == 0 {
                        [idx[k], idx[k + 1], idx[k + 2]]
                    } else {
                        [idx[k + 1], idx[k], idx[k + 2]]
                    };
                    if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                        out.push(t);
                    }
                }
            }
            PrimitiveKind::Fan => {
                for k in 1..idx.len().saturating_sub(1) {
                    let t = [idx[0], idx[k], idx[k + 1]];
                    if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                        out.push(t);
                    }
                }
            }
        }
    }
}

/// Skinning data of a skin mesh.
#[derive(Debug, Clone, Default)]
pub struct Skin {
    /// Inverse bind matrices, 16 floats each, row-major.
    pub initial_transforms: Vec<[f32; 16]>,
    /// Parallel influence arrays: vertex, bone (index into `node_index`), weight.
    pub vertex_index: Vec<u32>,
    pub bone_index: Vec<u32>,
    pub weight: Vec<f32>,
    /// Shape node for each bone.
    pub node_index: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Mesh {
    pub kind: MeshKind,
    pub num_frames: u32,
    pub num_mat_frames: u32,
    /// Mesh whose vertex data this mesh shares (data is copied here anyway).
    pub parent_mesh: Option<usize>,
    pub bounds: Bounds,
    pub center: [f32; 3],
    pub radius: f32,
    /// Vertices of all frames (`verts_per_frame` each). For skin meshes these
    /// are the initial (bind pose) vertices.
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Encoded normals.
    pub encoded_normals: Vec<u8>,
    pub tverts: Vec<[f32; 2]>,
    pub primitives: Vec<Primitive>,
    pub indices: Vec<u32>,
    pub verts_per_frame: u32,
    pub flags: u32,
    pub skin: Option<Skin>,
}

impl Mesh {
    pub const BILLBOARD: u32 = 1 << 31;
    pub const HAS_DETAIL_TEXTURE: u32 = 1 << 30;
    pub const BILLBOARD_Z_AXIS: u32 = 1 << 29;
    pub const USE_ENCODED_NORMALS: u32 = 1 << 28;

    fn empty(kind: MeshKind) -> Mesh {
        Mesh {
            kind,
            num_frames: 0,
            num_mat_frames: 0,
            parent_mesh: None,
            bounds: Bounds::EMPTY,
            center: [0.0; 3],
            radius: 0.0,
            vertices: Vec::new(),
            normals: Vec::new(),
            encoded_normals: Vec::new(),
            tverts: Vec::new(),
            primitives: Vec::new(),
            indices: Vec::new(),
            verts_per_frame: 0,
            flags: 0,
            skin: None,
        }
    }

    /// All primitives expanded to a triangle list (frame-relative indices).
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        let mut out = Vec::new();
        for p in &self.primitives {
            p.triangles_into(&self.indices, &mut out);
        }
        out
    }

    /// Number of vertices in frame 0.
    pub fn frame_vertex_count(&self) -> usize {
        match self.verts_per_frame as usize {
            0 => self.vertices.len(),
            n => n.min(self.vertices.len()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Material {
    /// Name as stored (texture file name without extension, or `*.ifl`).
    pub name: String,
    pub flags: u32,
    /// Reflectance / bump / detail map material indices (None = 0xFFFFFFFF).
    pub reflectance_map: Option<u32>,
    pub bump_map: Option<u32>,
    pub detail_map: Option<u32>,
    pub detail_scale: f32,
    pub reflection_amount: f32,
}

impl Material {
    pub const S_WRAP: u32 = 1 << 0;
    pub const T_WRAP: u32 = 1 << 1;
    pub const TRANSLUCENT: u32 = 1 << 2;
    pub const ADDITIVE: u32 = 1 << 3;
    pub const SUBTRACTIVE: u32 = 1 << 4;
    pub const SELF_ILLUMINATING: u32 = 1 << 5;
    pub const NEVER_ENV_MAP: u32 = 1 << 6;
    pub const NO_MIP_MAP: u32 = 1 << 7;
    pub const MIP_MAP_ZERO_BORDER: u32 = 1 << 8;
    pub const IFL_MATERIAL: u32 = 1 << 27;
    pub const IFL_FRAME: u32 = 1 << 28;
    pub const DETAIL_MAP_ONLY: u32 = 1 << 29;
    pub const BUMP_MAP_ONLY: u32 = 1 << 30;
    pub const REFLECTANCE_MAP_ONLY: u32 = 1 << 31;

    /// Base file name with any directory part stripped.
    pub fn texture_name(&self) -> &str {
        self.name.rsplit(['/', '\\']).next().unwrap_or(&self.name)
    }

    pub fn is_ifl(&self) -> bool {
        self.name.to_ascii_lowercase().ends_with(".ifl")
    }
}

#[derive(Debug, Clone)]
pub struct Sequence {
    pub name: String,
    pub flags: u32,
    pub num_keyframes: i32,
    pub duration: f32,
    pub priority: i32,
    pub first_ground_frame: i32,
    pub num_ground_frames: i32,
    pub base_rotation: i32,
    pub base_translation: i32,
    pub base_scale: i32,
    pub base_object_state: i32,
    pub base_decal_state: i32,
    pub first_trigger: i32,
    pub num_triggers: i32,
    pub tool_begin: f32,
    /// Nodes whose rotation / translation the sequence animates, ascending.
    /// The k-th member's keyframes are at `base_rotation` (`base_translation`)
    /// `+ k * num_keyframes` in [`Shape::node_rotations`] ([`Shape::node_translations`]).
    pub rotation_matters: Vec<usize>,
    pub translation_matters: Vec<usize>,
    /// Nodes whose scale it animates; the k-th member's keyframes are at
    /// `base_scale + k * num_keyframes` in the scale array of its kind.
    pub scale_matters: Vec<usize>,
    /// Objects whose visibility it animates; the k-th member's states are at
    /// `base_object_state + k * num_keyframes` in [`Shape::object_states`].
    pub vis_matters: Vec<usize>,
    /// Objects whose IFL, vertex frame and material (UV) frame it animates.
    pub ifl_matters: Vec<usize>,
    pub frame_matters: Vec<usize>,
    pub mat_frame_matters: Vec<usize>,
}

impl Sequence {
    pub const UNIFORM_SCALE: u32 = 1 << 0;
    pub const ALIGNED_SCALE: u32 = 1 << 1;
    pub const ARBITRARY_SCALE: u32 = 1 << 2;
    pub const BLEND: u32 = 1 << 3;
    pub const CYCLIC: u32 = 1 << 4;
    pub const MAKE_PATH: u32 = 1 << 5;
    pub const IFL_INIT: u32 = 1 << 6;
    pub const HAS_TRANSLUCENCY: u32 = 1 << 7;
}

/// Geometry of one (mesh, material) pair, ready for rendering, in shape space.
#[derive(Debug, Clone)]
pub struct RenderMesh {
    pub object: usize,
    pub mesh: usize,
    /// Index into [`Shape::materials`]; None for primitives without material.
    pub material: Option<usize>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Index of each vertex in the source mesh (e.g. for skin weights).
    pub source: Vec<u32>,
    pub triangles: Vec<[u32; 3]>,
}

#[derive(Debug, Clone)]
pub struct Shape {
    pub version: u16,
    pub exporter_version: u16,
    pub radius: f32,
    pub tube_radius: f32,
    pub center: [f32; 3],
    pub bounds: Bounds,
    pub smallest_visible_size: i32,
    pub smallest_visible_dl: i32,
    pub nodes: Vec<Node>,
    pub objects: Vec<Object>,
    pub sub_shapes: Vec<SubShape>,
    pub ifl_materials: Vec<IflMaterial>,
    pub detail_levels: Vec<DetailLevel>,
    pub meshes: Vec<Mesh>,
    pub names: Vec<String>,
    pub sequences: Vec<Sequence>,
    /// Animation keyframes of all sequences.
    pub node_rotations: Vec<[f32; 4]>,
    pub node_translations: Vec<[f32; 3]>,
    /// Scale keyframes; which array a sequence uses depends on its
    /// `UNIFORM_SCALE` / `ALIGNED_SCALE` / `ARBITRARY_SCALE` flag.
    pub node_uniform_scales: Vec<f32>,
    pub node_aligned_scales: Vec<[f32; 3]>,
    pub node_arbitrary_scale_factors: Vec<[f32; 3]>,
    pub node_arbitrary_scale_rotations: Vec<[f32; 4]>,
    /// Ground transform translations (a sequence's `first_ground_frame`
    /// .. `+ num_ground_frames`): how far the shape travels over the
    /// sequence (tank treads: ground distance per tread cycle).
    pub ground_translations: Vec<[f32; 3]>,
    /// Object visibility per object state (the first `objects.len()` are the
    /// default states, then sequence keyframes).
    pub object_states: Vec<f32>,
    /// Vertex frame and material (UV) frame per object state.
    pub object_state_frames: Vec<i32>,
    pub object_state_mat_frames: Vec<i32>,
    pub materials: Vec<Material>,
    /// Number of three-stream guard checkpoints verified while parsing.
    pub checkpoints: u32,
}

// ---------------------------------------------------------------------------
// Readers

/// The three-stream memory buffer (TSShapeAlloc in read mode).
struct Alloc<'a> {
    s32: Reader<'a>,
    s16: Reader<'a>,
    s8: Reader<'a>,
    guard: u32,
}

impl<'a> Alloc<'a> {
    fn u32(&mut self) -> Result<u32, Error> {
        self.s32
            .u32()
            .map_err(|e| Error(format!("32-bit stream: {e}")))
    }

    fn i32(&mut self) -> Result<i32, Error> {
        Ok(self.u32()? as i32)
    }

    fn f32(&mut self) -> Result<f32, Error> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn count(&mut self, what: &str) -> Result<usize, Error> {
        let n = self.i32()?;
        // Every counted item takes at least one byte, which bounds sane counts.
        if n < 0 || n as usize > self.s32.d.len() + self.s16.d.len() + self.s8.d.len() {
            return err(format!("implausible {what} count {n}"));
        }
        Ok(n as usize)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        self.s16
            .u16()
            .map_err(|e| Error(format!("16-bit stream: {e}")))
    }

    fn u8(&mut self) -> Result<u8, Error> {
        self.s8
            .u8()
            .map_err(|e| Error(format!("8-bit stream: {e}")))
    }

    fn skip32(&mut self, n: usize) -> Result<(), Error> {
        self.s32
            .take(n.saturating_mul(4))
            .map(|_| ())
            .map_err(|e| Error(format!("32-bit stream: {e}")))
    }

    fn skip16(&mut self, n: usize) -> Result<(), Error> {
        self.s16
            .take(n.saturating_mul(2))
            .map(|_| ())
            .map_err(|e| Error(format!("16-bit stream: {e}")))
    }

    fn vec3(&mut self) -> Result<[f32; 3], Error> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    fn vec3s(&mut self, n: usize) -> Result<Vec<[f32; 3]>, Error> {
        (0..n).map(|_| self.vec3()).collect()
    }

    fn vec2s(&mut self, n: usize) -> Result<Vec<[f32; 2]>, Error> {
        (0..n).map(|_| Ok([self.f32()?, self.f32()?])).collect()
    }

    fn u32s(&mut self, n: usize) -> Result<Vec<u32>, Error> {
        (0..n).map(|_| self.u32()).collect()
    }

    fn quat16(&mut self) -> Result<[f32; 4], Error> {
        let mut q = [0.0; 4];
        for c in &mut q {
            *c = self.u16()? as i16 as f32 / 32767.0;
        }
        Ok(q)
    }

    fn bounds(&mut self) -> Result<Bounds, Error> {
        Ok(Bounds {
            min: self.vec3()?,
            max: self.vec3()?,
        })
    }

    fn check_guard(&mut self, what: &str) -> Result<(), Error> {
        let g = self.guard;
        let (a, b, c) = (self.u32()?, self.u16()?, self.u8()?);
        if a != g || b != g as u16 || c != g as u8 {
            return err(format!(
                "checkpoint {g} ({what}) mismatch: got {a}/{b}/{c} at 32-bit word {}",
                self.s32.pos / 4 - 1
            ));
        }
        self.guard += 1;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Parsing

impl Shape {
    pub fn parse(data: &[u8]) -> Result<Shape, Error> {
        let mut r = Reader { d: data, pos: 0 };
        let version = r.u16()?;
        let exporter_version = r.u16()?;
        if !(23..=25).contains(&version) {
            return err(format!(
                "unsupported dts version {version} (exporter {exporter_version})"
            ));
        }
        let size = r.u32()? as usize;
        let start16 = r.u32()? as usize;
        let start8 = r.u32()? as usize;
        if start16 > start8 || start8 > size {
            return err(format!("bad stream offsets {start16}/{start8}/{size}"));
        }
        let buf = r.take(
            size.checked_mul(4)
                .ok_or_else(|| Error("buffer too large".into()))?,
        )?;
        let mut a = Alloc {
            s32: Reader {
                d: &buf[..start16 * 4],
                pos: 0,
            },
            s16: Reader {
                d: &buf[start16 * 4..start8 * 4],
                pos: 0,
            },
            s8: Reader {
                d: &buf[start8 * 4..],
                pos: 0,
            },
            guard: 0,
        };

        let mut shape = assemble(&mut a, version)?;
        shape.exporter_version = exporter_version;

        // Leftovers may only be alignment padding.
        let left = (
            a.s32.d.len() - a.s32.pos,
            a.s16.d.len() - a.s16.pos,
            a.s8.d.len() - a.s8.pos,
        );
        if left.0 != 0 || left.1 > 2 || left.2 > 3 {
            return err(format!(
                "unconsumed stream data (32/16/8-bit bytes): {left:?}"
            ));
        }

        let num_seq = r.u32()? as usize;
        if num_seq > data.len() {
            return err(format!("implausible sequence count {num_seq}"));
        }
        for _ in 0..num_seq {
            let seq = read_sequence(&mut r, &shape.names)?;
            shape.sequences.push(seq);
        }
        shape.materials = read_materials(&mut r, version)?;
        if r.pos != data.len() {
            return err(format!("{} trailing bytes", data.len() - r.pos));
        }
        shape.validate()?;
        Ok(shape)
    }

    fn validate(&self) -> Result<(), Error> {
        let n_nodes = self.nodes.len();
        for (i, n) in self.nodes.iter().enumerate() {
            if n.parent.is_some_and(|p| p >= n_nodes) {
                return err(format!("node {i} has bad parent"));
            }
        }
        for (i, o) in self.objects.iter().enumerate() {
            if o.first_mesh + o.num_meshes > self.meshes.len() {
                return err(format!("object {i} mesh range out of bounds"));
            }
            if o.node.is_some_and(|n| n >= n_nodes) {
                return err(format!("object {i} node out of range"));
            }
        }
        Ok(())
    }

    /// Looks up a name by index (-1 or out of range gives "").
    pub fn name(&self, index: i32) -> &str {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.names.get(i))
            .map_or("", |s| s.as_str())
    }

    /// Shape-space transform of every node in the default pose.
    pub fn node_world_transforms(&self) -> Vec<Transform> {
        let mut out: Vec<Option<Transform>> = vec![None; self.nodes.len()];
        fn resolve(
            shape: &Shape,
            i: usize,
            out: &mut Vec<Option<Transform>>,
            depth: usize,
        ) -> Transform {
            if let Some(t) = out[i] {
                return t;
            }
            let local = shape.nodes[i].local_transform();
            let t = match shape.nodes[i].parent {
                Some(p) if p < shape.nodes.len() && p != i && depth < shape.nodes.len() => {
                    mul(&resolve(shape, p, out, depth + 1), &local)
                }
                _ => local,
            };
            out[i] = Some(t);
            t
        }
        for i in 0..self.nodes.len() {
            resolve(self, i, &mut out, 0);
        }
        out.into_iter().map(|t| t.unwrap_or(IDENTITY)).collect()
    }

    /// Shape-space transform of every node with keyframe `keyframe` of
    /// sequence `seq` applied (`setSequence(thread, seq, pos)` then
    /// `animate`): the nodes the sequence rotates or translates take that
    /// keyframe's local rotation / translation, the others keep their
    /// defaults. Scale tracks are not applied. `None` if there is no such
    /// sequence.
    pub fn node_world_transforms_at(&self, seq: &str, keyframe: usize) -> Option<Vec<Transform>> {
        let s = self.sequences.iter().find(|s| s.name.eq_ignore_ascii_case(seq))?;
        let nk = s.num_keyframes.max(1) as usize;
        let k = keyframe.min(nk - 1);
        let mut rot: Vec<[f32; 4]> = self.nodes.iter().map(|n| n.rotation).collect();
        let mut tr: Vec<[f32; 3]> = self.nodes.iter().map(|n| n.translation).collect();
        for (j, &node) in s.rotation_matters.iter().enumerate() {
            if let (Some(q), Some(slot)) = (self.node_rotations.get(s.base_rotation.max(0) as usize + j * nk + k), rot.get_mut(node)) {
                *slot = *q;
            }
        }
        for (j, &node) in s.translation_matters.iter().enumerate() {
            if let (Some(t), Some(slot)) = (self.node_translations.get(s.base_translation.max(0) as usize + j * nk + k), tr.get_mut(node)) {
                *slot = *t;
            }
        }
        let local: Vec<Transform> = rot
            .iter()
            .zip(&tr)
            .map(|(q, t)| {
                let mut m = quat_to_matrix(*q);
                for (row, v) in m.iter_mut().zip(t) {
                    row[3] = *v;
                }
                m
            })
            .collect();
        let mut out = Vec::with_capacity(local.len());
        for i in 0..local.len() {
            // Walk up to the root (parents may come after children).
            let mut m = local[i];
            let mut p = self.nodes[i].parent;
            let mut depth = 0;
            while let Some(pi) = p.filter(|&pi| pi < local.len() && depth < local.len()) {
                m = mul(&local[pi], &m);
                p = self.nodes[pi].parent;
                depth += 1;
            }
            out.push(m);
        }
        Some(out)
    }

    /// The mesh of `object` used at detail level `detail`, if any.
    pub fn detail_mesh(&self, detail: usize, object: usize) -> Option<usize> {
        let d = self.detail_levels.get(detail)?;
        let o = self.objects.get(object)?;
        let od = usize::try_from(d.object_detail).ok()?;
        (od < o.num_meshes).then_some(o.first_mesh + od)
    }

    /// Objects drawn at a detail level (those of its sub-shape).
    pub fn detail_objects(&self, detail: usize) -> std::ops::Range<usize> {
        let ss = self
            .detail_levels
            .get(detail)
            .and_then(|d| usize::try_from(d.sub_shape).ok())
            .and_then(|s| self.sub_shapes.get(s));
        match ss {
            Some(s) => s.first_object..(s.first_object + s.num_objects).min(self.objects.len()),
            None => 0..0,
        }
    }

    /// Geometry of a detail level in shape space and the default pose: one
    /// entry per (mesh, material), vertices compacted, frame 0 only. Objects
    /// hidden by default and null/decal meshes are skipped; skin meshes are
    /// skinned with the default node transforms.
    pub fn detail_geometry(&self, detail: usize) -> Vec<RenderMesh> {
        self.detail_geometry_impl(detail, false)
    }

    /// Like [`Shape::detail_geometry`], but also includes objects hidden by
    /// default (for callers that switch visibility at run time).
    pub fn detail_geometry_all(&self, detail: usize) -> Vec<RenderMesh> {
        self.detail_geometry_impl(detail, true)
    }

    fn detail_geometry_impl(&self, detail: usize, hidden: bool) -> Vec<RenderMesh> {
        let world = self.node_world_transforms();
        let mut out = Vec::new();
        for oi in self.detail_objects(detail) {
            let obj = &self.objects[oi];
            if obj.default_visibility <= 0.0 && !hidden {
                continue;
            }
            let Some(mi) = self.detail_mesh(detail, oi) else {
                continue;
            };
            let mesh = &self.meshes[mi];
            if matches!(mesh.kind, MeshKind::Null | MeshKind::Decal) {
                continue;
            }
            let nv = mesh.frame_vertex_count();
            let (mut pos, mut nrm) = if mesh.kind == MeshKind::Skin {
                skin_default_pose(mesh, &world)
            } else {
                (
                    mesh.vertices[..nv].to_vec(),
                    mesh.normals.get(..nv).map_or_else(Vec::new, |n| n.to_vec()),
                )
            };
            nrm.resize(pos.len(), [0.0, 0.0, 1.0]);
            if let Some(t) = obj.node.and_then(|n| world.get(n)) {
                for p in &mut pos {
                    *p = transform_point(t, *p);
                }
                for n in &mut nrm {
                    *n = transform_vector(t, *n);
                }
            }

            // Group triangles by material, keeping first-seen order.
            let mut groups: Vec<(Option<u32>, Vec<[u32; 3]>)> = Vec::new();
            for p in &mesh.primitives {
                let gi = match groups.iter().position(|g| g.0 == p.material) {
                    Some(gi) => gi,
                    None => {
                        groups.push((p.material, Vec::new()));
                        groups.len() - 1
                    }
                };
                p.triangles_into(&mesh.indices, &mut groups[gi].1);
            }
            repair_normals(&pos, &mut nrm, groups.iter().flat_map(|g| &g.1));
            for (mat, tris) in groups {
                let mut remap = vec![u32::MAX; pos.len()];
                let mut rm = RenderMesh {
                    object: oi,
                    mesh: mi,
                    material: mat.map(|m| m as usize),
                    positions: Vec::new(),
                    normals: Vec::new(),
                    uvs: Vec::new(),
                    source: Vec::new(),
                    triangles: Vec::with_capacity(tris.len()),
                };
                'tri: for t in tris {
                    let mut nt = [0u32; 3];
                    for (k, &v) in t.iter().enumerate() {
                        let v = v as usize;
                        if v >= pos.len() {
                            continue 'tri;
                        }
                        if remap[v] == u32::MAX {
                            remap[v] = rm.positions.len() as u32;
                            rm.positions.push(pos[v]);
                            rm.normals.push(nrm[v]);
                            rm.source.push(v as u32);
                            rm.uvs
                                .push(mesh.tverts.get(v).copied().unwrap_or([0.0, 0.0]));
                        }
                        nt[k] = remap[v];
                    }
                    rm.triangles.push(nt);
                }
                if !rm.triangles.is_empty() {
                    out.push(rm);
                }
            }
        }
        out
    }

    /// [`Shape::detail_geometry`] of the highest detail level.
    pub fn detail0_geometry(&self) -> Vec<RenderMesh> {
        self.detail_geometry(0)
    }

    /// The points of the highest detail's geometry (shape space) that rest
    /// on the ground when the shape lies with `down` (a direction in shape
    /// space) pointing into it: the side of the bounds facing `down` most
    /// is the base, and every vertex within 0.3 m of it (or beyond it; of
    /// the geometry's own extreme that way when none comes that near) is a
    /// foot, with the midpoints and centres of the triangles made only of
    /// feet (so a dip under a wide base face is found too). A skeleton's
    /// rib ends and skull, a tree's roots, a rock's underside.
    pub fn base_points(&self, down: [f32; 3]) -> Vec<[f32; 3]> {
        let a = (0..3).max_by(|&i, &j| down[i].abs().total_cmp(&down[j].abs())).unwrap();
        let s = if down[a] < 0.0 { -1.0 } else { 1.0 };
        // How far a point is along `down`'s axis towards the base.
        let along = |p: &[f32; 3]| s * p[a];
        let plane = if s > 0.0 { self.bounds.max[a] } else { -self.bounds.min[a] };
        let geo = self.detail0_geometry();
        let far = geo.iter().flat_map(|g| &g.positions).map(along).fold(f32::MIN, f32::max);
        let cut = plane.min(far) - 0.3;
        let mut out = Vec::new();
        for g in &geo {
            let foot: Vec<bool> = g.positions.iter().map(|p| along(p) >= cut).collect();
            out.extend(g.positions.iter().zip(&foot).filter(|(_, f)| **f).map(|(p, _)| *p));
            for t in &g.triangles {
                if t.iter().all(|&i| foot[i as usize]) {
                    let [p, q, r] = t.map(|i| g.positions[i as usize]);
                    let mid = |u: [f32; 3], v: [f32; 3]| [(u[0] + v[0]) / 2.0, (u[1] + v[1]) / 2.0, (u[2] + v[2]) / 2.0];
                    out.extend([mid(p, q), mid(q, r), mid(r, p), [(p[0] + q[0] + r[0]) / 3.0, (p[1] + q[1] + r[1]) / 3.0, (p[2] + q[2] + r[2]) / 3.0]]);
                }
            }
        }
        out
    }
}

fn assemble(a: &mut Alloc, version: u16) -> Result<Shape, Error> {
    let num_nodes = a.count("node")?;
    let num_objects = a.count("object")?;
    let num_decals = a.count("decal")?;
    let num_sub_shapes = a.count("sub-shape")?;
    let num_ifl = a.count("ifl material")?;
    let num_node_rots = a.count("node rotation")?;
    let num_node_trans = a.count("node translation")?;
    let num_uniform = a.count("uniform scale")?;
    let num_aligned = a.count("aligned scale")?;
    let num_arbitrary = a.count("arbitrary scale")?;
    let num_ground = if version > 23 {
        a.count("ground frame")?
    } else {
        0
    };
    let num_object_states = a.count("object state")?;
    let num_decal_states = a.count("decal state")?;
    let num_triggers = a.count("trigger")?;
    let num_details = a.count("detail")?;
    let num_meshes = a.count("mesh")?;
    let num_names = a.count("name")?;
    let smallest_visible_size = a.i32()?;
    let smallest_visible_dl = a.i32()?;
    a.check_guard("counts")?;

    let radius = a.f32()?;
    let tube_radius = a.f32()?;
    let center = a.vec3()?;
    let bounds = a.bounds()?;
    a.check_guard("bounds")?;

    let mut raw_nodes = Vec::with_capacity(num_nodes);
    for _ in 0..num_nodes {
        let (name, parent) = (a.i32()?, a.i32()?);
        a.skip32(3)?;
        raw_nodes.push((name, parent));
    }
    a.check_guard("nodes")?;

    let mut raw_objects = Vec::with_capacity(num_objects);
    for _ in 0..num_objects {
        let (name, num_meshes, start, node) = (a.i32()?, a.i32()?, a.i32()?, a.i32()?);
        a.skip32(2)?;
        raw_objects.push((name, num_meshes, start, node));
    }
    a.check_guard("objects")?;

    a.skip32(num_decals * 5)?;
    a.check_guard("decals")?;

    let mut raw_ifl = Vec::with_capacity(num_ifl);
    for _ in 0..num_ifl {
        raw_ifl.push([a.i32()?, a.i32()?, a.i32()?, a.i32()?, a.i32()?]);
    }
    a.check_guard("ifl materials")?;

    let ss_first_node = a.u32s(num_sub_shapes)?;
    let ss_first_object = a.u32s(num_sub_shapes)?;
    a.skip32(num_sub_shapes)?;
    a.check_guard("sub-shape starts")?;
    let ss_num_nodes = a.u32s(num_sub_shapes)?;
    let ss_num_objects = a.u32s(num_sub_shapes)?;
    a.skip32(num_sub_shapes)?;
    a.check_guard("sub-shape counts")?;

    let rotations = (0..num_nodes)
        .map(|_| a.quat16())
        .collect::<Result<Vec<_>, _>>()?;
    let translations = a.vec3s(num_nodes)?;
    // Animation data: node translations/rotations.
    let node_translations = a.vec3s(num_node_trans)?;
    let node_rotations = (0..num_node_rots).map(|_| a.quat16()).collect::<Result<Vec<_>, _>>()?;
    a.check_guard("node transforms")?;
    // Scales (version > 21).
    let node_uniform_scales = (0..num_uniform).map(|_| a.f32()).collect::<Result<Vec<_>, _>>()?;
    let node_aligned_scales = a.vec3s(num_aligned)?;
    let node_arbitrary_scale_factors = a.vec3s(num_arbitrary)?;
    let node_arbitrary_scale_rotations = (0..num_arbitrary).map(|_| a.quat16()).collect::<Result<Vec<_>, _>>()?;
    a.check_guard("node scales")?;
    let mut ground_translations = Vec::new();
    if version > 23 {
        ground_translations = a.vec3s(num_ground)?;
        a.skip16(num_ground * 4)?;
        a.check_guard("ground frames")?;
    }

    let mut object_vis = Vec::with_capacity(num_object_states);
    let mut object_frames = Vec::with_capacity(num_object_states);
    let mut object_mat_frames = Vec::with_capacity(num_object_states);
    for _ in 0..num_object_states {
        object_vis.push(a.f32()?);
        object_frames.push(a.i32()?);
        object_mat_frames.push(a.i32()?);
    }
    a.check_guard("object states")?;
    a.skip32(num_decal_states)?;
    a.check_guard("decal states")?;
    a.skip32(num_triggers * 2)?;
    a.check_guard("triggers")?;

    let mut raw_details = Vec::with_capacity(num_details);
    for _ in 0..num_details {
        raw_details.push((
            a.i32()?,
            a.i32()?,
            a.i32()?,
            a.f32()?,
            a.f32()?,
            a.f32()?,
            a.i32()?,
        ));
    }
    a.check_guard("details")?;

    let mut meshes: Vec<Mesh> = Vec::with_capacity(num_meshes);
    for i in 0..num_meshes {
        let ty = a.u32()?;
        let mesh = read_mesh(a, ty, &meshes).map_err(|e| Error(format!("mesh {i}: {}", e.0)))?;
        meshes.push(mesh);
    }
    a.check_guard("meshes")?;

    let mut names = Vec::with_capacity(num_names);
    for _ in 0..num_names {
        let mut s = String::new();
        loop {
            match a.u8()? {
                0 => break,
                c => s.push(c as char),
            }
        }
        names.push(s);
    }
    a.check_guard("names")?;

    let name = |i: i32| {
        usize::try_from(i)
            .ok()
            .and_then(|i| names.get(i))
            .cloned()
            .unwrap_or_default()
    };
    let opt = |i: i32| usize::try_from(i).ok();

    let nodes = raw_nodes
        .iter()
        .zip(rotations.into_iter().zip(translations))
        .map(|(&(n, p), (rotation, translation))| Node {
            name: name(n),
            parent: opt(p),
            translation,
            rotation,
        })
        .collect();
    let mut objects = Vec::with_capacity(num_objects);
    for (i, &(n, nm, start, node)) in raw_objects.iter().enumerate() {
        if nm < 0 || start < 0 {
            return err(format!("object {i} has negative mesh range"));
        }
        objects.push(Object {
            name: name(n),
            node: opt(node),
            first_mesh: start as usize,
            num_meshes: nm as usize,
            default_visibility: object_vis.get(i).copied().unwrap_or(1.0),
        });
    }
    let sub_shapes = (0..num_sub_shapes)
        .map(|i| SubShape {
            first_node: ss_first_node[i] as usize,
            num_nodes: ss_num_nodes[i] as usize,
            first_object: ss_first_object[i] as usize,
            num_objects: ss_num_objects[i] as usize,
        })
        .collect();
    let ifl_materials = raw_ifl
        .iter()
        .map(|f| IflMaterial {
            name: name(f[0]),
            material_slot: f[1].max(0) as usize,
            first_frame: f[2],
            time: f[3],
            num_frames: f[4],
        })
        .collect();
    let detail_levels = raw_details
        .iter()
        .map(|&(n, ss, od, size, avg, max, polys)| DetailLevel {
            name: name(n),
            sub_shape: ss,
            object_detail: od,
            size,
            average_error: avg,
            max_error: max,
            poly_count: polys,
        })
        .collect();

    Ok(Shape {
        version,
        exporter_version: 0,
        radius,
        tube_radius,
        center,
        bounds,
        smallest_visible_size,
        smallest_visible_dl,
        nodes,
        objects,
        sub_shapes,
        ifl_materials,
        detail_levels,
        meshes,
        names,
        sequences: Vec::new(),
        node_rotations,
        node_translations,
        node_uniform_scales,
        node_aligned_scales,
        node_arbitrary_scale_factors,
        node_arbitrary_scale_rotations,
        ground_translations,
        object_states: object_vis,
        object_state_frames: object_frames,
        object_state_mat_frames: object_mat_frames,
        materials: Vec::new(),
        checkpoints: a.guard,
    })
}

/// Data owned by the mesh itself, or a copy of the parent's when shared.
fn shared<T: Clone>(
    a: &mut Alloc,
    parent: Option<&Mesh>,
    n: usize,
    read: impl FnOnce(&mut Alloc, usize) -> Result<Vec<T>, Error>,
    get: impl FnOnce(&Mesh) -> &[T],
) -> Result<Vec<T>, Error> {
    match parent {
        None => read(a, n),
        Some(p) => get(p)
            .get(..n)
            .map(|s| s.to_vec())
            .ok_or_else(|| Error(format!("shared data larger than parent's ({n})"))),
    }
}

fn read_mesh(a: &mut Alloc, ty: u32, prev: &[Mesh]) -> Result<Mesh, Error> {
    let kind = match ty {
        0 => MeshKind::Standard,
        1 => MeshKind::Skin,
        2 => MeshKind::Decal,
        3 => MeshKind::Sorted,
        4 => return Ok(Mesh::empty(MeshKind::Null)),
        _ => return err(format!("unknown mesh type {ty:#x}")),
    };
    let mut m = Mesh::empty(kind);
    if kind == MeshKind::Decal {
        // Deprecated decal mesh: parse and discard.
        let n = a.count("decal primitive")?;
        a.skip16(n * 2)?;
        a.skip32(n)?;
        let n = a.count("decal index")?;
        a.skip16(n)?;
        let n = a.count("decal start")?;
        a.skip32(n * 9)?;
        a.skip32(1)?;
        a.check_guard("decal mesh")?;
        return Ok(m);
    }

    a.check_guard("mesh start")?;
    m.num_frames = a.u32()?;
    m.num_mat_frames = a.u32()?;
    let parent_idx = a.i32()?;
    m.bounds = a.bounds()?;
    m.center = a.vec3()?;
    m.radius = a.f32()?;
    let parent = match usize::try_from(parent_idx) {
        Err(_) => None,
        Ok(p) if p < prev.len() => {
            m.parent_mesh = Some(p);
            Some(&prev[p])
        }
        Ok(p) => return err(format!("parent mesh {p} not yet read")),
    };

    let nv = a.count("vertex")?;
    m.vertices = shared(a, parent, nv, |a, n| a.vec3s(n), |p| &p.vertices)?;
    let nt = a.count("tvert")?;
    m.tverts = shared(a, parent, nt, |a, n| a.vec2s(n), |p| &p.tverts)?;
    m.normals = shared(a, parent, nv, |a, n| a.vec3s(n), |p| &p.normals)?;
    m.encoded_normals = shared(
        a,
        parent,
        nv,
        |a, n| (0..n).map(|_| a.u8()).collect(),
        |p| &p.encoded_normals,
    )?;

    let np = a.count("primitive")?;
    let se: Vec<(u16, u16)> = (0..np)
        .map(|_| Ok((a.u16()?, a.u16()?)))
        .collect::<Result<_, Error>>()?;
    let mats = a.u32s(np)?;
    m.primitives = se
        .iter()
        .zip(mats)
        .map(|(&(s, n), raw)| Primitive::from_raw(s as u32, n as u32, raw))
        .collect();
    let ni = a.count("index")?;
    m.indices = (0..ni)
        .map(|_| a.u16().map(u32::from))
        .collect::<Result<_, _>>()?;
    let nmerge = a.count("merge index")?;
    a.skip16(nmerge)?;
    m.verts_per_frame = a.u32()?;
    m.flags = a.u32()?;
    a.check_guard("mesh end")?;

    match kind {
        MeshKind::Skin => {
            let mut s = Skin::default();
            let n = a.count("skin vertex")?;
            let iv = shared(a, parent, n, |a, n| a.vec3s(n), |p| &p.vertices)?;
            let inrm = shared(a, parent, n, |a, n| a.vec3s(n), |p| &p.normals)?;
            let ienc = shared(
                a,
                parent,
                n,
                |a, n| (0..n).map(|_| a.u8()).collect(),
                |p| &p.encoded_normals,
            )?;
            if m.vertices.is_empty() {
                m.vertices = iv;
            }
            if m.normals.is_empty() {
                m.normals = inrm;
            }
            if m.encoded_normals.is_empty() {
                m.encoded_normals = ienc;
            }
            let ps = parent.and_then(|p| p.skin.as_ref());
            let need = |what: &str| Error(format!("skin shares {what} with a non-skin parent"));
            let n = a.count("initial transform")?;
            s.initial_transforms = match parent {
                None => (0..n)
                    .map(|_| {
                        let mut t = [0.0; 16];
                        for v in &mut t {
                            *v = a.f32()?;
                        }
                        Ok(t)
                    })
                    .collect::<Result<_, Error>>()?,
                Some(_) => ps
                    .ok_or_else(|| need("transforms"))?
                    .initial_transforms
                    .get(..n)
                    .ok_or_else(|| need("transforms"))?
                    .to_vec(),
            };
            let n = a.count("skin weight")?;
            match parent {
                None => {
                    s.vertex_index = a.u32s(n)?;
                    s.bone_index = a.u32s(n)?;
                    s.weight = (0..n).map(|_| a.f32()).collect::<Result<_, _>>()?;
                }
                Some(_) => {
                    let p = ps.ok_or_else(|| need("weights"))?;
                    s.vertex_index = p
                        .vertex_index
                        .get(..n)
                        .ok_or_else(|| need("weights"))?
                        .to_vec();
                    s.bone_index = p
                        .bone_index
                        .get(..n)
                        .ok_or_else(|| need("weights"))?
                        .to_vec();
                    s.weight = p.weight.get(..n).ok_or_else(|| need("weights"))?.to_vec();
                }
            }
            let n = a.count("skin node")?;
            s.node_index = match parent {
                None => a.u32s(n)?,
                Some(_) => ps
                    .ok_or_else(|| need("nodes"))?
                    .node_index
                    .get(..n)
                    .ok_or_else(|| need("nodes"))?
                    .to_vec(),
            };
            a.check_guard("skin")?;
            m.skin = Some(s);
        }
        MeshKind::Sorted => {
            let n = a.count("cluster")?;
            a.skip32(n * 8)?;
            for what in ["start cluster", "first vert", "num verts", "first tvert"] {
                let n = a.count(what)?;
                a.skip32(n)?;
            }
            a.skip32(1)?; // always_write_depth
            a.check_guard("sorted mesh")?;
        }
        _ => {}
    }
    Ok(m)
}

fn read_sequence(r: &mut Reader, names: &[String]) -> Result<Sequence, Error> {
    let name_index = r.i32()?;
    let seq = Sequence {
        name: usize::try_from(name_index)
            .ok()
            .and_then(|i| names.get(i))
            .cloned()
            .unwrap_or_default(),
        flags: r.u32()?,
        num_keyframes: r.i32()?,
        duration: r.f32()?,
        priority: r.i32()?,
        first_ground_frame: r.i32()?,
        num_ground_frames: r.i32()?,
        base_rotation: r.i32()?,
        base_translation: r.i32()?,
        base_scale: r.i32()?,
        base_object_state: r.i32()?,
        base_decal_state: r.i32()?,
        first_trigger: r.i32()?,
        num_triggers: r.i32()?,
        tool_begin: r.f32()?,
        rotation_matters: Vec::new(),
        translation_matters: Vec::new(),
        scale_matters: Vec::new(),
        vis_matters: Vec::new(),
        ifl_matters: Vec::new(),
        frame_matters: Vec::new(),
        mat_frame_matters: Vec::new(),
    };
    let mut seq = seq;
    // Membership sets (TSIntegerSet: ignored count, word count, bit words):
    // rotation, translation, scale, decal, ifl, vis, frame, mat frame.
    for set in 0..8 {
        let _num_ints = r.u32()?;
        let words = r.u32()? as usize;
        if words > 1 << 16 {
            return err(format!("implausible integer set size {words}"));
        }
        let bytes = r.take(words * 4)?;
        let members: Vec<usize> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .flat_map(|(w, b)| {
                let bits = u32::from_le_bytes(*b);
                (0..32).filter(move |i| bits & (1 << i) != 0).map(move |i| w * 32 + i)
            })
            .collect();
        match set {
            0 => seq.rotation_matters = members,
            1 => seq.translation_matters = members,
            2 => seq.scale_matters = members,
            4 => seq.ifl_matters = members,
            6 => seq.frame_matters = members,
            7 => seq.mat_frame_matters = members,
            5 => seq.vis_matters = members,
            _ => {}
        }
    }
    Ok(seq)
}

fn read_materials(r: &mut Reader, version: u16) -> Result<Vec<Material>, Error> {
    let list_version = r.u8()?;
    if list_version != 1 {
        return err(format!("unsupported material list version {list_version}"));
    }
    let n = r.u32()? as usize;
    if n > r.d.len() {
        return err(format!("implausible material count {n}"));
    }
    let mut names = Vec::with_capacity(n);
    for _ in 0..n {
        let len = r.u8()? as usize;
        names.push(crate::latin1(r.take(len)?));
    }
    let cols = |r: &mut Reader| -> Result<Vec<u32>, Error> { (0..n).map(|_| r.u32()).collect() };
    let flags = cols(r)?;
    let refl = cols(r)?;
    let bump = cols(r)?;
    let detail = cols(r)?;
    if version == 25 {
        cols(r)?;
    }
    let detail_scale = cols(r)?;
    let refl_amount = cols(r)?;
    let opt = |v: u32| (v != u32::MAX).then_some(v);
    Ok((0..n)
        .map(|i| Material {
            name: names[i].clone(),
            flags: flags[i],
            reflectance_map: opt(refl[i]),
            bump_map: opt(bump[i]),
            detail_map: opt(detail[i]),
            detail_scale: f32::from_bits(detail_scale[i]),
            reflection_amount: f32::from_bits(refl_amount[i]),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Math helpers

/// Some exporters wrote zero-length float normals (their encoded normals are
/// then meaningless too); rebuild those from the adjacent faces.
fn repair_normals<'a>(
    pos: &[[f32; 3]],
    nrm: &mut [[f32; 3]],
    tris: impl Iterator<Item = &'a [u32; 3]> + Clone,
) {
    let bad: Vec<bool> = nrm.iter().map(|n| !(0.5..1.5).contains(&len(*n))).collect();
    if !bad.contains(&true) {
        return;
    }
    let mut acc = vec![[0.0f32; 3]; pos.len()];
    for t in tris {
        if t.iter().any(|&i| i as usize >= pos.len()) || !t.iter().any(|&i| bad[i as usize]) {
            continue;
        }
        let [a, b, c] = t.map(|i| pos[i as usize]);
        let u = sub(b, a);
        let v = sub(c, a);
        // Clockwise front faces: the outward normal is v x u.
        let f = [
            v[1] * u[2] - v[2] * u[1],
            v[2] * u[0] - v[0] * u[2],
            v[0] * u[1] - v[1] * u[0],
        ];
        for &i in t {
            for k in 0..3 {
                acc[i as usize][k] += f[k];
            }
        }
    }
    for (i, n) in nrm.iter_mut().enumerate() {
        if bad[i] {
            let l = len(acc[i]);
            *n = if l > 0.0 {
                acc[i].map(|c| c / l)
            } else {
                [0.0, 0.0, 1.0]
            };
        }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn len(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

pub fn quat_to_matrix(q: [f32; 4]) -> Transform {
    let [x, y, z, w] = q;
    let len2 = x * x + y * y + z * z + w * w;
    if x * x + y * y + z * z < 1e-20 || len2 == 0.0 {
        return IDENTITY;
    }
    // Quat16 values are not exactly unit length; normalise.
    let s = 2.0 / len2;
    let (xs, ys, zs) = (x * s, y * s, z * s);
    let (wx, wy, wz) = (w * xs, w * ys, w * zs);
    let (xx, xy, xz) = (x * xs, x * ys, x * zs);
    let (yy, yz, zz) = (y * ys, y * zs, z * zs);
    [
        [1.0 - (yy + zz), xy + wz, xz - wy, 0.0],
        [xy - wz, 1.0 - (xx + zz), yz + wx, 0.0],
        [xz + wy, yz - wx, 1.0 - (xx + yy), 0.0],
    ]
}

/// `a * b` for affine 3x4 transforms.
pub fn mul(a: &Transform, b: &Transform) -> Transform {
    let mut r = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..4 {
            r[i][j] = a[i][0] * b[0][j]
                + a[i][1] * b[1][j]
                + a[i][2] * b[2][j]
                + if j == 3 { a[i][3] } else { 0.0 };
        }
    }
    r
}

pub fn transform_point(m: &Transform, p: [f32; 3]) -> [f32; 3] {
    let v = transform_vector(m, p);
    [v[0] + m[0][3], v[1] + m[1][3], v[2] + m[2][3]]
}

pub fn transform_vector(m: &Transform, v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn from_matrix16(m: &[f32; 16]) -> Transform {
    [
        [m[0], m[1], m[2], m[3]],
        [m[4], m[5], m[6], m[7]],
        [m[8], m[9], m[10], m[11]],
    ]
}

/// Skins frame 0 of a skin mesh with the given node transforms. Vertices
/// without influences keep their bind-pose position.
fn skin_default_pose(mesh: &Mesh, world: &[Transform]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let nv = mesh.frame_vertex_count();
    let verts = &mesh.vertices[..nv];
    let norms: Vec<[f32; 3]> = (0..nv)
        .map(|i| mesh.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]))
        .collect();
    let Some(skin) = &mesh.skin else {
        return (verts.to_vec(), norms);
    };
    let bones: Vec<Option<Transform>> = (0..skin.initial_transforms.len())
        .map(|b| {
            let node = *skin.node_index.get(b)? as usize;
            Some(mul(
                world.get(node)?,
                &from_matrix16(&skin.initial_transforms[b]),
            ))
        })
        .collect();
    let mut pos = vec![[0.0f32; 3]; nv];
    let mut nrm = vec![[0.0f32; 3]; nv];
    let mut touched = vec![false; nv];
    for k in 0..skin.vertex_index.len() {
        let v = skin.vertex_index[k] as usize;
        let Some(Some(bm)) = skin.bone_index.get(k).and_then(|&b| bones.get(b as usize)) else {
            continue;
        };
        if v >= nv {
            continue;
        }
        let w = skin.weight.get(k).copied().unwrap_or(0.0);
        let p = transform_point(bm, verts[v]);
        let n = transform_vector(bm, norms[v]);
        for i in 0..3 {
            pos[v][i] += w * p[i];
            nrm[v][i] += w * n[i];
        }
        touched[v] = true;
    }
    for v in 0..nv {
        if !touched[v] {
            pos[v] = verts[v];
            nrm[v] = norms[v];
        } else {
            let l = (nrm[v][0] * nrm[v][0] + nrm[v][1] * nrm[v][1] + nrm[v][2] * nrm[v][2]).sqrt();
            if l > 0.0 {
                nrm[v] = nrm[v].map(|c| c / l);
            }
        }
    }
    (pos, nrm)
}
