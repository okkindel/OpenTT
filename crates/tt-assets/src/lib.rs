//! Loaders for Think Tanks data files.

mod bytes;
pub mod data;
pub mod dts;
pub mod layout;
pub mod mission;
pub mod mods;
pub mod normal_map;
pub mod product;
pub mod terrain;
pub mod testdata;

use std::path::{Path, PathBuf};

/// The FNV-1a 64 offset basis: the start of a [`fnv1a`] hash.
pub const FNV_START: u64 = 0xcbf2_9ce4_8422_2325;

/// Feeds `bytes` into the 64-bit FNV-1a hash `h` (the content hashes of
/// mods, defs, missions and saves).
pub fn fnv1a(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x0100_0000_01b3);
    }
}

/// The file an asset reference names under the asset directory `root`
/// ([`layout`]): references are paths relative to it
/// (`models/tanks/tank01.dts`); `./x` is relative to the referencing
/// file's directory `referencing_dir`. The enabled mods apply
/// ([`mods::redirect`]): the file may be a mod's.
pub fn resolve(root: &Path, referencing_dir: &Path, path: &str) -> PathBuf {
    let path = path.replace('\\', "/");
    let p = match path.strip_prefix("./") {
        Some(rest) => referencing_dir.join(rest),
        None => root.join(path.trim_start_matches('/')),
    };
    mods::redirect(&p).unwrap_or(p)
}

/// Image file extensions a texture may have.
pub const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "bmp", "gif"];

/// Finds a texture: `base` itself when it names an image file, else `base`
/// plus an image extension.
pub fn find_texture(base: &Path) -> Option<PathBuf> {
    let ext = base.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if ext.is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str())) {
        if let Some(f) = find_ci(base) {
            return Some(f);
        }
    }
    IMAGE_EXTENSIONS.iter().find_map(|ext| {
        let mut p = base.as_os_str().to_owned();
        p.push(".");
        p.push(ext);
        find_ci(Path::new(&p))
    })
}

/// Bytes as Latin-1 text (the game's files and saves are 8-bit).
pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// The game was made on Windows: resolve `path` ignoring the case of each
/// component (exact matches are preferred). The enabled mods apply first
/// ([`mods::redirect`]).
pub fn find_ci(path: &Path) -> Option<PathBuf> {
    let redirected = mods::redirect(path);
    let path = redirected.as_deref().unwrap_or(path);
    if path.exists() {
        return Some(path.to_path_buf());
    }
    let mut out = PathBuf::new();
    for comp in path.components() {
        let next = out.join(comp);
        if next.exists() || out.as_os_str().is_empty() {
            out = next;
            continue;
        }
        let want = comp.as_os_str().to_str()?.to_lowercase();
        let found = std::fs::read_dir(&out)
            .ok()?
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_str().is_some_and(|n| n.to_lowercase() == want))?;
        out.push(found.file_name());
    }
    out.exists().then_some(out)
}
