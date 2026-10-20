struct Globals {
    view_proj: mat4x4<f32>,
    camera: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    fog_color: vec4<f32>,
    // x: fog start, y: visible distance, z: terrain size (samples), w: metres per material tile
    params: vec4<f32>,
    // x: seconds, y: IFL ticks per second, z: shadow casters, w: near plane
    time: vec4<f32>,
    // x: terrain grid origin, y: metres per square, z: samples per edge
    terrain: vec4<f32>,
    // Height fog bands: x visible distance (0 = unused), y min height, z max height.
    fog_volumes: array<vec4<f32>, 3>,
    // Camera basis for billboards.
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    cam_forward: vec4<f32>,
    // Graphics extras: x normal-map strength (0 off), y glass on, z
    // emissive boost (1 off), w sun glow.
    extras: vec4<f32>,
    // Dynamic point lights: x count; per light xyz position + w radius, then
    // rgb colour (crate::render::PointLight). y: shadow atlas tile texels,
    // z: shadow quality, w: far plane.
    light_info: vec4<f32>,
    lights: array<vec4<f32>, 32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var alpha_lo: texture_2d<f32>;
@group(0) @binding(2) var alpha_hi: texture_2d<f32>;
@group(0) @binding(3) var materials: texture_2d_array<f32>;
@group(0) @binding(4) var samp: sampler;
// Baked terrain lightmap (sRGB: sampling decodes the display-space factor).
@group(0) @binding(5) var lightmap: texture_2d<f32>;
@group(0) @binding(6) var detail_tex: texture_2d<f32>;
@group(0) @binding(7) var<storage, read> casters: array<ShadowCaster>;
@group(0) @binding(8) var shadow_atlas: texture_2d<f32>;
// Normal maps of the terrain materials (same layers as `materials`), with
// their own trilinear sampler (anisotropic filtering of eight normal layers
// is the costly part on integrated GPUs, and relief fades with distance).
@group(0) @binding(9) var terrain_normals: texture_2d_array<f32>;
@group(0) @binding(10) var normal_samp: sampler;

// Terrain normal maps fade out between these distances (m): relief is not
// visible further away, and the samples are skipped there.
const NORMAL_NEAR: f32 = 45.0;
const NORMAL_FAR: f32 = 90.0;
// Contrast of the normal-map relief.
const RELIEF_GAIN: f32 = 3.0;

// Light of the dynamic point lights at `world` on a surface facing `n`
// (N·L, smooth (1 - (d/r)^2)^2 falloff to zero at the radius; the sum is
// clamped so overlapping lights never wash a surface out).
fn point_lights(world: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let count = u32(g.light_info.x);
    for (var k = 0u; k < count; k++) {
        let pr = g.lights[2u * k];
        let d = pr.xyz - world;
        let q = dot(d, d) / (pr.w * pr.w);
        if (q < 1.0) {
            let f = (1.0 - q) * (1.0 - q);
            let ndl = max(dot(n, d * inverseSqrt(max(dot(d, d), 1e-4))), 0.0);
            sum += g.lights[2u * k + 1u].rgb * (f * ndl);
        }
    }
    return min(sum, vec3<f32>(1.0));
}

// Shading factor of a normal-mapped surface: the relief (mapped normal `np`
// against the geometric `ng`) lit by the sun's azimuth at a fixed moderate
// elevation, so the relief reads even with a high sun or a baked lightmap,
// plus a little cavity darkening of the tilted texels. 1 = flat.
fn relief(np: vec3<f32>, ng: vec3<f32>) -> f32 {
    let l = g.sun_dir.xyz;
    var lh = l - ng * dot(l, ng);
    if (dot(lh, lh) < 1e-4) {
        // Sun straight above: any fixed azimuth.
        lh = vec3<f32>(0.57, 0.57, 0.0) - ng * dot(vec3<f32>(0.57, 0.57, 0.0), ng);
    }
    let lr = normalize(normalize(lh) + ng * 0.9);
    let k = 1.0 + (dot(np, lr) - dot(ng, lr)) * RELIEF_GAIN;
    let cavity = mix(1.0, clamp(dot(np, ng), 0.0, 1.0), 0.7);
    return clamp(k * cavity, 0.3, 1.8);
}

// Terrain detail texture: modulates 2 x detail close to the camera and fades
// out with distance.
const DETAIL_TILE: f32 = 4.0;
const DETAIL_FADE: f32 = 60.0;

fn terrain_light(world: vec3<f32>) -> vec3<f32> {
    let grid = (world.xy - vec2<f32>(g.terrain.x)) / g.terrain.y;
    return textureSample(lightmap, samp, grid / g.terrain.z).rgb;
}

// Display-space value to linear, matching the sRGB texture decode.
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn haze(t: f32) -> f32 {
    let c = clamp(t, 0.0, 1.0);
    return 1.0 - (c - 1.0) * (c - 1.0);
}

// Each height fog band adds, linearly, the length of the view ray inside it
// divided by the band's visible distance (percentage 1, visibleDistanceMod
// 1); the sum is clamped to 1. (Keep this free of early returns and
// `continue`: a version with them drew no fog at all on Mesa/Intel.)
fn fog_factor(world: vec3<f32>) -> f32 {
    let cam = g.camera.xyz;
    let d = distance(world, cam);
    let h = select(haze((d - g.params.x) / max(g.params.y - g.params.x, 1.0)), 1.0, d >= g.params.y);
    var bands = 0.0;
    for (var k = 0; k < 3; k++) {
        let v = g.fog_volumes[k];
        if (v.x > 0.0) {
            let z0 = min(cam.z, world.z);
            let z1 = max(cam.z, world.z);
            var inside = 0.0;
            if (z1 - z0 < 0.01) {
                inside = select(0.0, d, z0 >= v.y && z0 <= v.z);
            } else {
                inside = d * max(min(z1, v.z) - max(z0, v.y), 0.0) / (z1 - z0);
            }
            bands = bands + inside / v.x;
        }
    }
    return clamp(h + bands, 0.0, 1.0);
}

struct TerrainOut {
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) grid: vec2<f32>,
};

