// Post-processing of the graphics extras: SSAO, bloom, tonemapping
// (crate::post). The scene is rendered into a linear HDR texture first.

struct Post {
    // x: bloom strength, y: tonemap on, z: SSAO strength (0 = off), w: SSAO samples
    params: vec4<f32>,
    // x: near, y: far, z: tan(half horizontal fov), w: tan(half vertical fov)
    proj: vec4<f32>,
    // x, y: texel size of `src` (composite: of the AO texture); z: SSAO
    // radius (m); w: SSAO fade distance (m)
    texel: vec4<f32>,
    // x, y: texel size of the bloom texture (composite)
    extra: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: Post;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var depth: texture_depth_2d;
@group(0) @binding(4) var ao_tex: texture_2d<f32>;
@group(0) @binding(5) var bloom_tex: texture_2d<f32>;

struct FullOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> FullOut {
    let q = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var o: FullOut;
    o.clip = vec4<f32>(q, 0.0, 1.0);
    o.uv = vec2<f32>(q.x * 0.5 + 0.5, 0.5 - q.y * 0.5);
    return o;
}

// ---- SSAO (Alchemy-style, from the depth buffer only)

// Distance along the view axis of a depth-buffer value (glam perspective_rh).
fn linear_depth(d: f32) -> f32 {
    let n = p.proj.x;
    let f = p.proj.y;
    return n * f / (f + d * (n - f));
}

fn view_pos(uv: vec2<f32>, l: f32) -> vec3<f32> {
    return vec3<f32>((uv.x * 2.0 - 1.0) * p.proj.z * l, (1.0 - 2.0 * uv.y) * p.proj.w * l, -l);
}

// Half-resolution linear depth: the nearest of each 2x2 block (sky = far).
@fragment
fn fs_ssao_depth(i: FullOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(depth));
    let c = vec2<i32>(i.clip.xy) * 2;
    let hi = dims - vec2<i32>(1);
    let d = min(min(textureLoad(depth, min(c, hi), 0), textureLoad(depth, min(c + vec2<i32>(1, 0), hi), 0)),
        min(textureLoad(depth, min(c + vec2<i32>(0, 1), hi), 0), textureLoad(depth, min(c + vec2<i32>(1, 1), hi), 0)));
    // Negative: sky (no occlusion, and never an occluder).
    return vec4<f32>(select(linear_depth(d), -1.0, d >= 1.0), 0.0, 0.0, 1.0);
}


// At half resolution, from `fs_ssao_depth`'s texture.
@fragment
fn fs_ssao(i: FullOut) -> @location(0) vec4<f32> {
    let l0 = textureLoad(src, vec2<i32>(i.clip.xy), 0).r;
    // (Sky pixels end up unshaded; no early return before the derivatives.)
    let l = select(l0, p.proj.y, l0 < 0.0);
    let pos = view_pos(i.uv, l);
    // Face normal from the reconstructed positions.
    var n = normalize(cross(dpdx(pos), dpdy(pos)));
    if (dot(n, pos) > 0.0) {
        n = -n;
    }
    let samples = i32(p.params.w);
    let radius = p.texel.z;
    // Sample disc radius in uv (vertically), and the aspect for x. Capped
    // on screen: close to the camera the disc would cover half the view
    // (scattered taps thrash the texture cache).
    let r_uv = min(radius / (2.0 * p.proj.w * l), 0.1);
    let aspect = p.proj.w / p.proj.z;
    // Per-pixel rotation (interleaved gradient noise; the composite's
    // 4-tap blur averages it out).
    let px = i.clip.xy;
    let noise = fract(52.9829189 * fract(0.06711056 * px.x + 0.00583715 * px.y));
    var occ = 0.0;
    // Golden-angle spiral, rotated per pixel; the direction steps by a
    // fixed rotation (no trigonometry per sample).
    let a0 = noise * 6.2831853;
    var dir = vec2<f32>(cos(a0), sin(a0));
    let step = vec2<f32>(cos(2.39996), sin(2.39996));
    let dt = 1.0 / f32(samples);
    let dims = vec2<f32>(textureDimensions(src));
    let hi = vec2<i32>(dims) - vec2<i32>(1);
    // Sky: no taps.
    let n_taps = select(samples, 0, l0 < 0.0);
    for (var k = 0; k < n_taps; k++) {
        let t = (f32(k) + 0.5) * dt;
        let off = vec2<f32>(dir.x * aspect, dir.y) * (r_uv * t);
        dir = vec2<f32>(dir.x * step.x - dir.y * step.y, dir.x * step.y + dir.y * step.x);
        let suv = i.uv + off;
        let sl = textureLoad(src, clamp(vec2<i32>(suv * dims), vec2<i32>(0), hi), 0).r;
        let v = view_pos(suv, sl) - pos;
        let vv = dot(v, v);
        // Only nearby occluders count (range check against the radius).
        let range = select(0.0, 1.0, vv < radius * radius * 4.0 && sl > 0.0);
        // Small bias (grows slowly with distance against depth precision).
        occ += range * max(dot(v, n) - (0.05 + 0.004 * l), 0.0) / (vv + 0.05);
    }
    var ao = max(1.0 - p.params.z * 2.0 * occ / f32(samples), 0.0);
    // Far, fogged ground stays unshaded.
    let fade = 1.0 - smoothstep(p.texel.w * 0.35, p.texel.w * 0.6, l);
    ao = mix(1.0, ao, fade * select(1.0, 0.0, l0 < 0.0));
    return vec4<f32>(ao, ao, ao, 1.0);
}

