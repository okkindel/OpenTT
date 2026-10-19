//! The mapping of the asset directory: for every file the importer makes,
//! where it comes from and how (`crates/tt-import/mapping.ron`), and the
//! list of OpenTT's own files (taken from the repository's `assets/`).
//!
//! `mapping.ron` is generated from the provenance tables of the asset
//! directory (`assets/*/SOURCES.md`) by [`from_sources`] (`tt-import
//! mapping --assets assets`); a test keeps the two in step and checks that
//! the mapping covers exactly the files of `assets/`.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// How one asset file is made. "asset" inputs are files the importer made
/// (or copied from OpenTT's own) before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum How {
    Copy(String),
    /// `upscaled`: the repository's copy is the 4x Real-ESRGAN upscale.
    Image { from: String, alpha: Option<String>, upscaled: bool },
    Ifl(String),
    Dml { from: String, keep: usize },
    Terrain(String),
    Mission(String),
    /// OpenTT's recolouring of an imported texture (`from`, an asset):
    /// HSV hue set to `hue` degrees, saturation raised to at least
    /// `min_saturation`, value scaled by `value`; `thicken` > 0: the
    /// alpha grown by a max filter of that size (at an image `at_width`
    /// wide) and the colour made one (the mean of the visible pixels).
    Recolour { from: String, hue: f32, min_saturation: f32, value: f32, thicken: u32, at_width: u32 },
    /// OpenTT's sound made with ffmpeg from imported sounds (`from`,
    /// assets, the filter graph's inputs `[0:a]`, `[1:a]`, ...; its output
    /// is `[out]`), then brought to `level` (measured by a first pass).
    Sound { from: Vec<String>, filter: String, level: Level },
    /// The "normal maps" option's map of a texture (an asset).
    NormalMap(String),
    /// A mission's menu picture: rendered with the game's renderer from
    /// `camera` (x y z yaw pitch hfov) with `--pictures`.
    Picture { mission: String, camera: [f32; 6], game_file: Option<String> },
}

impl How {
    pub fn game_files(&self) -> Vec<String> {
        use How::*;
        match self {
            Copy(f) | Ifl(f) | Terrain(f) | Mission(f) => vec![f.clone()],
            Dml { from, .. } => vec![from.clone()],
            Image { from, alpha, .. } => std::iter::once(from.clone()).chain(alpha.clone()).collect(),
            Picture { game_file, .. } => game_file.iter().cloned().collect(),
            Recolour { .. } | Sound { .. } | NormalMap(_) => Vec::new(),
        }
    }
}

/// The level a made sound is brought to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Level {
    /// Integrated loudness (EBU R128), LUFS.
    Lufs(f32),
    /// Sample peak, dBFS.
    Peak(f32),
}

/// One file the importer makes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Its asset path.
    pub asset: String,
    pub how: How,
}

/// `mapping.ron`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mapping {
    /// Every file made from the game install, in asset-path order.
    pub files: Vec<Entry>,
    /// OpenTT's own files (asset paths): copied from the repository's
    /// `assets/` as they are.
    pub ours: Vec<String>,
    pub replaced: Vec<(String, String)>,
}

/// The mapping built into the importer.
pub const MAPPING_RON: &str = include_str!("../mapping.ron");

const HEADER: &str = "The importer's mapping (crates/tt-import, docs/import.md): every asset file made\nfrom the game install (game path -> asset path, how) and our own\nfiles. Generated from assets/*/SOURCES.md by `tt-import mapping --assets assets`.";

impl Mapping {
    /// The mapping built into the importer.
    pub fn builtin() -> Mapping {
        tt_assets::data::from_ron(MAPPING_RON).expect("mapping.ron")
    }

    pub fn to_ron(&self) -> Result<String, String> {
        tt_assets::data::to_ron(HEADER, self)
    }

    pub fn get(&self, asset: &str) -> Option<&Entry> {
        self.files.iter().find(|e| e.asset == asset)
    }
}

/// The directories with a `SOURCES.md` provenance table.
pub const SOURCE_DIRS: &[&str] = &["data", "missions", "models", "sky", "sound", "terrain", "ui"];