@vertex
fn vs_terrain(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) grid: vec2<f32>) -> TerrainOut {
    var o: TerrainOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    o.world = pos;
    o.normal = normal;
    o.grid = grid;
    return o;
}

@fragment
fn fs_terrain(i: TerrainOut) -> @location(0) vec4<f32> {
    // Alpha maps are sampled at texel centres of the wrapped grid.
    let auv = (i.grid + vec2<f32>(0.5)) / g.params.z;
    let w0 = textureSample(alpha_lo, samp, auv);
    let w1 = textureSample(alpha_hi, samp, auv);
    let muv = i.world.xy / g.params.w;
    // Only layers with weight here are sampled (explicit gradients keep the
    // mip/anisotropy choice valid inside the branches).
    let gx = dpdx(muv);
    let gy = dpdy(muv);
    let duv = i.world.xy / DETAIL_TILE;
    let dgx = dpdx(duv);
    let dgy = dpdy(duv);
    let baked = terrain_light(i.world);
    let fog = fog_factor(i.world);
    if (fog >= 1.0) {
        // Fully fogged (beyond the visible distance, or deep in a fog
        // band): the fog colour, which is all the mix below leaves; the
        // material samples are skipped. (Everything that needs implicit
        // derivatives is above, in uniform control flow.)
        return vec4<f32>(g.fog_color.rgb, 1.0);
    }
    var c = vec3<f32>(0.0);
    if (w0.x > 0.0) { c += w0.x * textureSampleGrad(materials, samp, muv, 0, gx, gy).rgb; }
    if (w0.y > 0.0) { c += w0.y * textureSampleGrad(materials, samp, muv, 1, gx, gy).rgb; }
    if (w0.z > 0.0) { c += w0.z * textureSampleGrad(materials, samp, muv, 2, gx, gy).rgb; }
    if (w0.w > 0.0) { c += w0.w * textureSampleGrad(materials, samp, muv, 3, gx, gy).rgb; }
    if (w1.x > 0.0) { c += w1.x * textureSampleGrad(materials, samp, muv, 4, gx, gy).rgb; }
    if (w1.y > 0.0) { c += w1.y * textureSampleGrad(materials, samp, muv, 5, gx, gy).rgb; }
    if (w1.z > 0.0) { c += w1.z * textureSampleGrad(materials, samp, muv, 6, gx, gy).rgb; }
    if (w1.w > 0.0) { c += w1.w * textureSampleGrad(materials, samp, muv, 7, gx, gy).rgb; }
    let total = dot(w0, vec4<f32>(1.0)) + dot(w1, vec4<f32>(1.0));
    c = c / max(total, 0.001);
    // Pure modulate by the baked lightmap, no overbright (engine-notes §1.3).
    var lit = c * baked;
    var shading_n = normalize(i.normal);
    let dist = distance(i.world, g.camera.xyz);
    let nfade = 1.0 - smoothstep(NORMAL_NEAR, NORMAL_FAR, dist);
    if (g.extras.x > 0.0 && nfade > 0.0) {
        // Normal maps (graphics extra): the relief of the perturbed normal
        // scales the baked light.
        var t = vec3<f32>(0.0);
        if (w0.x > 0.0) { t += w0.x * textureSampleGrad(terrain_normals, normal_samp, muv, 0, gx, gy).rgb; }
        if (w0.y > 0.0) { t += w0.y * textureSampleGrad(terrain_normals, normal_samp, muv, 1, gx, gy).rgb; }
        if (w0.z > 0.0) { t += w0.z * textureSampleGrad(terrain_normals, normal_samp, muv, 2, gx, gy).rgb; }
        if (w0.w > 0.0) { t += w0.w * textureSampleGrad(terrain_normals, normal_samp, muv, 3, gx, gy).rgb; }
        if (w1.x > 0.0) { t += w1.x * textureSampleGrad(terrain_normals, normal_samp, muv, 4, gx, gy).rgb; }
        if (w1.y > 0.0) { t += w1.y * textureSampleGrad(terrain_normals, normal_samp, muv, 5, gx, gy).rgb; }
        if (w1.z > 0.0) { t += w1.z * textureSampleGrad(terrain_normals, normal_samp, muv, 6, gx, gy).rgb; }
        if (w1.w > 0.0) { t += w1.w * textureSampleGrad(terrain_normals, normal_samp, muv, 7, gx, gy).rgb; }
        let tn = t / max(total, 0.001) * 2.0 - vec3<f32>(1.0);
        let ng = normalize(i.normal);
        // Tangent frame of the world-XY material mapping (u = +x, v = +y;
        // the image +Y points to -v).
        let tx = normalize(vec3<f32>(1.0, 0.0, 0.0) - ng * ng.x);
        let ty = cross(ng, tx);
        let s = g.extras.x * nfade;
        let np = normalize(tx * tn.x * s - ty * tn.y * s + ng * max(tn.z, 0.05));
        lit = lit * relief(np, ng);
        shading_n = np;
    }
    if (g.light_info.x > 0.0) {
        // Mostly on the surface colour, plus a little of the light's own
        // colour (a cyan glow still shows on saturated blue ground).
        lit += (c + vec3<f32>(0.25)) * point_lights(i.world, shading_n);
    }
    let fade = 1.0 - clamp(dist / DETAIL_FADE, 0.0, 1.0);
    if (fade > 0.0) {
        let d = textureSampleGrad(detail_tex, samp, duv, dgx, dgy).rgb * 2.0;
        lit = lit * mix(vec3<f32>(1.0), d, fade);
    }
    // The darkening works on display values like the game's blend.
    return vec4<f32>(mix(lit, g.fog_color.rgb, fog), 1.0);
}

