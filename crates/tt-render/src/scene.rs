//! CPU-side scene built from a mission: terrain mesh, textures, lights,
//! and marker boxes for objects whose shapes are not loaded yet.

use glam::{Mat4, Quat, Vec3, Vec4};
use std::path::{Path, PathBuf};
use tt_assets::mission::SceneObject;
use tt_assets::terrain::{Terrain, MATERIAL_SLOTS};


#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// Position in terrain grid units (wraps every `size`).
    pub grid: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ColorVertex {
    pub pos: [f32; 3],
    pub color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkyVertex {
    pub dir: [f32; 3],
    pub uv: [f32; 2],
    pub layer: u32,
}

/// A placed shape: `.dts` file, object-to-world transform and the fields the
/// mission lighting reads.
pub struct ShapeInstance {
    pub path: PathBuf,
    pub transform: Mat4,
    /// Casts a static shadow into the terrain lightmap (every `Prop`, and
    /// `PowerUp`s whose def sets `static_shadow`).
    pub casts_shadow: bool,
    pub tree_lighting: bool,
    pub light_boost: f32,
}

pub struct Scene {
    pub name: String,
    pub shapes: crate::shapes::ShapeSet,
    /// Skybox: 4 side panorama faces and the top, `textures::size()`^2 RGBA8 each.
    pub sky_faces: Vec<Vec<u8>>,
    pub sky_vertices: Vec<SkyVertex>,
    pub terrain_vertices: Vec<TerrainVertex>,
    pub terrain_indices: Vec<u32>,
    pub terrain_size: u32,
    /// Blend weights for slots 0..4 and 4..8, RGBA8, `terrain_size`^2 each.
    pub alpha: [Vec<u8>; 2],
    /// One RGBA8 `textures::size()`^2 layer per slot.
    pub materials: Vec<Vec<u8>>,
    /// The image file of each material slot (normal maps sit next to it).
    pub material_files: Vec<Option<PathBuf>>,
    /// Unit vector pointing towards the sun.
    pub sun_dir: Vec3,
    /// Baked terrain lightmap (RGBA8, `lightmap_size`^2, display-space factor
    /// `q/32` per channel, uploaded as sRGB).
    pub lightmap: Vec<u8>,
    pub lightmap_size: u32,
    /// Terrain `detail_texture` (RGBA8 `textures::size()`^2), normalised
    /// to a mean of 128 so `2 x detail` keeps the average brightness.
    pub detail: Option<Vec<u8>>,
    /// The sun after `conform`, and the lightmap as colours, for lighting
    /// moving objects ([`Scene::shape_light`]).
    pub sun: crate::lighting::Sun,
    pub light: crate::lighting::Lightmap,
    /// Time the lighting bake took.
    pub lighting_time: std::time::Duration,
    pub terrain_origin: f32,
    pub square_size: f32,
    pub sun_color: Vec4,
    pub ambient: Vec4,
    pub fog_color: Vec4,
    pub visible_distance: f32,
    pub fog_distance: f32,
    /// Height fog bands: (visible distance, min height, max height).
    pub fog_volumes: Vec<[f32; 3]>,
    pub markers: Vec<ColorVertex>,
    pub spawn: Option<Vec3>,
    /// The terrain block (heights for placing objects, see [`Scene::terrain_height`]).
    pub terrain: Terrain,
    /// Particle, emitter and explosion defs (for [`crate::fx::Effects`]).
    pub fx: crate::fx::FxLibrary,
}

fn latin1(path: &Path) -> Result<String, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(tt_assets::latin1(&b))
}

fn vec4(o: &SceneObject, field: &str, default: Vec4) -> Vec4 {
    match o.floats(field).as_slice() {
        [r, g, b, a, ..] => Vec4::new(*r, *g, *b, *a),
        [r, g, b] => Vec4::new(*r, *g, *b, 1.0),
        _ => default,
    }
}