/// OpenTT's own sounds made from the game's sounds (`sound/SOURCES.md`
/// describes them in words): (asset, inputs, ffmpeg filter graph, level).
const SOUNDS: &[(&str, &[&str], &str, Level)] = &[
    (
        "sound/sfx/explodemortar.ogg",
        &["sound/sfx/explodevolcano.ogg", "sound/sfx/explodearea.ogg"],
        "[0:a]asetrate=37485,aresample=44100[v];[1:a]asetrate=33075,aresample=44100,volume=0.8[a];[v][a]amix=inputs=2:duration=longest:normalize=0,bass=g=4:f=90,afade=t=out:st=2.0:d=0.8[out]",
        Level::Lufs(-11.4),
    ),
    (
        "sound/sfx/explodescatter.ogg",
        &["sound/sfx/explode.ogg"],
        "[0:a]atrim=0:0.3,asetpts=PTS-STARTPTS,asetrate=70560,aresample=44100,highpass=f=450,afade=t=out:st=0.08:d=0.14[out]",
        Level::Peak(-6.0),
    ),
    (
        "sound/sfx/mortarfire.ogg",
        &["sound/sfx/fireproj.ogg", "sound/sfx/explode.ogg"],
        "[0:a]asetrate=27342,aresample=44100,bass=g=6:f=110[f];[1:a]atrim=0:0.45,asetpts=PTS-STARTPTS,asetrate=22050,aresample=44100,lowpass=f=380,volume=0.7[e];[f][e]amix=inputs=2:duration=longest:normalize=0[out]",
        Level::Lufs(-13.0),
    ),
    (
        "sound/sfx/scatterfire.ogg",
        &["sound/sfx/fireproj.ogg"],
        "[0:a]atrim=0:0.5,asetpts=PTS-STARTPTS,asplit=4[a][b][c][d];[a]asetrate=52038,aresample=44100[a1];[b]asetrate=58212,aresample=44100,adelay=17,volume=0.8[b1];[c]asetrate=55125,aresample=44100,adelay=36,volume=0.75[c1];[d]asetrate=62622,aresample=44100,adelay=58,volume=0.6[d1];[a1][b1][c1][d1]amix=inputs=4:duration=longest:normalize=0,highpass=f=280[out]",
        Level::Lufs(-13.7),
    ),
];

/// One row of a `SOURCES.md` table: (asset path, source text).
fn rows(text: &str, dir: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("| `")?;
            let (file, rest) = rest.split_once("` | ")?;
            let src = rest.strip_suffix(" |").unwrap_or(rest);
            Some((format!("{dir}/{file}"), src.to_string()))
        })
        .collect()
}

/// The number after `key` in `s` (`"hue set to 22 deg"`, key `"hue set to "`).
fn number_after(s: &str, key: &str) -> Option<f32> {
    let rest = &s[s.find(key)? + key.len()..];
    let n: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    n.trim_end_matches('.').parse().ok()
}

/// The entry of one provenance row; `Ok(None)` for OpenTT's own files.
fn entry(asset: &str, src: &str) -> Result<Option<How>, String> {
    let err = || format!("{asset}: unrecognised provenance: {src}");
    let (sources, how) = src.split_once(": ").unwrap_or(("", src));
    let game_files: Vec<&str> = sources.split(" + ").map(|s| s.split(" (").next().unwrap_or(s).trim()).filter(|s| s.starts_with("game/") || s.starts_with("common/")).collect();
    let ext = asset.rsplit('.').next().unwrap_or("");
    let own = src.starts_with("generated: Piper")
        || src.starts_with("own text")
        || src.starts_with("(none)")
        || src.starts_with("the port's own")
        || src.starts_with("authored data");
    if own {
        return Ok(None);
    }
    if how.contains("examples/normal_maps.rs") {
        return Ok(Some(How::NormalMap(sources.to_string())));
    }
    if src.contains("--example mission_picture") {
        let id = asset.rsplit('/').next().and_then(|n| n.strip_suffix(".png")).ok_or_else(err)?;
        let cam = src.split("--camera ").nth(1).ok_or_else(err)?;
        let v: Vec<f32> = cam.split_whitespace().take(6).map(|x| x.parse::<f32>()).collect::<Result<_, _>>().map_err(|_| err())?;
        let camera: [f32; 6] = v.try_into().map_err(|_| err())?;
        let game_file = src.strip_prefix("game/").map(|r| format!("game/{}", r.split_whitespace().next().unwrap_or_default()));
        return Ok(Some(How::Picture { mission: id.to_string(), camera, game_file }));
    }
    if let Some(&(_, from, filter, level)) = SOUNDS.iter().find(|s| s.0 == asset) {
        return Ok(Some(How::Sound { from: from.iter().map(|s| s.to_string()).collect(), filter: filter.to_string(), level }));
    }
    if how.contains("Python PIL, HSV") {
        let from = sources.split(" + ").next().ok_or_else(err)?.to_string();
        let hue = number_after(how, "hue set to ").ok_or_else(err)?;
        let min_saturation = number_after(how, "saturation at least ").ok_or_else(err)?;
        let value = number_after(how, "value x").unwrap_or(1.0);
        let thicken = number_after(how, "alpha ").map_or(0, |v| v as u32);
        let at_width = if thicken > 0 { 128 } else { 0 };
        return Ok(Some(How::Recolour { from, hue, min_saturation, value, thicken, at_width }));
    }
    if how.contains("tt-convert missions") && game_files.len() == 1 {
        return Ok(Some(How::Mission(game_files[0].to_string())));
    }
    if how.contains("Real-ESRGAN") || how.contains("copied unchanged") {
        let upscaled = how.contains("Real-ESRGAN");
        return Ok(Some(match game_files.as_slice() {
            [c] => How::Image { from: c.to_string(), alpha: None, upscaled },
            [c, a] => How::Image { from: c.to_string(), alpha: Some(a.to_string()), upscaled },
            _ => return Err(err()),
        }));
    }
    if how.starts_with("original") && game_files.len() == 1 {
        let o = game_files[0].to_string();
        return Ok(Some(match ext {
            "ifl" => How::Ifl(o),
            "dml" => How::Dml { from: o, keep: 5 },
            "ter" => How::Terrain(o),
            _ => How::Copy(o),
        }));
    }
    Err(err())
}