// The terrain's depth pre-pass (colour writes are masked off).
@fragment
fn fs_depth_only() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

@group(1) @binding(0) var sky_tex: texture_2d_array<f32>;
@group(1) @binding(1) var sky_samp: sampler;

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) dir: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) layer: u32,
};

@vertex
fn vs_sky(@location(0) dir: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) layer: u32) -> SkyOut {
    var o: SkyOut;
    let p = g.view_proj * vec4<f32>(g.camera.xyz + dir, 1.0);
    // Pin the sky to the far plane.
    o.clip = vec4<f32>(p.xy, p.w, p.w);
    o.dir = dir;
    o.uv = uv;
    o.layer = layer;
    return o;
}

@fragment
fn fs_sky(i: SkyOut) -> @location(0) vec4<f32> {
    let c = textureSample(sky_tex, sky_samp, i.uv, i.layer).rgb;
    let up = normalize(i.dir).z;
    let f = 1.0 - smoothstep(0.0, 0.35, up);
    var col = mix(c, g.fog_color.rgb, f);
    if (g.extras.w > 0.0) {
        // Sun glow (HDR extra): a bright core and a soft halo around the sun.
        let s = max(dot(normalize(i.dir), g.sun_dir.xyz), 0.0);
        col += g.sun_color.rgb * g.extras.w * (pow(s, 900.0) * 6.0 + pow(s, 24.0) * 0.35);
    }
    return vec4<f32>(col, 1.0);
}

