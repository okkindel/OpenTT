//! A minimal DTS v24 writer for OpenTT's own procedural shapes (one node,
//! one object, no sequences), `#[path]`-included by `mortar_rocket.rs` and
//! `powerup_terrain.rs` (not an example of its own).

#[derive(Default)]
pub struct Mesh {
    pub verts: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Triangles per material.
    pub tris: [Vec<[u16; 3]>; 2],
}


struct Streams {
    s32: Vec<u8>,
    s16: Vec<u8>,
    s8: Vec<u8>,
    guard: u32,
}

impl Streams {
    fn u32(&mut self, v: u32) {
        self.s32.extend(v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.u32(v as u32);
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn vec3(&mut self, v: [f32; 3]) {
        v.iter().for_each(|&x| self.f32(x));
    }
    fn u16(&mut self, v: u16) {
        self.s16.extend(v.to_le_bytes());
    }
    fn u8(&mut self, v: u8) {
        self.s8.push(v);
    }
    fn guard(&mut self) {
        let g = self.guard;
        self.u32(g);
        self.u16(g as u16);
        self.u8(g as u8);
        self.guard += 1;
    }
}

/// The shape of mesh `m` (object `name`, `materials`: name and flags);
/// `collision`: the mesh is also its `Collision-1` detail (solid).
pub fn write_dts(m: &Mesh, name: &str, materials: &[(&str, u32)], collision: bool) -> Vec<u8> {
    let names = [name, "detail2", "Collision-1"];
    let names = &names[..2 + collision as usize];
    let meshes = 1 + collision as i32;
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &m.verts {
        for k in 0..3 {
            min[k] = min[k].min(v[k]);
            max[k] = max[k].max(v[k]);
        }
    }
    let center = [0.0; 3];
    let radius = m.verts.iter().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()).fold(0.0, f32::max);
    let tube = m.verts.iter().map(|v| (v[0] * v[0] + v[2] * v[2]).sqrt()).fold(0.0, f32::max);
    let mut s = Streams { s32: Vec::new(), s16: Vec::new(), s8: Vec::new(), guard: 0 };
    // Counts: nodes, objects, decals, sub-shapes, ifl, node rots, node
    // translations, uniform / aligned / arbitrary scales, ground frames,
    // object states, decal states, triggers, details, meshes, names.
    for n in [1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, meshes, meshes, names.len() as i32] {
        s.i32(n);
    }
    s.i32(0); // smallest visible size
    s.i32(0); // smallest visible detail level
    s.guard();
    s.f32(radius);
    s.f32(tube);
    s.vec3(center);
    s.vec3(min);
    s.vec3(max);
    s.guard();
    // Node: name, parent, first object, first child, next sibling.
    for v in [0, -1, -1, -1, -1] {
        s.i32(v);
    }
    s.guard();
    // Object: name, meshes, first mesh, node, next sibling, first decal.
    for v in [0, meshes, 0, 0, -1, -1] {
        s.i32(v);
    }
    s.guard();
    s.guard(); // decals
    s.guard(); // ifl materials
    for v in [0, 0, 0] {
        s.i32(v); // sub-shape first node / object / decal
    }
    s.guard();
    for v in [1, 1, 0] {
        s.i32(v); // sub-shape node / object / decal counts
    }
    s.guard();
    for q in [0u16, 0, 0, 32767] {
        s.u16(q); // default rotation (identity)
    }
    s.vec3([0.0; 3]); // default translation
    s.guard(); // node transforms
    s.guard(); // scales
    s.guard(); // ground frames
    s.f32(1.0);
    s.i32(0);
    s.i32(0);
    s.guard(); // object states
    s.guard(); // decal states
    s.guard(); // triggers
    let polys: usize = m.tris.iter().map(Vec::len).sum();
    s.i32(1); // detail name "detail2"
    s.i32(0); // sub-shape
    s.i32(0); // object detail
    s.f32(2.0); // size
    s.f32(-1.0);
    s.f32(-1.0);
    s.i32(polys as i32);
    if collision {
        for v in [2, 0, 1] {
            s.i32(v); // "Collision-1", sub-shape, object detail
        }
        s.f32(-1.0); // size: never drawn
        s.f32(-1.0);
        s.f32(-1.0);
        s.i32(polys as i32);
    }
    s.guard();
    // The mesh (once more for the collision detail).
    for _ in 0..meshes {
        s.u32(0); // standard
        s.guard();
        s.u32(1); // frames
        s.u32(1); // material frames
        s.i32(-1); // parent
        s.vec3(min);
        s.vec3(max);
        s.vec3(center);
        s.f32(radius);
        s.i32(m.verts.len() as i32);
        m.verts.iter().for_each(|&v| s.vec3(v));
        s.i32(m.uvs.len() as i32);
        m.uvs.iter().for_each(|&[u, v]| {
            s.f32(u);
            s.f32(v)
        });
        m.normals.iter().for_each(|&n| s.vec3(n));
        m.normals.iter().for_each(|_| s.u8(0));
        let prims: Vec<(usize, &Vec<[u16; 3]>)> = m.tris.iter().enumerate().filter(|(_, t)| !t.is_empty()).collect();
        s.i32(prims.len() as i32);
        let mut start = 0;
        for (_, t) in &prims {
            s.u16(start as u16);
            s.u16((t.len() * 3) as u16);
            start += t.len() * 3;
        }
        for (mat, _) in &prims {
            s.u32(0x2000_0000 | *mat as u32); // indexed triangles
        }
        s.i32(start as i32);
        for (_, t) in &prims {
            t.iter().flatten().for_each(|&i| s.u16(i));
        }
        s.i32(0); // merge indices
        s.u32(m.verts.len() as u32); // vertices per frame
        s.u32(0); // flags
        s.guard();
    }
    s.guard(); // meshes
    for n in names.iter() {
        n.bytes().for_each(|b| s.u8(b));
        s.u8(0);
    }
    s.guard();
    while !s.s16.len().is_multiple_of(4) {
        s.s16.push(0);
    }
    while !s.s8.len().is_multiple_of(4) {
        s.s8.push(0);
    }
    let (w32, w16, w8) = (s.s32.len() / 4, s.s16.len() / 4, s.s8.len() / 4);
    let mut out = Vec::new();
    out.extend(24u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(((w32 + w16 + w8) as u32).to_le_bytes());
    out.extend((w32 as u32).to_le_bytes());
    out.extend(((w32 + w16) as u32).to_le_bytes());
    out.extend(&s.s32);
    out.extend(&s.s16);
    out.extend(&s.s8);
    out.extend(0u32.to_le_bytes()); // sequences
    out.push(1); // material list version
    out.extend((materials.len() as u32).to_le_bytes());
    for (name, _) in materials {
        out.push(name.len() as u8);
        out.extend(name.bytes());
    }
    let col = |out: &mut Vec<u8>, f: &dyn Fn(u32) -> u32| materials.iter().for_each(|&(_, fl)| out.extend(f(fl).to_le_bytes()));
    col(&mut out, &|fl| fl); // flags
    for _ in 0..3 {
        col(&mut out, &|_| u32::MAX); // reflectance / bump / detail maps
    }
    col(&mut out, &|_| 1.0f32.to_bits()); // detail scale
    col(&mut out, &|_| 0.0f32.to_bits()); // reflection amount
    out
}