// Blur of the AO at its resolution (8 bilinear taps).
@fragment
fn fs_ssao_blur(i: FullOut) -> @location(0) vec4<f32> {
    // Inner 4x4 box (bilinear at ±1) plus an outer ring at ±2.5: the
    // interleaved noise of the 8-sample spiral averages out.
    let t = p.texel.xy;
    let u = t * 2.5;
    let ao = (textureSampleLevel(src, samp, i.uv + vec2<f32>(-t.x, -t.y), 0.0).r + textureSampleLevel(src, samp, i.uv + vec2<f32>(t.x, -t.y), 0.0).r
        + textureSampleLevel(src, samp, i.uv + vec2<f32>(-t.x, t.y), 0.0).r + textureSampleLevel(src, samp, i.uv + vec2<f32>(t.x, t.y), 0.0).r) * 0.15
        + (textureSampleLevel(src, samp, i.uv + vec2<f32>(u.x, 0.0), 0.0).r + textureSampleLevel(src, samp, i.uv - vec2<f32>(u.x, 0.0), 0.0).r
        + textureSampleLevel(src, samp, i.uv + vec2<f32>(0.0, u.y), 0.0).r + textureSampleLevel(src, samp, i.uv - vec2<f32>(0.0, u.y), 0.0).r) * 0.1;
    return vec4<f32>(ao, ao, ao, 1.0);
}

// ---- Bloom

// Bright pass into 1/4 resolution (4 bilinear taps = a 4x4 box).
@fragment
fn fs_bloom_pre(i: FullOut) -> @location(0) vec4<f32> {
    let t = p.texel.xy;
    var c = textureSample(src, samp, i.uv + vec2<f32>(-t.x, -t.y)).rgb;
    c += textureSample(src, samp, i.uv + vec2<f32>(t.x, -t.y)).rgb;
    c += textureSample(src, samp, i.uv + vec2<f32>(-t.x, t.y)).rgb;
    c += textureSample(src, samp, i.uv + vec2<f32>(t.x, t.y)).rgb;
    c *= 0.25;
    // Soft threshold at 1 (only HDR highlights bloom).
    let lum = max(c.r, max(c.g, c.b));
    let knee = 0.1;
    let soft = clamp(lum - 1.0 + knee, 0.0, 2.0 * knee);
    let w = max(soft * soft / (4.0 * knee), lum - 1.0) / max(lum, 1e-4);
    return vec4<f32>(c * w, 1.0);
}

@fragment
fn fs_bloom_down(i: FullOut) -> @location(0) vec4<f32> {
    let t = p.texel.xy;
    var c = textureSample(src, samp, i.uv + vec2<f32>(-t.x, -t.y)).rgb;
    c += textureSample(src, samp, i.uv + vec2<f32>(t.x, -t.y)).rgb;
    c += textureSample(src, samp, i.uv + vec2<f32>(-t.x, t.y)).rgb;
    c += textureSample(src, samp, i.uv + vec2<f32>(t.x, t.y)).rgb;
    return vec4<f32>(c * 0.25, 1.0);
}

// Tent upsample of the next smaller level, added to this one.
@fragment
fn fs_bloom_up(i: FullOut) -> @location(0) vec4<f32> {
    let t = p.texel.xy;
    var c = textureSample(src, samp, i.uv).rgb * 4.0;
    c += (textureSample(src, samp, i.uv + vec2<f32>(t.x, 0.0)).rgb + textureSample(src, samp, i.uv - vec2<f32>(t.x, 0.0)).rgb
        + textureSample(src, samp, i.uv + vec2<f32>(0.0, t.y)).rgb + textureSample(src, samp, i.uv - vec2<f32>(0.0, t.y)).rgb) * 2.0;
    c += textureSample(src, samp, i.uv + t).rgb + textureSample(src, samp, i.uv - t).rgb
        + textureSample(src, samp, i.uv + vec2<f32>(t.x, -t.y)).rgb + textureSample(src, samp, i.uv + vec2<f32>(-t.x, t.y)).rgb;
    return vec4<f32>(c / 16.0, 1.0);
}

// ---- Composite into the target

// Identity below the knee, highlights roll off towards 1 (keeps the
// cartoon colours, no filmic curve).
fn soft_clip(x: vec3<f32>) -> vec3<f32> {
    let k = 0.8;
    let over = max(x - vec3<f32>(k), vec3<f32>(0.0));
    return min(x, vec3<f32>(k)) + (1.0 - k) * (vec3<f32>(1.0) - exp(-over / (1.0 - k)));
}

@fragment
fn fs_composite(i: FullOut) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(src, samp, i.uv, 0.0);
    if (p.params.z > 0.0) {
        // (Blurred at half resolution by `fs_ssao_blur`.)
        let ao = textureSampleLevel(ao_tex, samp, i.uv, 0.0).r;
        c = vec4<f32>(c.rgb * ao, c.a);
    }
    if (p.params.x > 0.0) {
        // One bilinear tap of the 1/4-resolution bloom (already smooth: the
        // up chain's tent filters).
        c = vec4<f32>(c.rgb + textureSampleLevel(bloom_tex, samp, i.uv, 0.0).rgb * p.params.x, c.a);
    }
    if (p.params.y > 0.0) {
        c = vec4<f32>(soft_clip(c.rgb), c.a);
    }
    return c;
}