@group(1) @binding(0) var shape_tex: texture_2d_array<f32>;
@group(1) @binding(1) var shape_samp: sampler;
@group(1) @binding(2) var<storage, read> timeline: array<u32>;
// Normal maps per texture layer (graphics extra; flat when off).
@group(1) @binding(3) var shape_normals: texture_2d_array<f32>;
@group(1) @binding(4) var shape_normal_samp: sampler;
// The opaque scene behind blended surfaces (glass refraction).
@group(3) @binding(0) var scene_behind: texture_2d<f32>;
@group(3) @binding(1) var behind_samp: sampler;
@group(2) @binding(0) var<storage, read> poses: array<mat4x4<f32>>;

struct ShapeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) layer: u32,
    @location(4) @interpolate(flat) flags: u32,
    // Sun diffuse scale k of this instance.
    @location(5) @interpolate(flat) light: f32,
};

// World position (and normal) of a shape vertex.
struct Posed {
    world: vec4<f32>,
    normal: vec3<f32>,
};

fn pose_vertex(pos: vec3<f32>, normal: vec3<f32>, flags: u32, model_in: mat4x4<f32>, object: u32, pose: vec4<u32>, bones: vec4<u32>, weights: vec4<f32>) -> Posed {
    var model = model_in;
    // Posed instances: per-object matrix (node pose relative to the default
    // pose; zero for hidden objects, which collapses them).
    if (pose.x != 0xffffffffu) {
        let obj = poses[pose.x + object];
        if ((flags & 8u) != 0u) {
            // Skinned: blend the bones' default-to-posed deltas; the object
            // matrix only says whether it is visible.
            var skin = poses[pose.z + bones.x] * weights.x + poses[pose.z + bones.y] * weights.y
                + poses[pose.z + bones.z] * weights.z + poses[pose.z + bones.w] * weights.w;
            if (obj[3][3] == 0.0) {
                skin = mat4x4<f32>(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
            }
            model = model * skin;
        } else {
            model = model * obj;
        }
    }
    var o: Posed;
    if ((flags & 2u) != 0u) {
        // Billboard (TSMesh billboard): node-local x/y/z map to camera
        // right/forward/up around the node origin, keeping the node's scale.
        var fwd = g.cam_forward.xyz;
        var up = g.cam_up.xyz;
        var right = g.cam_right.xyz;
        if ((flags & 4u) != 0u) {
            up = vec3<f32>(0.0, 0.0, 1.0);
            fwd = normalize(vec3<f32>(fwd.xy, 0.0) + vec3<f32>(0.0, 1e-5, 0.0));
            right = normalize(cross(fwd, up));
        }
        let sc = vec3<f32>(length(model[0].xyz), length(model[1].xyz), length(model[2].xyz));
        let p = pos * sc;
        o.world = vec4<f32>(model[3].xyz + right * p.x + fwd * p.y + up * p.z, 1.0);
        o.normal = -fwd;
    } else {
        o.world = model * vec4<f32>(pos, 1.0);
        o.normal = (model * vec4<f32>(normal, 0.0)).xyz;
    }
    return o;
}

@vertex
fn vs_shape(
    @location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
    @location(3) tl_start: u32, @location(4) tl_len: u32, @location(5) flags: u32,
    @location(6) m0: vec4<f32>, @location(7) m1: vec4<f32>, @location(8) m2: vec4<f32>, @location(9) m3: vec4<f32>,
    @location(10) light: vec4<f32>, @location(11) object: u32, @location(12) pose: vec4<u32>,
    @location(13) shadow: vec4<f32>, @location(14) bones: vec4<u32>, @location(15) weights: vec4<f32>,
) -> ShapeOut {
    let p = pose_vertex(pos, normal, flags, mat4x4<f32>(m0, m1, m2, m3), object, pose, bones, weights);
    var o: ShapeOut;
    o.clip = g.view_proj * p.world;
    o.world = p.world.xyz;
    o.normal = p.normal;
    o.uv = uv;
    var tick = u32(g.time.x * g.time.y);
    // Per-instance IFL time (material index in flags bits 8..15).
    let slot = (flags >> 8u) & 0xffu;
    if (pose.x != 0xffffffffu && slot < 16u) {
        let t = poses[pose.w + slot / 16u][slot / 4u % 4u][slot % 4u];
        if (t >= 0.0) {
            tick = u32(t);
        }
    }
    o.layer = timeline[tl_start + tick % max(tl_len, 1u)];
    o.flags = flags;
    o.light = light.x;
    return o;
}

struct ShadowCaster {
    // xyz: shape centre, w: box radius
    center: vec4<f32>,
    // xyz: light direction, w: shadow length
    dir: vec4<f32>,
    // xyz: axis across the light, w: opacity
    u: vec4<f32>,
    // xyz: second axis, w: atlas tile
    v: vec4<f32>,
};

const ATLAS_TILES: f32 = 8.0;

// Silhouette of a caster's shadow detail seen along the light, into its
// atlas tile (the viewport selects the tile).
@vertex
fn vs_shadow_atlas(
    @location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
    @location(3) tl_start: u32, @location(4) tl_len: u32, @location(5) flags: u32,
    @location(6) m0: vec4<f32>, @location(7) m1: vec4<f32>, @location(8) m2: vec4<f32>, @location(9) m3: vec4<f32>,
    @location(10) light: vec4<f32>, @location(11) object: u32, @location(12) pose: vec4<u32>,
    @location(13) shadow: vec4<f32>, @location(14) bones: vec4<u32>, @location(15) weights: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    let c = casters[pose.y];
    let p = pose_vertex(pos, normal, flags, mat4x4<f32>(m0, m1, m2, m3), object, pose, bones, weights).world.xyz - c.center.xyz;
    return vec4<f32>(dot(p, c.u.xyz) / c.center.w, dot(p, c.v.xyz) / c.center.w, 0.5, 1.0);
}

@fragment
fn fs_shadow_atlas() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0);
}

