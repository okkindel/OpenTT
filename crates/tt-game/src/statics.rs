//! Static obstacles (mission `Prop` shapes) as world-space triangles.

use std::sync::Arc;

use glam::{Affine3A, Vec3};
use tt_assets::dts::{MeshKind, Shape};

use crate::geom::Aabb;

/// Collision geometry of a shape in shape space.
#[derive(Debug, Clone, PartialEq)]
pub struct StaticShape {
    pub triangles: Vec<[Vec3; 3]>,
    pub bounds: Aabb,
}

impl StaticShape {
    /// Triangles of all the shape's collision detail levels (`Collision-N`,
    /// any case; e.g. recharge.dts has four) in the default pose, as
    /// `buildPolyList` collects them. A shape without a collision detail has
    /// no collision at all (boost and jump pads).
    pub fn from_shape(shape: &Shape) -> StaticShape {
        let world = shape.node_world_transforms();
        let mut triangles = Vec::new();
        for (di, d) in shape.detail_levels.iter().enumerate() {
            if !d.name.to_ascii_lowercase().starts_with("collision") {
                continue;
            }
            for oi in shape.detail_objects(di) {
                let Some(mi) = shape.detail_mesh(di, oi) else { continue };
                let mesh = &shape.meshes[mi];
                if matches!(mesh.kind, MeshKind::Null | MeshKind::Decal) {
                    continue;
                }
                let n = mesh.frame_vertex_count().min(mesh.vertices.len());
                let xf = shape.objects[oi].node.and_then(|n| world.get(n)).map(crate::geom::affine_from_rows).unwrap_or(Affine3A::IDENTITY);
                let verts: Vec<Vec3> = mesh.vertices[..n].iter().map(|&v| xf.transform_point3(Vec3::from(v))).collect();
                for t in mesh.triangles() {
                    if let (Some(&a), Some(&b), Some(&c)) = (verts.get(t[0] as usize), verts.get(t[1] as usize), verts.get(t[2] as usize)) {
                        // DTS triangles wind clockwise seen from outside; store
                        // them counter-clockwise so (b−a)×(c−a) points out.
                        triangles.push([a, c, b]);
                    }
                }
            }
        }
        let bounds = Aabb::from_bounds(&shape.bounds);
        if triangles.is_empty() {
            return StaticShape { triangles, bounds };
        }
        StaticShape { bounds: Aabb::from_points(triangles.iter().flatten().copied()).union(&bounds), triangles }
    }

    /// A closed box (12 triangles, outward winding).
    pub fn from_box(b: Aabb) -> StaticShape {
        let c = b.corners();
        const F: [[usize; 4]; 6] = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let mut triangles = Vec::new();
        for f in F {
            triangles.push([c[f[0]], c[f[1]], c[f[2]]]);
            triangles.push([c[f[0]], c[f[2]], c[f[3]]]);
        }
        StaticShape { triangles, bounds: b }
    }
}

/// A placed static obstacle.
#[derive(Debug, Clone)]
pub struct StaticObject {
    pub shape: Arc<StaticShape>,
    pub transform: Affine3A,
    /// World-space triangles and their face normals.
    pub triangles: Vec<[Vec3; 3]>,
    pub normals: Vec<Vec3>,
    pub world_box: Aabb,
    /// The PowerUp/Boost/Volcano this collision geometry belongs to.
    pub trigger: Option<crate::TriggerId>,
}

impl StaticObject {
    pub fn new(shape: Arc<StaticShape>, transform: Affine3A) -> StaticObject {
        let triangles: Vec<[Vec3; 3]> = shape.triangles.iter().map(|t| t.map(|v| transform.transform_point3(v))).collect();
        let normals = triangles.iter().map(|t| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero()).collect();
        let world_box = Aabb::from_points(triangles.iter().flatten().copied());
        StaticObject { world_box: if world_box.is_empty() { shape.bounds.transformed(&transform) } else { world_box }, shape, transform, triangles, normals, trigger: None }
    }
}