/// A texture (`base`: its file without extension) resampled to the layer size.
pub(crate) fn load_material(base: &Path) -> Vec<u8> {
    let flat = vec![128u8; (crate::textures::size() * crate::textures::size() * 4) as usize];
    if base.as_os_str().is_empty() {
        return flat;
    }
    let Some(file) = crate::textures::find(base) else {
        eprintln!("warning: texture not found: {}", base.display());
        return flat;
    };
    match crate::textures::layer(&file) {
        Ok(px) => px.to_vec(),
        Err(e) => {
            eprintln!("warning: {}: {e}", file.display());
            flat
        }
    }
}

impl Scene {
    /// The scene of a mission from its RON file `mission`: its objects, the
    /// conditional ones as with no game settings (no mission type), and
    /// only the defs the mission declares itself (the game's come
    /// from the running engine: [`Scene::from_objects`]).
    pub fn load(root: &Path, mission: &Path) -> Result<Scene, String> {
        let m = tt_assets::data::mission::load(mission)?;
        let objects = m.scene_objects(|c| !c.unless.1.is_empty());
        let all: Vec<&SceneObject> = objects.iter().flat_map(|o| o.walk()).collect();
        Scene::from_objects(root, mission.parent().unwrap_or(root), &all, &m.def_objects())
    }

    /// Builds a scene from already-instantiated mission objects (e.g. the
    /// engine's loaded mission, flattened) and the defs
    /// they reference by name. `mission_dir` resolves `./` paths.
    pub fn from_objects(root: &Path, mission_dir: &Path, all: &[&SceneObject], defs: &[SceneObject]) -> Result<Scene, String> {
        Scene::from_objects_filtered(root, mission_dir, all, defs, &|_| true)
    }