// Dynamic shadows on the terrain (Shadow::render: the silhouette projected
// along the light onto the receiving terrain polygons inside the caster's
// box). The terrain under each caster's box is drawn again with
// fs_terrain_shadow, which darkens what is there, so only those pixels pay.
fn caster_shadow(k: u32, world: vec3<f32>) -> f32 {
    let c = casters[k];
    let rel = world - c.center.xyz;
    let along = dot(rel, c.dir.xyz);
    if (along < -c.center.w || along > c.dir.w) {
        return 0.0;
    }
    let x = dot(rel, c.u.xyz) / c.center.w;
    let y = dot(rel, c.v.xyz) / c.center.w;
    if (abs(x) >= 1.0 || abs(y) >= 1.0) {
        return 0.0;
    }
    let tile = vec2<f32>(c.v.w % ATLAS_TILES, floor(c.v.w / ATLAS_TILES));
    let texel = 1.0 / g.light_info.y;
    let quality = g.light_info.z;
    if (quality < 0.5) {
        let local = clamp(vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5), vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
        return c.u.w * textureSampleLevel(shadow_atlas, samp, (tile + local) / ATLAS_TILES, 0.0).r;
    }
    // Soft / high: PCF over a disc that widens with the distance from the
    // caster along the light (contact-hard, softer further out), and the
    // shadow fades towards the end of its reach.
    let t = clamp((along + c.center.w) / max(c.dir.w, 1e-3), 0.0, 1.0);
    // (High: twice the texels, the same blur far out but a crisper contact
    // edge, and more taps.)
    let high = quality > 1.5;
    let radius = texel * select(mix(1.2, 5.0, t), mix(1.2, 10.0, t), high);
    let taps = select(8, 16, high);
    let centre = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    let lo = vec2<f32>(0.5 * texel);
    let hi = vec2<f32>(1.0 - 0.5 * texel);
    var sum = 0.0;
    // Vogel-disc taps, bilinear each.
    for (var k = 0; k < taps; k++) {
        let a = f32(k) * 2.39996 + 0.5;
        let r = sqrt((f32(k) + 0.5) / f32(taps)) * radius;
        let local = clamp(centre + vec2<f32>(cos(a), sin(a)) * r, lo, hi);
        sum += textureSampleLevel(shadow_atlas, samp, (tile + local) / ATLAS_TILES, 0.0).r;
    }
    return c.u.w * (sum / f32(taps)) * (1.0 - smoothstep(0.6, 1.0, t));
}