/// The mapping the provenance tables of the asset directory `assets` describe.
pub fn from_sources(assets: &Path) -> Result<Mapping, String> {
    let mut files = Vec::new();
    let mut ours = Vec::new();
    let mut errors = Vec::new();
    for dir in SOURCE_DIRS {
        let p = assets.join(dir).join("SOURCES.md");
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        for (asset, src) in rows(&text, dir) {
            // Asset file names are lowercase (docs/layout.md § Naming),
            // whatever case the game's file has.
            if asset != asset.to_ascii_lowercase() {
                errors.push(format!("{asset}: asset paths are lowercase"));
                continue;
            }
            match entry(&asset, &src) {
                Ok(Some(how)) => files.push(Entry { asset, how }),
                Ok(None) => ours.push(asset),
                Err(e) => errors.push(e),
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let mut replaced = Vec::new();
    let archive = assets.join("../archive/SOURCES.md");
    if let Ok(text) = std::fs::read_to_string(&archive) {
        for (file, src) in rows(&text, "archive") {
            let Some(name) = file.strip_prefix("archive/original_voice/") else { continue };
            let asset = format!("sound/voice/{}", name.to_ascii_lowercase());
            let game_file = src.split(':').next().unwrap_or_default().trim().to_string();
            if ours.contains(&asset) && game_file.starts_with("game/") {
                replaced.push((game_file, asset));
            }
        }
    }
    files.sort_by(|a, b| a.asset.cmp(&b.asset));
    ours.sort();
    replaced.sort();
    Ok(Mapping { files, ours, replaced })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_rows() {
        assert_eq!(entry("models/tanks/tank01.dts", "game/data/shapes/tanks/tank01.dts: original"), Ok(Some(How::Copy("game/data/shapes/tanks/tank01.dts".into()))));
        assert_eq!(
            entry("ui/icons/lives.png", "game/client/ui/bitmaps/Lives.jpg + game/client/ui/bitmaps/Lives.alpha.jpg: Real-ESRGAN x4plus, 42x42 -> 168x168; alpha channel from the mask (Real-ESRGAN x4plus, 42x42 -> 168x168), merged into one RGBA PNG"),
            Ok(Some(How::Image { from: "game/client/ui/bitmaps/Lives.jpg".into(), alpha: Some("game/client/ui/bitmaps/Lives.alpha.jpg".into()), upscaled: true }))
        );
        assert_eq!(entry("sound/voice/vocarea.ogg", "generated: Piper TTS voice"), Ok(None));
        assert_eq!(entry("data/tanks.ron", "authored data of this project: the tanks"), Ok(None));
        assert_eq!(
            entry("reticles/scatterreticle.png", "models/reticles/reticle.png: generated (x): Python PIL, HSV hue set to 52 deg, saturation at least 0.9, alpha kept"),
            Ok(Some(How::Recolour { from: "models/reticles/reticle.png".into(), hue: 52.0, min_saturation: 0.9, value: 1.0, thicken: 0, at_width: 0 }))
        );
        assert!(entry("x/y.png", "something else").is_err());
    }
}