    /// [`Scene::from_objects`], drawing only the objects for which `draw`
    /// returns true (e.g. leave out power-ups to draw them as dynamic
    /// instances). Objects left out still cast their static shadow into the
    /// lightmap if they would otherwise (props, `static_shadow` PowerUps),
    /// since the engine bakes them at mission load regardless.
    pub fn from_objects_filtered(
        root: &Path,
        mission_dir: &Path,
        all: &[&SceneObject],
        defs: &[SceneObject],
        draw: &dyn Fn(&SceneObject) -> bool,
    ) -> Result<Scene, String> {
        let find = |class: &str| all.iter().copied().find(|o| o.class.eq_ignore_ascii_case(class));

        let _all = crate::loadstats::span(|| "scene: total".into());
        let sp = crate::loadstats::span(|| "scene: terrain parse".into());
        let tb = find("Terrain").ok_or("mission has no Terrain")?;
        let ter_path: PathBuf = tt_assets::resolve(root, mission_dir, tb.get("terrain_file").ok_or("no terrainFile")?);
        let ter_data = std::fs::read(&ter_path).map_err(|e| format!("{}: {e}", ter_path.display()))?;
        let terrain = Terrain::parse(&ter_data).map_err(|e| format!("{}: {e}", ter_path.display()))?;
        let square_size = tb.get("square_size").and_then(|s| s.parse().ok()).unwrap_or(8.0);
        drop(sp);
        let sp = crate::loadstats::span(|| "scene: detail + materials + sky decode".into());
        // Every terrain, detail and sky texture decoded on all cores first.
        let sky_list = find("Sky").and_then(|s| s.get("material_list")).and_then(|dml| sky_bases(root, mission_dir, dml));
        let bases: Vec<PathBuf> = tb
            .get("detail_texture")
            .filter(|p| !p.is_empty())
            .map(|p| tt_assets::resolve(root, mission_dir, p))
            .into_iter()
            .chain(terrain.materials.iter().filter(|m| !m.is_empty()).map(|m| root.join(m)))
            .chain(sky_list.iter().flatten().cloned())
            .collect();
        let jobs: Vec<crate::textures::Job> = bases.iter().filter_map(|b| crate::textures::find(b)).collect();
        crate::textures::prefetch(&jobs);
        let detail = tb.get("detail_texture").filter(|p| !p.is_empty()).map(|p| {
            let mut px = load_material(&tt_assets::resolve(root, mission_dir, p));
            let n = (px.len() / 4) as f64;
            let mean = px.chunks(4).map(|c| (c[0] as f64 + c[1] as f64 + c[2] as f64) / 3.0).sum::<f64>() / n;
            if mean > 1.0 {
                for c in px.chunks_mut(4) {
                    for v in &mut c[..3] {
                        *v = (*v as f64 * 128.0 / mean).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            px
        });
        let o = terrain.origin(square_size);
        let origin = (o, o);

        let (vertices, indices) = terrain_mesh(&terrain, square_size, origin, 1024.0);

        let n = terrain.size();
        let mut alpha = [vec![0u8; n * n * 4], vec![0u8; n * n * 4]];
        for slot in 0..MATERIAL_SLOTS {
            if let Some(a) = &terrain.alpha[slot] {
                for (i, &w) in a.iter().enumerate() {
                    alpha[slot / 4][i * 4 + slot % 4] = w;
                }
            }
        }
        let materials = terrain.materials.iter().map(|m| load_material(&if m.is_empty() { PathBuf::new() } else { root.join(m) })).collect();
        let material_files = terrain.materials.iter().map(|m| if m.is_empty() { None } else { crate::textures::find(&root.join(m)) }).collect();

        drop(sp);
        let sun = find("Sun");
        let sky = find("Sky");
        let direction = sun
            .map(|s| s.floats("direction"))
            .filter(|v| v.len() == 3)
            .map(|v| Vec3::new(v[0], v[1], v[2]))
            .unwrap_or(Vec3::new(0.0, std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2));
        let sun_color = sun.map(|s| vec4(s, "color", Vec4::new(0.7, 0.7, 0.7, 1.0))).unwrap_or(Vec4::new(0.7, 0.7, 0.7, 1.0));
        let ambient = sun.map(|s| vec4(s, "ambient", Vec4::new(0.3, 0.3, 0.3, 1.0))).unwrap_or(Vec4::new(0.3, 0.3, 0.3, 1.0));
        let sun_light = crate::lighting::Sun::new(direction, sun_color.truncate(), ambient.truncate());
        let sun_dir = -sun_light.direction;
        let num = |o: Option<&SceneObject>, f: &str, d: f32| o.and_then(|o| o.get(f)).and_then(|v| v.parse().ok()).unwrap_or(d);

        let mut shapes = Vec::new();
        let mut hidden_shapes = Vec::new();
        let mut markers = Vec::new();
        let mut spawn = None;
        for o in all {
            let drawn = draw(o);
            let pos = match o.floats("position").as_slice() {
                [x, y, z, ..] => Vec3::new(*x, *y, *z),
                _ => continue,
            };
            let block = o.get("def").and_then(|db| defs.iter().find(|b| b.name.eq_ignore_ascii_case(db)));
            let shape = o
                .get("shape")
                .map(str::to_string)
                .or_else(|| block.and_then(|b| b.get("shape").or_else(|| b.get("shape"))).map(str::to_string));
            if let Some(shape) = shape.filter(|s| !s.is_empty()) {
                let path = tt_assets::resolve(root, mission_dir, &shape);
                if tt_assets::find_ci(&path).is_some_and(|p| !p.to_string_lossy().contains("markers")) {
                    let flag = |v: Option<&str>| v.is_some_and(|v| v.eq_ignore_ascii_case("true") || v.parse::<f32>().is_ok_and(|n| n != 0.0));
                    let casts_shadow = o.class.eq_ignore_ascii_case("Prop")
                        || (o.class.eq_ignore_ascii_case("PowerUp") && flag(block.and_then(|b| b.get("static_shadow"))));
                    shapes.push(ShapeInstance {
                        path,
                        transform: transform(o, pos),
                        casts_shadow,
                        tree_lighting: flag(o.get("tree_lighting")),
                        light_boost: o.get("light_boost").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                    });
                    if !drawn {
                        hidden_shapes.push(shapes.len() - 1);
                    }
                    continue;
                }
            }
            let (color, half) = match o.class.as_str() {
                "SpawnPoint" => {
                    spawn.get_or_insert(pos);
                    ([0.1, 1.0, 0.2], 1.5)
                }
                "PowerUp" => ([1.0, 0.9, 0.1], 1.0),
                "Boost" => ([1.0, 0.3, 0.1], 1.0),
                "Prop" => ([0.6, 0.6, 0.65], 2.0),
                "Volcano" => ([1.0, 0.0, 0.0], 4.0),
                _ => continue,
            };
            if drawn {
                push_box(&mut markers, pos + Vec3::Z * half, half, color);
            }
        }

        let sp = crate::loadstats::span(|| "scene: sky".into());
        let sky_faces = sky.and_then(|s| s.get("material_list")).map(|dml| load_sky(root, mission_dir, dml)).unwrap_or_default();

        let name = find("Info").and_then(|o| o.get("display_name")).unwrap_or("").to_string();
        drop(sp);
        let sp = crate::loadstats::span(|| format!("scene: static shapes ({})", shapes.len()));
        let mut shape_set = crate::shapes::ShapeSet::load(&shapes);
        drop(sp);
        let tile = tb.get("tile").is_some_and(|v| v.trim() != "0" && !v.eq_ignore_ascii_case("false"));
        let started = std::time::Instant::now();
        let light = light_scene(&terrain, square_size, o, tile, &sun_light, &shapes, &mut shape_set);
        let lighting_time = started.elapsed();
        crate::loadstats::note(|| format!("scene: lighting bake: {:.1} ms", lighting_time.as_secs_f64() * 1e3));
        // Left-out objects took part in the lighting; collapse their instances.
        for &i in &hidden_shapes {
            if let Some(placed) = shape_set.placed[i] {
                shape_set.instances[placed.slot as usize].model = [[0.0; 4]; 4];
            }
        }
        if std::env::var_os("TT_LIGHTING_LOG").is_some() {
            eprintln!("lighting: baked in {:.1} ms", lighting_time.as_secs_f64() * 1e3);
        }
        Ok(Scene {
            name,
            shapes: shape_set,
            sky_vertices: if sky_faces.is_empty() { Vec::new() } else { sky_cube() },
            sky_faces,
            terrain_vertices: vertices,
            terrain_indices: indices,
            terrain_size: n as u32,
            alpha,
            materials,
            material_files,
            sun_dir,
            lightmap: light.rgba8(),
            lightmap_size: light.size as u32,
            detail,
            sun: sun_light,
            light,
            lighting_time,
            terrain_origin: o,
            square_size,
            sun_color: sun_light.color.extend(1.0),
            ambient: sun_light.ambient.extend(1.0),
            fog_color: sky.map(|s| vec4(s, "fog_color", Vec4::splat(0.7))).unwrap_or(Vec4::splat(0.7)),
            visible_distance: num(sky, "visible_distance", 500.0),
            fog_distance: num(sky, "fog_distance", 100.0),
            fog_volumes: (1..=3)
                .filter_map(|i| match sky?.floats(&format!("fog_volume{i}")).as_slice() {
                    [vis, lo, hi] if *vis > 1.0 && hi > lo => Some([*vis, *lo, *hi]),
                    _ => None,
                })
                .collect(),
            markers,
            spawn,
            terrain,
            fx: crate::fx::FxLibrary::from_defs(root, defs),
        })
    }
}

/// Grid mesh covering [-half_extent, half_extent]^2 in world space.
fn terrain_mesh(t: &Terrain, square: f32, origin: (f32, f32), half_extent: f32) -> (Vec<TerrainVertex>, Vec<u32>) {
    let g0x = ((-half_extent - origin.0) / square).floor() as i64;
    let g0y = ((-half_extent - origin.1) / square).floor() as i64;
    let cells = (2.0 * half_extent / square).ceil() as i64;
    let verts_per_row = cells + 1;
    let mut vertices = Vec::with_capacity((verts_per_row * verts_per_row) as usize);
    for j in 0..=cells {
        for i in 0..=cells {
            let (gx, gy) = (g0x + i, g0y + j);
            let h = t.height(gx, gy);
            let dx = t.height(gx + 1, gy) - t.height(gx - 1, gy);
            let dy = t.height(gx, gy + 1) - t.height(gx, gy - 1);
            let normal = Vec3::new(-dx, -dy, 2.0 * square).normalize();
            vertices.push(TerrainVertex {
                pos: [origin.0 + gx as f32 * square, origin.1 + gy as f32 * square, h],
                normal: normal.into(),
                grid: [gx as f32, gy as f32],
            });
        }
    }
    let mut indices = Vec::with_capacity((cells * cells * 6) as usize);
    for j in 0..cells {
        for i in 0..cells {
            let a = (j * verts_per_row + i) as u32;
            let b = a + 1;
            let c = a + verts_per_row as u32;
            let d = c + 1;
            // The engine's checkerboard split: square (x, y) is cut along
            // (0,0)–(1,1) when (x ^ y) & 1 == 0, else along (1,0)–(0,1). With
            // one diagonal everywhere, half the squares rendered up to ~0.9 m
            // off the surface the sim stands objects on, so pads and tanks on
            // rough slopes looked half buried or floating.
            if ((g0x + i) ^ (g0y + j)) & 1 == 0 {
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            } else {
                indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
    }
    (vertices, indices)
}

fn push_box(out: &mut Vec<ColorVertex>, c: Vec3, h: f32, color: [f32; 3]) {
    let corners: Vec<Vec3> = (0..8)
        .map(|i| c + Vec3::new(if i & 1 == 0 { -h } else { h }, if i & 2 == 0 { -h } else { h }, if i & 4 == 0 { -h } else { h }))
        .collect();
    const FACES: [[usize; 4]; 6] = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    for (fi, f) in FACES.iter().enumerate() {
        let shade = [0.6, 1.0, 0.75, 0.85, 0.7, 0.9][fi];
        let col = [color[0] * shade, color[1] * shade, color[2] * shade];
        for idx in [0, 1, 2, 0, 2, 3] {
            out.push(ColorVertex { pos: corners[f[idx]].into(), color: col });
        }
    }
}

/// Sky material list (`.dml`): one texture name per line, relative to the
/// list. Entries 0-3 are the side panorama, 4 the top, then cloud layers.
/// The faces come back with their shared edges matched ([`crate::sky`]).
fn load_sky(root: &Path, mission_dir: &Path, dml: &str) -> Vec<Vec<u8>> {
    let path = tt_assets::resolve(root, mission_dir, dml);
    if latin1(&path).is_err() {
        eprintln!("warning: sky list not found: {}", path.display());
        return Vec::new();
    }
    let mut faces: Vec<Vec<u8>> = sky_bases(root, mission_dir, dml).unwrap_or_default().iter().map(|b| load_material(b)).collect();
    crate::sky::match_seams(&mut faces, crate::textures::size() as usize);
    faces
}

/// The five sky textures (without extension) of a `.dml` list, if it has them.
pub(crate) fn sky_bases(root: &Path, mission_dir: &Path, dml: &str) -> Option<Vec<PathBuf>> {
    let path = tt_assets::resolve(root, mission_dir, dml);
    let text = latin1(&path).ok()?;
    let dir = path.parent()?;
    let names: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    (names.len() >= 5).then(|| names[..5].iter().map(|n| dir.join(n)).collect())
}

/// Unit cube around the camera. Sides face north, east, south, west in
/// texture order (the panorama runs clockwise seen from above), then the top.
pub(crate) fn sky_cube() -> Vec<SkyVertex> {
    let mut v = Vec::new();
    let sides = [
        (Vec3::new(-1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 0.0)),   // north: west -> east
        (Vec3::new(1.0, 1.0, 0.0), Vec3::new(1.0, -1.0, 0.0)),   // east
        (Vec3::new(1.0, -1.0, 0.0), Vec3::new(-1.0, -1.0, 0.0)), // south
        (Vec3::new(-1.0, -1.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)), // west
    ];
    for (layer, (a, b)) in sides.iter().enumerate() {
        let quad = [
            (*a + Vec3::Z, [0.0, 0.0]),
            (*b + Vec3::Z, [1.0, 0.0]),
            (*b - Vec3::Z, [1.0, 1.0]),
            (*a - Vec3::Z, [0.0, 1.0]),
        ];
        for i in [0, 1, 2, 0, 2, 3] {
            v.push(SkyVertex { dir: quad[i].0.into(), uv: quad[i].1, layer: layer as u32 });
        }
    }
    let top = [
        (Vec3::new(-1.0, -1.0, 1.0), [0.0, 1.0]),
        (Vec3::new(1.0, -1.0, 1.0), [1.0, 1.0]),
        (Vec3::new(1.0, 1.0, 1.0), [1.0, 0.0]),
        (Vec3::new(-1.0, 1.0, 1.0), [0.0, 0.0]),
    ];
    for i in [0, 1, 2, 0, 2, 3] {
        v.push(SkyVertex { dir: top[i].0.into(), uv: top[i].1, layer: 4 });
    }
    v
}

fn transform(o: &SceneObject, pos: Vec3) -> Mat4 {
    let rot = match o.floats("rotation").as_slice() {
        [x, y, z, deg] if Vec3::new(*x, *y, *z).length_squared() > 0.0 => {
            Quat::from_axis_angle(Vec3::new(*x, *y, *z).normalize(), -deg.to_radians())
        }
        _ => Quat::IDENTITY,
    };
    let scale = match o.floats("scale").as_slice() {
        [x, y, z] => Vec3::new(*x, *y, *z),
        _ => Vec3::ONE,
    };
    Mat4::from_scale_rotation_translation(scale, rot, pos)
}

impl Scene {
    /// A scene with nothing in it: a 2×2 flat terrain and no sky, shapes or
    /// fog. Used by [`crate::objectview::ObjectView`], which draws only
    /// dynamic shapes.
    pub(crate) fn empty() -> Scene {
        let terrain = Terrain {
            version: 0,
            block_shift: 1,
            heights: vec![0; 4],
            header: [0; 8],
            material_map: vec![0; 4],
            materials: Default::default(),
            alpha: Default::default(),
            texture_script: String::new(),
            heightfield_script: String::new(),
        };
        let layer = vec![128u8; (crate::textures::size() * crate::textures::size() * 4) as usize];
        let sun = crate::lighting::Sun::new(Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO, Vec3::ONE);
        let v = |x: f32, y: f32| TerrainVertex { pos: [x, y, -1e4], normal: [0.0, 0.0, 1.0], grid: [0.0, 0.0] };
        Scene {
            name: String::new(),
            shapes: crate::shapes::ShapeSet::load(&[]),
            sky_faces: Vec::new(),
            sky_vertices: Vec::new(),
            terrain_vertices: vec![v(0.0, 0.0), v(1.0, 0.0), v(0.0, 1.0)],
            terrain_indices: vec![0, 1, 2],
            terrain_size: 2,
            alpha: [vec![255u8; 16], vec![0u8; 16]],
            materials: vec![layer.clone(), layer],
            material_files: vec![None, None],
            sun_dir: -sun.direction,
            lightmap: vec![255u8; 16],
            lightmap_size: 2,
            detail: None,
            light: crate::lighting::Lightmap::uniform(Vec3::ONE, 2, 1.0, 0.0),
            sun,
            lighting_time: std::time::Duration::ZERO,
            terrain_origin: 0.0,
            square_size: 1.0,
            sun_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
            ambient: Vec4::ONE,
            fog_color: Vec4::new(0.0, 0.0, 0.0, 0.0),
            visible_distance: 1e6,
            fog_distance: 1e6,
            fog_volumes: Vec::new(),
            markers: Vec::new(),
            spawn: None,
            terrain,
            fx: Default::default(),
        }
    }

    /// Sun diffuse scale `k` for a moving shape at `pos` (engine-notes §1.4:
    /// lightmap under the object minus the average ambient). Static shapes
    /// get theirs at load time in [`ShapeSet::instances`](crate::shapes::ShapeSet).
    pub fn shape_light(&self, pos: Vec3) -> f32 {
        crate::lighting::dynamic_factor(&self.sun, &self.light, pos)
    }

    /// Terrain height in metres at a world position (bilinear; the block repeats).
    pub fn terrain_height(&self, x: f32, y: f32) -> f32 {
        let g = (glam::Vec2::new(x, y) - glam::Vec2::splat(self.terrain_origin)) / self.square_size;
        crate::lighting::height_at(&self.terrain, g.x, g.y)
    }
}

/// Bakes the terrain lightmap with static shape shadows and writes every
/// shape instance's diffuse factor.
fn light_scene(
    terrain: &Terrain,
    square: f32,
    origin: f32,
    tile: bool,
    sun: &crate::lighting::Sun,
    inputs: &[ShapeInstance],
    set: &mut crate::shapes::ShapeSet,
) -> crate::lighting::Lightmap {
    use crate::lighting::{Caster, StaticObject};
    let ti = crate::lighting::TerrainInfo { terrain, square, origin, tile };
    let mut casters = Vec::new();
    let mut objects = Vec::new();
    for (input, placed) in inputs.iter().zip(&set.placed) {
        let Some(placed) = placed else { continue };
        let g = &set.geometry[placed.geometry];
        let m = input.transform;
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for i in 0..8 {
            let p = m.transform_point3(Vec3::new(
                if i & 1 == 0 { g.min.x } else { g.max.x },
                if i & 2 == 0 { g.min.y } else { g.max.y },
                if i & 4 == 0 { g.min.z } else { g.max.z },
            ));
            min = min.min(p);
            max = max.max(p);
        }
        let caster = (input.casts_shadow && !g.shadow.is_empty()).then(|| {
            casters.push(Caster::new(g.shadow.iter().map(|t| t.map(|p| m.transform_point3(p))).collect()));
            casters.len() - 1
        });
        objects.push((placed.slot, StaticObject { min, max, caster, tree_lighting: input.tree_lighting, light_boost: input.light_boost }));
    }
    let baked = crate::lighting::bake(&ti, sun, &casters);
    for (slot, obj) in &objects {
        set.instances[*slot as usize].light[0] = crate::lighting::static_factor(&ti, sun, &baked.terrain_only, &casters, obj);
    }
    if std::env::var_os("TT_LIGHTING_LOG").is_some() {
        eprintln!("lighting: {}² lightmap, {} casters, {} shapes", baked.lightmap.size, casters.len(), objects.len());
    }
    baked.lightmap
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every square is cut along the diagonal the engine (and tt_game's
    /// heightfield) uses: (0,0)–(1,1) when (x ^ y) & 1 == 0, else
    /// (1,0)–(0,1).
    #[test]
    fn terrain_mesh_uses_the_checkerboard_split() {
        let t = Terrain {
            version: 0,
            block_shift: 3,
            heights: (0..64u16).map(|i| (i * 37 % 11) * 32).collect(),
            header: [0; 8],
            material_map: Vec::new(),
            materials: Default::default(),
            alpha: Default::default(),
            texture_script: String::new(),
            heightfield_script: String::new(),
        };
        let (v, idx) = terrain_mesh(&t, 8.0, (-32.0, -32.0), 20.0);
        assert!(idx.len() >= 6 * 25);
        for cell in idx.chunks(6) {
            let g = |i: u32| (v[i as usize].grid[0] as i64, v[i as usize].grid[1] as i64);
            let (t1, t2) = (&cell[..3], &cell[3..]);
            let shared: Vec<(i64, i64)> = t1.iter().filter(|i| t2.contains(i)).map(|&i| g(i)).collect();
            assert_eq!(shared.len(), 2);
            let (x0, y0) = (shared[0].0.min(shared[1].0), shared[0].1.min(shared[1].1));
            let rising = (shared[0].0 - shared[1].0) * (shared[0].1 - shared[1].1) > 0;
            assert_eq!(rising, (x0 ^ y0) & 1 == 0, "square ({x0}, {y0})");
            // Counter-clockwise seen from above (front faces up).
            for tri in [t1, t2] {
                let p: Vec<Vec3> = tri.iter().map(|&i| Vec3::new(v[i as usize].pos[0], v[i as usize].pos[1], 0.0)).collect();
                assert!((p[1] - p[0]).cross(p[2] - p[0]).z > 0.0);
            }
        }
    }
}