struct TerrainShadowOut {
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) @interpolate(flat) caster: u32,
};

@vertex
fn vs_terrain_shadow(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) grid: vec2<f32>, @builtin(instance_index) caster: u32) -> TerrainShadowOut {
    var o: TerrainShadowOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    o.world = pos;
    o.caster = caster;
    return o;
}

@fragment
fn fs_terrain_shadow(i: TerrainShadowOut) -> @location(0) vec4<f32> {
    // Blend: dst * (1 - alpha). Fog weakens the shadow.
    let s = caster_shadow(i.caster, i.world);
    let f = to_linear(vec3<f32>(1.0 - s)).x;
    return vec4<f32>(0.0, 0.0, 0.0, (1.0 - f) * (1.0 - fog_factor(i.world)));
}

// Normal of a shape pixel with the layer's normal map applied (cotangent
// frame from screen derivatives, no vertex tangents needed).
// Skipped (the flat normal) with the extra off and beyond NORMAL_FAR.
fn mapped_normal(i: ShapeOut) -> vec3<f32> {
    let n = normalize(i.normal);
    if (g.extras.x <= 0.0) {
        return n;
    }
    let dp1 = dpdx(i.world);
    let dp2 = dpdy(i.world);
    let duv1 = dpdx(i.uv);
    let duv2 = dpdy(i.uv);
    let s = g.extras.x * (1.0 - smoothstep(NORMAL_NEAR, NORMAL_FAR, distance(i.world, g.camera.xyz)));
    if (s <= 0.0) {
        return n;
    }
    let tn = textureSampleGrad(shape_normals, shape_normal_samp, i.uv, i.layer, duv1, duv2).rgb * 2.0 - vec3<f32>(1.0);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let invmax = inverseSqrt(max(max(dot(t, t), dot(b, b)), 1e-12));
    // Image +Y is up, texture v grows down.
    let p = t * invmax * tn.x * s - b * invmax * tn.y * s + n * max(tn.z, 0.05);
    return normalize(p);
}

