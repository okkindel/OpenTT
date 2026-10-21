//! Textures, their mip chains and samplers on the GPU.

use std::path::PathBuf;
use std::sync::OnceLock;
use wgpu::util::DeviceExt;

/// Edge length of the normal-map array layers (that of every layer,
/// [`crate::textures::size`]: their mips come from the same cache).
pub(super) const NORMAL_SIZE: u32 = 512;

/// Normal maps for texture files (`X.png` -> `X.normal.png`, flat where
/// missing), `NORMAL_SIZE`² RGBA8 layers (at least two), shared from the
/// cache ([`crate::textures::normal_layer`]).
pub(super) fn normal_layers(files: &[Option<PathBuf>]) -> Vec<std::sync::Arc<Vec<u8>>> {
    let flat = crate::textures::flat_normal();
    let mut maps: Vec<_> = crate::textures::parallel_map(files, |f| f.as_ref().and_then(|f| crate::textures::normal_layer(f, NORMAL_SIZE)).unwrap_or_else(|| flat.clone()));
    while maps.len() < 2 {
        maps.push(flat.clone());
    }
    maps
}

/// A 1x1 two-layer flat normal map (normal maps off).
pub(super) fn flat_normals(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    texture(device, queue, 1, 2, wgpu::TextureFormat::Rgba8Unorm, &[128, 128, 255, 255, 128, 128, 255, 255])
}

/// Box-filtered mip chain of a square RGBA8 image (`size` must be a power of two).
/// sRGB data is averaged in linear space so distant textures don't darken.
pub(crate) fn mip_chain(layer: &[u8], size: u32, srgb: bool) -> Vec<u8> {
    let (to_lin, to_byte) = mip_tables(srgb);
    let mut out = Vec::with_capacity(layer.len() * 4 / 3 + 4);
    out.extend_from_slice(layer);
    let mut cur: Vec<f32> = layer.chunks(4).flat_map(|p| [to_lin[p[0] as usize], to_lin[p[1] as usize], to_lin[p[2] as usize], p[3] as f32 / 255.0]).collect();
    let mut s = size as usize;
    while s > 1 {
        let h = s / 2;
        let mut next = vec![0.0f32; h * h * 4];
        for y in 0..h {
            for x in 0..h {
                for c in 0..4 {
                    let at = |xx: usize, yy: usize| cur[(yy * s + xx) * 4 + c];
                    next[(y * h + x) * 4 + c] = (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1)) / 4.0;
                }
            }
        }
        out.extend(next.chunks(4).flat_map(|p| [to_byte.byte(p[0]), to_byte.byte(p[1]), to_byte.byte(p[2]), (p[3] * 255.0).round() as u8]));
        cur = next;
        s = h;
    }
    out
}

/// Colour channel conversions of [`mip_chain`]: byte to (linear) value by
/// table, and value to byte by the exact thresholds of
/// `round(v^(1/2.2) * 255)` (or `round(v * 255)`), clamped.
fn mip_tables(srgb: bool) -> (&'static [f32; 256], &'static ByteThresholds) {
    static T: OnceLock<[([f32; 256], ByteThresholds); 2]> = OnceLock::new();
    let t = T.get_or_init(|| {
        [false, true].map(|srgb| {
            let lin = std::array::from_fn(|c| if srgb { (c as f32 / 255.0).powf(2.2) } else { c as f32 / 255.0 });
            (lin, ByteThresholds::new(move |v: f32| ((if srgb { v.powf(1.0 / 2.2) } else { v }) * 255.0).round().clamp(0.0, 255.0) as u8))
        })
    });
    let (lin, th) = &t[srgb as usize];
    (lin, th)
}

/// For a monotonic `f: f32 -> u8` on [0, 1]: `first[k]` is the smallest
/// value with `f(v) >= k`, so `f(v)` is the count of thresholds <= `v`.
struct ByteThresholds {
    first: [f32; 256],
}

impl ByteThresholds {
    fn new(f: impl Fn(f32) -> u8 + Copy) -> ByteThresholds {
        let mut first = [f32::INFINITY; 256];
        first[0] = f32::NEG_INFINITY;
        for (k, slot) in first.iter_mut().enumerate().skip(1) {
            // Bisection over the bit patterns of non-negative floats (their
            // order is the numeric order) in [0, 1].
            let (mut lo, mut hi) = (0u32, 1.0f32.to_bits() + 1);
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if f(f32::from_bits(mid)) as usize >= k {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            if lo <= 1.0f32.to_bits() {
                *slot = f32::from_bits(lo);
            }
        }
        ByteThresholds { first }
    }

    #[inline]
    fn byte(&self, v: f32) -> u8 {
        if !(0.0..=1.0).contains(&v) {
            return self.slow(v);
        }
        // Thresholds <= v (first[0] always counts).
        (self.first.partition_point(|&t| t <= v) - 1) as u8
    }

    #[cold]
    fn slow(&self, v: f32) -> u8 {
        if v.is_nan() || v <= 0.0 {
            0
        } else {
            255
        }
    }
}

/// A trilinear, repeating sampler (`anisotropy` 1 = none) that skips the
/// top `skip_mips` mip levels (at most 8).
pub(super) fn repeat_sampler(device: &wgpu::Device, anisotropy: u16, skip_mips: u32) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        anisotropy_clamp: anisotropy,
        lod_min_clamp: skip_mips.min(8) as f32,
        ..Default::default()
    })
}

