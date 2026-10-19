//! Texture lookup, decoding and the size of the texture array layers. The
//! asset directory holds the 4x upscaled textures, uploaded at 512 px.
//!
//! Decoded layers are cached for the life of the process (keyed by file),
//! so a second mission, and shapes sharing a skin, decode nothing again;
//! [`prefetch`] decodes a batch on all cores.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Edge length of every texture array layer.
pub fn size() -> u32 {
    512
}

/// Texture file for a path without extension (`.png`, `.jpg`, `.bmp`, `.gif`).
pub fn find(base: &Path) -> Option<PathBuf> {
    tt_assets::find_texture(base)
}

/// An image file to decode into a layer.
pub type Job = PathBuf;

/// A decoded layer ([`size`]² RGBA8), or why the file could not be read.
pub type Layer = Result<Arc<Vec<u8>>, String>;

type Slot = Arc<OnceLock<Layer>>;

fn cache() -> &'static Mutex<HashMap<Job, Slot>> {
    static CACHE: OnceLock<Mutex<HashMap<Job, Slot>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Image `name` next to a shape (any case, optional extension; DTS
/// materials name their textures without one).
pub fn image_file(dir: &Path, name: &str) -> Option<Job> {
    let stem = name.rsplit_once('.').filter(|(_, ext)| ["png", "jpg", "bmp", "gif"].contains(&ext.to_lowercase().as_str()));
    find(&dir.join(stem.map(|(s, _)| s).unwrap_or(name)))
}

/// `file` resampled to the layer size (cached; concurrent callers of one
/// file decode it once). Transparency is the image's own alpha channel.
pub fn layer(file: &Path) -> Layer {
    let slot = cache().lock().unwrap().entry(file.to_path_buf()).or_default().clone();
    slot.get_or_init(|| decode(file).map(Arc::new)).clone()
}

fn decode(file: &Path) -> Result<Vec<u8>, String> {
    let size = size();
    let img = image::open(file).map_err(|e| e.to_string())?;
    Ok(img.resize_exact(size, size, image::imageops::FilterType::Lanczos3).to_rgba8().into_raw())
}

type NSlot = Arc<OnceLock<Option<Arc<Vec<u8>>>>>;

fn normals_cache() -> &'static Mutex<HashMap<(PathBuf, u32), NSlot>> {
    static NORMALS: OnceLock<Mutex<HashMap<(PathBuf, u32), NSlot>>> = OnceLock::new();
    NORMALS.get_or_init(Default::default)
}

type MipSlot = (Arc<Vec<u8>>, Arc<OnceLock<Arc<Vec<u8>>>>);

fn mips_cache() -> &'static Mutex<HashMap<(usize, bool), MipSlot>> {
    static MIPS: OnceLock<Mutex<HashMap<(usize, bool), MipSlot>>> = OnceLock::new();
    MIPS.get_or_init(Default::default)
}

/// Bytes the caches hold: decoded layers, normal maps, mip levels.
pub fn cache_bytes() -> (usize, usize, usize) {
    let layers = cache().lock().unwrap().values().filter_map(|s| s.get()).filter_map(|l| l.as_ref().ok()).map(|v| v.len()).sum();
    let normals = normals_cache().lock().unwrap().values().filter_map(|s| s.get()).flatten().map(|v| v.len()).sum();
    let mips = mips_cache().lock().unwrap().values().filter_map(|(_, m)| m.get()).map(|v| v.len()).sum();
    (layers, normals, mips)
}

/// Frees what only a later load could use, once the GPU has the current
/// textures: every normal map and mip level (derived data, quick to make
/// again on all cores) and the decoded layers nothing else holds (the
/// scene's static shapes keep theirs).
pub fn trim() {
    normals_cache().lock().unwrap().clear();
    mips_cache().lock().unwrap().clear();
    cache().lock().unwrap().retain(|_, slot| slot.get().is_none_or(|l| l.as_ref().is_ok_and(|v| Arc::strong_count(v) > 1)));
}

/// Decodes `jobs` into the cache on all cores (already cached ones are free).
pub fn prefetch(jobs: &[Job]) {
    let mut seen = std::collections::HashSet::new();
    let todo: Vec<&Job> = jobs.iter().filter(|j| seen.insert(*j)).collect();
    parallel_map(&todo, |f| {
        let _ = layer(f);
    });
}

/// The normal map next to texture `file` (`X.normal.png`, normal-map
/// extra) at `edge`² RGBA8, cached like [`layer`]; None if there is none.
pub fn normal_layer(file: &Path, edge: u32) -> Option<Arc<Vec<u8>>> {
    let slot = normals_cache().lock().unwrap().entry((file.to_path_buf(), edge)).or_default().clone();
    slot.get_or_init(|| {
        let stem = file.file_stem()?.to_string_lossy().into_owned();
        let img = image::open(file.with_file_name(format!("{stem}.normal.png"))).ok()?;
        Some(Arc::new(img.resize_exact(edge, edge, image::imageops::FilterType::Triangle).to_rgba8().into_raw()))
    })
    .clone()
}

/// Plain white layer (layer 0 of every shape set).
pub fn white() -> Arc<Vec<u8>> {
    static WHITE: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
    WHITE.get_or_init(|| Arc::new(vec![255u8; (size() * size() * 4) as usize])).clone()
}

/// Flat normal-map layer of [`size`] (no `X.normal.png`).
pub fn flat_normal() -> Arc<Vec<u8>> {
    static FLAT: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
    FLAT.get_or_init(|| Arc::new([128u8, 128, 255, 255].repeat((size() * size()) as usize))).clone()
}

/// Mip levels below the base of an sRGB layer of [`size`] (the
/// renderer's box filter), cached per layer allocation: layers from
/// [`layer`] and [`white`] stay alive, so each is filtered once.
pub fn srgb_mips(px: &Arc<Vec<u8>>) -> Arc<Vec<u8>> {
    mips(px, true)
}

/// [`srgb_mips`] of an sRGB (`srgb`) or linear (normal map) layer.
pub fn mips(px: &Arc<Vec<u8>>, srgb: bool) -> Arc<Vec<u8>> {
    let slot = {
        let mut m = mips_cache().lock().unwrap();
        // The key is the allocation; holding the Arc keeps the key unique.
        m.entry((Arc::as_ptr(px) as usize, srgb)).or_insert_with(|| (px.clone(), Default::default())).1.clone()
    };
    slot.get_or_init(|| Arc::new(crate::render::mip_chain(px, size(), srgb).split_off(px.len()))).clone()
}

/// `f` over `items` on up to all cores, results in order.
pub fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(items.len());
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(item) = items.get(i) else { break };
                *out[i].lock().unwrap() = Some(f(item));
            });
        }
    });
    out.into_iter().map(|m| m.into_inner().unwrap().unwrap()).collect()
}