fn shade(i: ShapeOut, t: vec4<f32>) -> vec3<f32> {
    return shade_n(i, t, normalize(i.normal));
}

fn shade_n(i: ShapeOut, t: vec4<f32>, n: vec3<f32>) -> vec3<f32> {
    var light = vec3<f32>(g.extras.z);
    if ((i.flags & 1u) == 0u) {
        // Fixed-function GL sun (engine-notes §1.4): directional light at
        // -direction, ambient = Sun.ambient, diffuse = colour * k, the sum
        // clamped like a GL colour and applied in display space. (GL lit per
        // vertex; we evaluate N.L per pixel.)
        let ng = normalize(i.normal);
        let d = g.ambient.rgb + g.sun_color.rgb * i.light * max(dot(ng, g.sun_dir.xyz), 0.0);
        light = to_linear(clamp(d, vec3<f32>(0.0), vec3<f32>(1.0)));
        if (g.extras.x > 0.0) {
            // Normal maps: the relief as a factor (the GL light is often
            // clamped at 1), a bit weaker in shadow (`i.light`).
            light = light * mix(1.0, relief(n, ng), 0.75 + 0.25 * clamp(i.light, 0.0, 1.0));
        }
        if (g.light_info.x > 0.0) {
            light += point_lights(i.world, n);
        }
    }
    return mix(t.rgb * light, g.fog_color.rgb, fog_factor(i.world));
}

@fragment
fn fs_shape(i: ShapeOut) -> @location(0) vec4<f32> {
    let n = mapped_normal(i);
    let t = textureSample(shape_tex, shape_samp, i.uv, i.layer);
    if (t.a < 0.5) {
        discard;
    }
    return vec4<f32>(shade_n(i, t, n), 1.0);
}

@fragment
fn fs_shape_blend(i: ShapeOut, @builtin(front_facing) ccw: bool) -> @location(0) vec4<f32> {
    let n = mapped_normal(i);
    let t = textureSample(shape_tex, shape_samp, i.uv, i.layer);
    let fog = fog_factor(i.world);
    // Face normal from the surface itself (the jar's stored normals are not
    // usable for reflections).
    let facet = cross(dpdx(i.world), dpdy(i.world));
    // Blended surfaces fade out with the fog instead of turning fog-coloured.
    var out = vec4<f32>(shade_n(i, t, n), t.a * (1.0 - fog));
    if (g.extras.y > 0.0 && (i.flags & 16u) != 0u) {
        // Glass (graphics extra): the scene behind, shifted along the
        // normal, tinted by the glass texture, plus a Fresnel reflection of
        // the sky and a sun highlight.
        let v = normalize(g.camera.xyz - i.world);
        var nn = normalize(facet + vec3<f32>(0.0, 0.0, 1e-9));
        // Back faces keep what is there: each glass pixel replaces the
        // scene behind, so only the front face may write.
        // (DTS front faces are clockwise, so they are the non-CCW ones.)
        let back = ccw;
        if (dot(nn, v) < 0.0) {
            nn = -nn;
        }
        let ndc = g.view_proj * vec4<f32>(i.world - (nn - v * dot(nn, v)) * 0.03, 1.0);
        let uv = clamp(vec2<f32>(ndc.x / ndc.w * 0.5 + 0.5, 0.5 - ndc.y / ndc.w * 0.5), vec2<f32>(0.0), vec2<f32>(1.0));
        let behind = textureSampleLevel(scene_behind, behind_samp, uv, 0.0).rgb;
        let base = shade_n(i, t, nn);
        let fres = 0.04 + 0.96 * pow(1.0 - clamp(dot(nn, v), 0.0, 1.0), 5.0);
        let r = reflect(-v, nn);
        let env = mix(g.fog_color.rgb, vec3<f32>(0.9, 0.95, 1.0), clamp(r.z * 0.5 + 0.5, 0.0, 1.0));
        let spec = pow(max(dot(r, g.sun_dir.xyz), 0.0), 60.0) * g.sun_color.rgb * 0.8;
        let tinted = mix(behind, base, t.a);
        out = select(vec4<f32>(mix(tinted, env, fres * 0.35) + spec, 1.0 - fog), vec4<f32>(0.0), back);
    }
    return out;
}