/// [`texture`] of sRGB [`crate::textures::size`] layers, their mip levels
/// from the cache ([`crate::textures::srgb_mips`]).
pub(crate) fn srgb_layers_texture(device: &wgpu::Device, queue: &wgpu::Queue, layers: &[std::sync::Arc<Vec<u8>>]) -> wgpu::TextureView {
    layers_texture(device, queue, layers, wgpu::TextureFormat::Rgba8UnormSrgb)
}

/// [`texture`] of [`crate::textures::size`] layers in `format` (sRGB or
/// linear RGBA8), their mip levels from the cache ([`crate::textures::mips`]).
pub(crate) fn layers_texture(device: &wgpu::Device, queue: &wgpu::Queue, layers: &[std::sync::Arc<Vec<u8>>], format: wgpu::TextureFormat) -> wgpu::TextureView {
    let size = crate::textures::size();
    let mips = crate::textures::parallel_map(layers, |l| crate::textures::mips(l, format.is_srgb()));
    let mut data = Vec::with_capacity(layers.iter().zip(&mips).map(|(l, m)| l.len() + m.len()).sum());
    for (l, m) in layers.iter().zip(&mips) {
        data.extend_from_slice(l);
        data.extend_from_slice(m);
    }
    upload(device, queue, size, layers.len() as u32, size.ilog2() + 1, format, &data)
}

pub(crate) fn texture(device: &wgpu::Device, queue: &wgpu::Queue, size: u32, layers: u32, format: wgpu::TextureFormat, data: &[u8]) -> wgpu::TextureView {
    let mips = if size.is_power_of_two() { size.ilog2() + 1 } else { 1 };
    let layer_bytes = (size * size * 4) as usize;
    let data: Vec<u8> = if mips > 1 {
        let layers: Vec<&[u8]> = data.chunks(layer_bytes).collect();
        crate::textures::parallel_map(&layers, |l| mip_chain(l, size, format.is_srgb())).concat()
    } else {
        data.to_vec()
    };
    upload(device, queue, size, layers, mips, format, &data)
}

/// A texture (array) from layer-major data with all `mips` levels.
fn upload(device: &wgpu::Device, queue: &wgpu::Queue, size: u32, layers: u32, mips: u32, format: wgpu::TextureFormat, data: &[u8]) -> wgpu::TextureView {
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layers },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        data,
    );
    tex.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(if layers > 1 { wgpu::TextureViewDimension::D2Array } else { wgpu::TextureViewDimension::D2 }),
        ..Default::default()
    })
}

#[cfg(test)]
mod mip_tests {
    use super::*;

    /// The reference mip chain (per-channel `powf`), which the tables must match bit for bit.
    fn mip_chain_reference(layer: &[u8], size: u32, srgb: bool) -> Vec<u8> {
        let to_lin = |c: u8| if srgb { (c as f32 / 255.0).powf(2.2) } else { c as f32 / 255.0 };
        let to_byte = |v: f32| ((if srgb { v.powf(1.0 / 2.2) } else { v }) * 255.0).round().clamp(0.0, 255.0) as u8;
        let mut out = layer.to_vec();
        let mut cur: Vec<f32> = layer.chunks(4).flat_map(|p| [to_lin(p[0]), to_lin(p[1]), to_lin(p[2]), p[3] as f32 / 255.0]).collect();
        let mut s = size as usize;
        while s > 1 {
            let h = s / 2;
            let mut next = vec![0.0f32; h * h * 4];
            for y in 0..h {
                for x in 0..h {
                    for c in 0..4 {
                        let at = |xx: usize, yy: usize| cur[(yy * s + xx) * 4 + c];
                        next[(y * h + x) * 4 + c] = (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1)) / 4.0;
                    }
                }
            }
            out.extend(next.chunks(4).flat_map(|p| [to_byte(p[0]), to_byte(p[1]), to_byte(p[2]), (p[3] * 255.0).round() as u8]));
            cur = next;
            s = h;
        }
        out
    }

    #[test]
    fn byte_thresholds_match_powf_exactly() {
        for srgb in [false, true] {
            let (_, th) = mip_tables(srgb);
            let exact = |v: f32| ((if srgb { v.powf(1.0 / 2.2) } else { v }) * 255.0).round().clamp(0.0, 255.0) as u8;
            // Every float in [0, 1] at a stride, plus the neighbours of each threshold.
            let mut bits = 0u32;
            while bits <= 1.0f32.to_bits() {
                let v = f32::from_bits(bits);
                assert_eq!(th.byte(v), exact(v), "srgb {srgb} v {v:e}");
                bits += 997;
            }
            for &t in &th.first[1..] {
                if t.is_finite() {
                    for v in [f32::from_bits(t.to_bits() - 1), t, f32::from_bits(t.to_bits() + 1)] {
                        assert_eq!(th.byte(v), exact(v), "srgb {srgb} v {v:e}");
                    }
                }
            }
            for v in [-1.0, 0.0, 1.0, 1.5, f32::NAN] {
                assert_eq!(th.byte(v), exact(v), "srgb {srgb} v {v}");
            }
        }
    }

    #[test]
    fn mip_chain_matches_reference() {
        let size = 64u32;
        let mut seed = 12345u32;
        let layer: Vec<u8> = (0..size * size * 4)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect();
        for srgb in [false, true] {
            assert_eq!(mip_chain(&layer, size, srgb), mip_chain_reference(&layer, size, srgb));
        }
    }
}
