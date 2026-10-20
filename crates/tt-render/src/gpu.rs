//! GPU setup shared by the game, the viewer and the benchmarks: the
//! instance, adapter and device, offscreen targets and their readback.

use crate::{Camera, Scene};
use glam::Vec3;

/// The wgpu instance of every window and offscreen target: all the
/// backends of this build (Windows: DX12, Vulkan and GL; Linux: Vulkan and
/// GL; macOS: Metal), or only those named by `WGPU_BACKEND` (`dx12`,
/// `vulkan`, `gl`, `metal`, comma-separated) for a player whose driver
/// misbehaves on the default choice (README § Troubleshooting).
pub fn instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default())
}

/// The adapter, device and queue to draw with (on `compatible` when given).
/// The adapter is wgpu's choice, else any adapter of any enabled backend
/// that can draw there (the best kind first, [`adapter_rank`]), else the
/// software one (WARP on Windows); the error lists what was found.
pub async fn gpu(compatible: Option<&wgpu::Surface<'_>>, instance: &wgpu::Instance) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), String> {
    let adapter = adapter(compatible, instance).await?;
    let info = adapter.get_info();
    // Optional features: GPU pass timing (`TT_GPU_TIMING`) and the packed
    // HDR scene format of the graphics extras.
    let features = adapter.features() & (crate::gputime::wanted_features() | crate::post::OPTIONAL_FEATURES);
    let desc = wgpu::DeviceDescriptor { required_features: features, required_limits: device_limits(&adapter.limits()), ..Default::default() };
    let (device, queue) = adapter.request_device(&desc, None).await.map_err(|e| format!("{} ({:?}): {e}", info.name, info.backend))?;
    Ok((adapter, device, queue))
}

/// See [`gpu`].
async fn adapter(compatible: Option<&wgpu::Surface<'_>>, instance: &wgpu::Instance) -> Result<wgpu::Adapter, String> {
    if let Some(a) = instance.request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: compatible, ..Default::default() }).await {
        return Ok(a);
    }
    let all = instance.enumerate_adapters(wgpu::Backends::all());
    let found: Vec<String> = all.iter().map(|a| a.get_info()).map(|i| format!("{} ({:?}, {:?})", i.name, i.backend, i.device_type)).collect();
    let usable = all.into_iter().filter(|a| compatible.is_none_or(|s| a.is_surface_supported(s))).min_by_key(|a| adapter_rank(a.get_info().device_type));
    if let Some(a) = usable {
        return Ok(a);
    }
    if let Some(a) = instance.request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: compatible, force_fallback_adapter: true, ..Default::default() }).await {
        return Ok(a);
    }
    let found = if found.is_empty() { "none".to_string() } else { found.join(", ") };
    Err(format!("no graphics adapter can draw the game (adapters found: {found}); update the graphics driver, or choose another graphics backend with the environment variable WGPU_BACKEND=dx12, vulkan or gl"))
}

/// The order adapters are tried in when wgpu's own choice found none: a
/// real GPU first, the software (CPU) ones last.
pub fn adapter_rank(t: wgpu::DeviceType) -> u8 {
    match t {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

/// The limits the device is asked for: wgpu's defaults (WebGPU's) when the
/// adapter has them, else everything it has (older GPUs and GL drivers fall
/// short of a default, which would refuse the device outright).
pub fn device_limits(supported: &wgpu::Limits) -> wgpu::Limits {
    let default = wgpu::Limits::default();
    if default.check_limits(supported) {
        default
    } else {
        supported.clone()
    }
}

/// Camera above the first spawn point looking at the map centre.
pub fn initial_camera(scene: &Scene) -> Camera {
    let spawn = scene.spawn.unwrap_or(Vec3::new(0.0, -150.0, 100.0));
    let pos = spawn + Vec3::new(0.0, 0.0, 40.0);
    let to_centre = Vec3::new(-pos.x, -pos.y, 0.0);
    Camera { pos, yaw: to_centre.x.atan2(to_centre.y), pitch: -0.3 }
}

/// A 2D texture to draw into and read back (screenshots, offscreen views).
pub fn render_target(device: &wgpu::Device, label: &str, (width, height): (u32, u32), format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Reads a whole texture (`COPY_SRC`, 8-bit RGBA or BGRA) back as tightly
/// packed RGBA8 rows, top row first.
pub fn read_rgba(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, size: (u32, u32), format: wgpu::TextureFormat) -> Vec<u8> {
    let (w, h) = size;
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);
    let data = buf.slice(..).get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    if matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
        for p in pixels.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_gpus_before_software_adapters() {
        use wgpu::DeviceType::*;
        let mut kinds = [Cpu, Other, VirtualGpu, IntegratedGpu, DiscreteGpu];
        kinds.sort_by_key(|&t| adapter_rank(t));
        assert_eq!(kinds, [DiscreteGpu, IntegratedGpu, VirtualGpu, Other, Cpu]);
    }

    #[test]
    fn device_limits_fall_back_to_the_adapters() {
        // An adapter with the defaults (or more) is asked for the defaults.
        let big = wgpu::Limits { max_texture_dimension_2d: 16384, ..wgpu::Limits::default() };
        assert_eq!(device_limits(&big), wgpu::Limits::default());
        // One short of a default (an old GL driver) for what it has.
        let gl = wgpu::Limits::downlevel_webgl2_defaults();
        assert!(!wgpu::Limits::default().check_limits(&gl));
        assert_eq!(device_limits(&gl), gl);
    }
}