@fragment
fn fs_shape_add(i: ShapeOut) -> @location(0) vec4<f32> {
    let t = textureSample(shape_tex, shape_samp, i.uv, i.layer);
    // Additive surfaces (muzzle flashes, glows) are light sources: HDR boost.
    return vec4<f32>(shade(i, t) * select(g.extras.z, 1.0, (i.flags & 1u) != 0u), t.a * (1.0 - fog_factor(i.world)));
}

// Fills the current viewport (scissored) with the fog colour at the far plane:
// clearing a sub-rectangle of a target, which a load op can't do.
@vertex
fn vs_clear(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 1.0, 1.0);
}

@fragment
fn fs_clear() -> @location(0) vec4<f32> {
    // Scenes keep alpha 1 in the fog colour; object views may clear to a
    // transparent background.
    return g.fog_color;
}

@group(1) @binding(0) var sprite_tex: texture_2d<f32>;
@group(1) @binding(1) var sprite_samp: sampler;
// The opaque scene's depth (soft particles).
@group(2) @binding(0) var sprite_depth: texture_depth_2d;

struct SpriteOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    // x: distance along the view axis, y: soft fade distance (0 = hard).
    @location(3) soft: vec2<f32>,
};

@vertex
fn vs_sprite(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>, @location(3) soft: f32) -> SpriteOut {
    var o: SpriteOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    o.world = pos;
    o.uv = uv;
    o.color = color;
    o.soft = vec2<f32>(dot(pos - g.camera.xyz, g.cam_forward.xyz), soft);
    return o;
}

// Soft particles: a camera-facing sprite fades out where it nears the scene
// behind it (no hard line where a big explosion cuts the ground), and towards
// its own border (some explosion images are not fully transparent there).
// Returns the alpha `a` faded (transparent texels skip the depth read).
fn sprite_soft(i: SpriteOut, a: f32) -> f32 {
    if (i.soft.y <= 0.0 || a < 0.002) {
        return a;
    }
    let d = textureLoad(sprite_depth, vec2<i32>(i.clip.xy), 0);
    let near = g.time.w;
    let far = g.light_info.w;
    // perspective_rh: depth 0 at the near plane, 1 at the far plane.
    let scene = near * far / max(far - d * (far - near), 1e-6);
    let e = min(min(i.uv.x, 1.0 - i.uv.x), min(i.uv.y, 1.0 - i.uv.y));
    return a * saturate((scene - i.soft.x) / i.soft.y) * smoothstep(0.0, 0.1, e);
}

@fragment
fn fs_sprite(i: SpriteOut) -> @location(0) vec4<f32> {
    let t = textureSample(sprite_tex, sprite_samp, i.uv) * i.color;
    // Like blended shape surfaces: fade out with the fog.
    return vec4<f32>(t.rgb, sprite_soft(i, t.a) * (1.0 - fog_factor(i.world)));
}

@fragment
fn fs_sprite_add(i: SpriteOut) -> @location(0) vec4<f32> {
    let t = textureSample(sprite_tex, sprite_samp, i.uv) * i.color;
    // Explosions, projectiles: HDR boost.
    return vec4<f32>(t.rgb * g.extras.z, sprite_soft(i, t.a) * (1.0 - fog_factor(i.world)));
}

// Upscale of a view rendered at a reduced resolution.
@group(0) @binding(0) var blit_tex: texture_2d<f32>;
@group(0) @binding(1) var blit_samp: sampler;

struct BlitOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_blit(@builtin(vertex_index) i: u32) -> BlitOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var o: BlitOut;
    o.clip = vec4<f32>(p, 0.0, 1.0);
    o.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return o;
}

@fragment
fn fs_blit(i: BlitOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(blit_tex, blit_samp, i.uv, 0.0);
}
