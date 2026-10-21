//! The import: the asset directory built from a game install
//! ([`Options::game`]), the mapping ([`crate::mapping`]) and OpenTT's
//! own files ([`Options::ours`], among them the game's data tables
//! `data/`).
//!
//! Order: OpenTT's own files are copied; then the files made from
//! imported ones (OpenTT's recoloured textures, its sounds, the normal
//! maps) and the mission pictures.

use crate::convert;
use crate::images;
use crate::mapping::{Entry, How, Level, Mapping};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A stage of the import, for progress reports ([`Progress`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Checking the install, preparing the output.
    Starting,
    /// The bundled files (the game's data tables, maps, the Mortar rocket,
    /// announcer lines).
    Bundled,
    Models,
    Textures,
    Sounds,
    Maps,
    /// The new weapons' textures and sounds, made from imported ones.
    Weapons,
    NormalMaps,
    Pictures,
    /// Moving the finished import into place.
    Finishing,
}

impl Phase {
    /// What the phase does, for the player.
    pub fn label(self) -> &'static str {
        match self {
            Phase::Starting => "Preparing",
            Phase::Bundled => "Copying bundled files",
            Phase::Models => "Copying models",
            Phase::Textures => "Converting textures",
            Phase::Sounds => "Copying sounds",
            Phase::Maps => "Converting maps",
            Phase::Weapons => "Making weapon effects",
            Phase::NormalMaps => "Making normal maps",
            Phase::Pictures => "Rendering mission pictures",
            Phase::Finishing => "Finishing",
        }
    }

    /// The phase an entry of the mapping belongs to.
    pub fn of(e: &Entry) -> Phase {
        match &e.how {
            How::Copy(f) if f.to_ascii_lowercase().ends_with(".dts") => Phase::Models,
            How::Copy(_) => Phase::Sounds,
            How::Image { .. } | How::Ifl(_) | How::Dml { .. } => Phase::Textures,
            How::Terrain(_) | How::Mission(_) => Phase::Maps,
            How::Recolour { .. } | How::Sound { .. } => Phase::Weapons,
            How::NormalMap(_) => Phase::NormalMaps,
            How::Picture { .. } => Phase::Pictures,
        }
    }
}

/// Where an import is: reported before each file ([`Options::progress`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub phase: Phase,
    /// Files done so far, of `total` (the whole import).
    pub done: usize,
    pub total: usize,
    /// The asset being made.
    pub file: String,
}

/// Progress callback ([`Options::progress`]).
pub type ProgressFn = Arc<dyn Fn(&Progress) + Send + Sync>;

/// A mission picture to render: the asset to write, the mission id and the
/// camera (`x y z` position, `x y z` target) of `missions/SOURCES.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct PictureJob {
    pub asset: String,
    pub mission: String,
    pub camera: [f32; 6],
}

/// Renders one mission picture into `root/job.asset` with the assets of
/// `root` (the import's output) at [`PICTURE_SIZE`] ([`Options::pictures`]).
/// The game supplies it (`tt_engine::mission_picture`): this crate cannot
/// depend on the game.
pub type PictureFn = Arc<dyn Fn(&Path, &PictureJob) -> Result<(), String> + Send + Sync>;

/// The error text of a cancelled import ([`Options::cancel`]).
pub const CANCELLED: &str = "import cancelled";

/// The file of an imported asset directory naming the version of the
/// program whose own files it has ([`refresh_ours`]).
pub const OURS_VERSION: &str = ".ours-version";

/// Rewrites OpenTT's own built-in files ([`crate::OURS`]) in the imported asset
/// directory `out` when they are another version's (the game was updated
/// since the import); returns whether it did.
pub fn refresh_ours(out: &Path) -> Result<bool, String> {
    let version = env!("CARGO_PKG_VERSION");
    if std::fs::read_to_string(out.join(OURS_VERSION)).is_ok_and(|v| v.trim() == version) {
        return Ok(false);
    }
    for (asset, bytes) in crate::OURS {
        if let Some(b) = bytes {
            write_file(&out.join(asset), b)?;
        }
    }
    Ok(true)
}

/// Writes `bytes` to `p`, creating its directory.
fn write_file(p: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(p, bytes).map_err(|e| format!("{}: {e}", p.display()))
}

/// What to import and how.
#[derive(Clone)]
pub struct Options {
    /// The ThinkTanks install (the directory with `game/`).
    pub game: PathBuf,
    /// The asset directory to write (created; files there are overwritten).
    pub out: PathBuf,
    /// The directory holding OpenTT's own files (the repository's `assets/`);
    /// `None`: the copies built into this program ([`crate::OURS`]).
    pub ours: Option<PathBuf>,
    /// Generate the normal maps (`*.normal.png`).
    pub normal_maps: bool,
    /// Renders the mission pictures; `None`: the pictures of the install
    /// are converted and OpenTT's own maps get a placeholder.
    pub pictures: Option<PictureFn>,
    /// ffmpeg for the sounds made from the game's (`ffmpeg`; empty: do not
    /// run it). Without it the built-in mixer makes them ([`crate::sound`]).
    pub ffmpeg: String,
    /// Print each file written.
    pub verbose: bool,
    /// Called before each file.
    pub progress: Option<ProgressFn>,
    /// Set to stop the import (it then fails with [`CANCELLED`]).
    pub cancel: Option<Arc<AtomicBool>>,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("game", &self.game)
            .field("out", &self.out)
            .field("ours", &self.ours)
            .field("normal_maps", &self.normal_maps)
            .field("pictures", &self.pictures.is_some())
            .field("ffmpeg", &self.ffmpeg)
            .field("verbose", &self.verbose)
            .finish_non_exhaustive()
    }
}

impl Options {
    pub fn new(game: impl Into<PathBuf>, out: impl Into<PathBuf>) -> Options {
        Options { game: game.into(), out: out.into(), ours: None, normal_maps: true, pictures: None, ffmpeg: "ffmpeg".into(), verbose: false, progress: None, cancel: None }
    }

    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

/// An asset the import did not make, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Skipped {
    pub asset: String,
    pub reason: String,
}

/// What the import did.
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub written: usize,
    /// Files made in a fallback way (no ffmpeg, no renderer, a mission
    /// file decoded from UTF-16, ...).
    pub warnings: Vec<String>,
    pub skipped: Vec<Skipped>,
    /// Files of the install the import looked for and did not find (in
    /// any accepted form, [`crate::locate::game_file`]); it went on without them.
    pub missing: Vec<String>,
}

impl Report {
    /// The tutorial's slides were all imported (the menus offer the tutorial).
    pub fn tutorial_available(&self) -> bool {
        !self.skipped.iter().any(|s| crate::mapping::tutorial_slide(&s.asset))
    }

    /// The report, a line each (the import screen, the import log): the
    /// counts, then the skipped assets, the files not found, the warnings.
    pub fn lines(&self) -> Vec<String> {
        let count = |n: usize, what: &str| if n == 0 { String::new() } else { format!(", {n} {what}") };
        let warnings = if self.warnings.len() == 1 { "warning" } else { "warnings" };
        let mut v = vec![format!("{} files imported{}{}{}.", self.written, count(self.skipped.len(), "skipped"), count(self.missing.len(), "optional files not found in the install"), count(self.warnings.len(), warnings))];
        if !self.tutorial_available() {
            v.push("The install has no tutorial art: the tutorial is not offered.".into());
        }
        let skipped = self.skipped.iter().map(|s| format!("{}: {}", s.asset, s.reason)).collect();
        for (title, items) in [("Skipped", skipped), ("Not found in the install", self.missing.clone()), ("Warnings", self.warnings.clone())] {
            if !items.is_empty() {
                v.push(format!("{title} ({}):", items.len()));
                v.extend(items.iter().map(|i| format!("  {i}")));
            }
        }
        v
    }

    fn skip(&mut self, asset: &str, reason: impl Into<String>) {
        self.skipped.push(Skipped { asset: asset.to_string(), reason: reason.into() });
    }

    fn not_found(&mut self, file: &str) {
        if !self.missing.iter().any(|m| m == file) {
            self.missing.push(file.to_string());
        }
    }
}

/// Whether `game` looks like a ThinkTanks install.
pub fn is_install(game: &Path) -> bool {
    tt_assets::find_ci(&game.join("game/data/missions")).is_some() && tt_assets::find_ci(&game.join("game/server/scripts")).is_some()
}

pub struct Importer<'a> {
    pub opts: &'a Options,
    pub mapping: &'a Mapping,
    index: HashMap<String, String>,
    report: Report,
    /// Files done and the total, for [`Options::progress`].
    done: usize,
    total: usize,
}

impl<'a> Importer<'a> {
    pub fn new(opts: &'a Options, mapping: &'a Mapping) -> Importer<'a> {
        let mut index = HashMap::new();
        for e in &mapping.files {
            let from = match &e.how {
                How::Copy(f) | How::Ifl(f) | How::Terrain(f) | How::Mission(f) => f,
                How::Dml { from, .. } | How::Image { from, .. } => from,
                _ => continue,
            };
            index.insert(from.to_ascii_lowercase(), e.asset.clone());
        }
        for (game_file, asset) in &mapping.replaced {
            index.insert(game_file.to_ascii_lowercase(), asset.clone());
        }
        Importer { opts, mapping, index, report: Report::default(), done: 0, total: 0 }
    }

    /// Reports the next file (`file` of `phase`); `Err` when cancelled.
    fn step(&mut self, phase: Phase, file: &str) -> Result<(), String> {
        if self.opts.cancelled() {
            return Err(CANCELLED.into());
        }
        if let Some(p) = &self.opts.progress {
            p(&Progress { phase, done: self.done, total: self.total.max(self.done), file: file.to_string() });
        }
        self.done += 1;
        Ok(())
    }

    pub fn reference(&self, value: &str, dir: &str) -> Option<String> {
        let v = value.replace('\\', "/");
        let lower = v.to_ascii_lowercase();
        let p = if let Some(r) = v.strip_prefix("~/") {
            format!("game/{r}")
        } else if let Some(r) = v.strip_prefix("./") {
            format!("{dir}/{r}")
        } else if lower.starts_with("game/") || lower.starts_with("common/") {
            v.clone()
        } else {
            return None;
        };
        let l = p.to_ascii_lowercase();
        if let Some(a) = self.index.get(&l) {
            return Some(a.clone());
        }
        // A texture named without its extension or with another one; a
        // sound named `.wav` that is an `.ogg`.
        let (stem, exts): (&str, &[&str]) = match l.rsplit_once('.') {
            Some((s, e)) if tt_assets::IMAGE_EXTENSIONS.contains(&e) => (s, &tt_assets::IMAGE_EXTENSIONS),
            Some((s, "wav" | "ogg")) => (s, &["ogg", "wav"]),
            _ => (&l, &tt_assets::IMAGE_EXTENSIONS),
        };
        exts.iter().find_map(|e| self.index.get(&format!("{stem}.{e}")).cloned())
    }

    /// The install's file for `path`, in any accepted form ([`crate::locate::game_file`]).
    fn game_file(&self, path: &str) -> Result<PathBuf, String> {
        crate::locate::game_file(&self.opts.game, path).ok_or_else(|| format!("{path}: not in the game install {}", self.opts.game.display()))
    }

    fn read_game_file(&self, path: &str) -> Result<Vec<u8>, String> {
        let p = self.game_file(path)?;
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
    }

    fn out(&self, asset: &str) -> PathBuf {
        self.opts.out.join(asset)
    }

    fn write(&mut self, asset: &str, bytes: &[u8]) -> Result<(), String> {
        write_file(&self.out(asset), bytes)?;
        self.wrote(asset);
        Ok(())
    }

    fn wrote(&mut self, asset: &str) {
        self.report.written += 1;
        if self.opts.verbose {
            println!("{asset}");
        }
    }

    fn file_name(asset: &str) -> &str {
        asset.rsplit('/').next().unwrap_or(asset)
    }

    /// An `.ifl`: each line's frame name → the frame's asset file name.
    fn ifl(&self, from: &str) -> Result<String, String> {
        let text = tt_assets::latin1(&self.read_game_file(from)?);
        let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        let mut out = String::new();
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                out.push('\n');
            }
            let body = line.trim_end_matches('\r');
            let frame = body.split_whitespace().next().unwrap_or("");
            if frame.is_empty() {
                out.push_str(line);
                continue;
            }
            let asset = self.reference(&format!("./{frame}"), dir).ok_or_else(|| format!("{from}: frame {frame} is not imported"))?;
            let at = line.find(frame).unwrap_or(0);
            out.push_str(&line[..at]);
            out.push_str(Self::file_name(&asset));
            out.push_str(&line[at + frame.len()..]);
        }
        Ok(out)
    }

    /// A `.dml`: the first `keep` entries, by their asset file names.
    fn dml(&self, from: &str, keep: usize) -> Result<String, String> {
        let text = tt_assets::latin1(&self.read_game_file(from)?);
        let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        let mut out = String::new();
        for name in text.lines().map(str::trim).filter(|l| !l.is_empty()).take(keep) {
            let asset = self.reference(&format!("./{name}"), dir).ok_or_else(|| format!("{from}: {name} is not imported"))?;
            out.push_str(Self::file_name(&asset));
            out.push('\n');
        }
        Ok(out)
    }

    /// A `.ter` with its material names as asset paths.
    fn terrain(&self, from: &str) -> Result<Vec<u8>, String> {
        let mut t = tt_assets::terrain::Terrain::parse(&self.read_game_file(from)?).map_err(|e| format!("{from}: {e}"))?;
        for m in t.materials.iter_mut().filter(|m| !m.is_empty()) {
            *m = self.reference(m, "").ok_or_else(|| format!("{from}: material {m} is not imported"))?;
        }
        t.to_bytes().map_err(|e| format!("{from}: {e}"))
    }

    fn image(&mut self, asset: &str, from: &str, alpha: Option<&str>) -> Result<(), String> {
        let color = images::open(&self.game_file(from)?)?;
        let mask = match alpha {
            Some(a) => match self.game_file(a) {
                Ok(p) => Some(images::open(&p)?),
                Err(_) => {
                    // Another edition's image with its alpha in it (the
                    // Mac edition's PNGs), or none: opaque.
                    if !color.color().has_alpha() {
                        self.report.not_found(a);
                        self.report.warnings.push(format!("{asset}: imported without transparency ({a} is not in the install)"));
                    }
                    None
                }
            },
            None => None,
        };
        let img = match mask {
            Some(m) => image::DynamicImage::ImageRgba8(images::merge_alpha(&color, &m)),
            None => images::normalized(color),
        };
        images::save_png(&img, &self.out(asset))?;
        self.wrote(asset);
        Ok(())
    }

    fn sound(&mut self, asset: &str, from: &[String], filter: &str, level: Level) -> Result<(), String> {
        let out = self.out(asset);
        let mut args: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into(), "-y".into()];
        for f in from {
            args.push("-i".into());
            args.push(self.out(f).to_string_lossy().into_owned());
        }
        // Two passes: the graph's loudness / peak, then the gain to `level`.
        let mut probe = args.clone();
        probe.extend(["-filter_complex".into(), format!("{filter};[out]ebur128,volumedetect[o2]"), "-map".into(), "[o2]".into(), "-f".into(), "null".into(), "-".into()]);
        let o = std::process::Command::new(&self.opts.ffmpeg).args(&probe).output().map_err(|e| format!("{}: {e}", self.opts.ffmpeg))?;
        if !o.status.success() {
            return Err(format!("{asset}: ffmpeg failed ({})", o.status));
        }
        let log = String::from_utf8_lossy(&o.stderr);
        let num = |key: &str| log.lines().rev().find_map(|l| l.trim().strip_prefix(key)).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse::<f32>().ok());
        let gain = match level {
            Level::Lufs(target) => target - num("I:").ok_or_else(|| format!("{asset}: ffmpeg ebur128 gave no loudness"))?,
            Level::Peak(target) => target - log.lines().find_map(|l| l.split("max_volume: ").nth(1)).and_then(|v| v.trim_end_matches(" dB").trim().parse::<f32>().ok()).ok_or_else(|| format!("{asset}: ffmpeg volumedetect gave no peak"))?,
        };
        args.extend(["-filter_complex".into(), format!("{filter};[out]volume={gain:.2}dB[o2]"), "-map".into(), "[o2]".into()]);
        args.extend(["-v".into(), "error".into(), "-ac".into(), "1".into(), "-ar".into(), "44100".into(), "-c:a".into(), "libvorbis".into(), "-q:a".into(), "4".into(), "-map_metadata".into(), "-1".into(), "-fflags".into(), "+bitexact".into(), out.to_string_lossy().into_owned()]);
        if let Some(d) = out.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let status = std::process::Command::new(&self.opts.ffmpeg).args(&args).status().map_err(|e| format!("{}: {e}", self.opts.ffmpeg))?;
        if !status.success() {
            return Err(format!("{asset}: ffmpeg failed ({status})"));
        }
        self.wrote(asset);
        Ok(())
    }

    fn picture(&mut self, asset: &str, mission: &str, game_file: Option<&str>) -> Result<(), String> {
        // The install's picture, when it has it; else as for OpenTT's own maps.
        let found = game_file.and_then(|o| {
            let p = crate::locate::game_file(&self.opts.game, o);
            if p.is_none() {
                self.report.not_found(o);
            }
            p
        });
        if let Some(p) = found {
            let img = images::normalized(images::open(&p)?);
            images::save_png(&img, &self.out(asset))?;
        } else {
            // OpenTT's own map: its terrain, shaded, tinted by its world.
            let ter = self.out(&format!("{}/{mission}.ter", tt_assets::layout::MISSIONS));
            let bytes = std::fs::read(&ter).map_err(|e| format!("{}: {e}", ter.display()))?;
            let t = tt_assets::terrain::Terrain::parse(&bytes).map_err(|e| format!("{}: {e}", ter.display()))?;
            let world = t.materials.iter().find(|m| !m.is_empty()).cloned().unwrap_or_default();
            let tint = if world.contains("/blue/") {
                [0.55, 0.65, 0.95]
            } else if world.contains("/red/") {
                [0.95, 0.55, 0.4]
            } else {
                [0.55, 0.9, 0.5]
            };
            images::save_png(&images::terrain_picture(&t, tint, PICTURE_SIZE.0 / 4, PICTURE_SIZE.1 / 4), &self.out(asset))?;
            if self.opts.pictures.is_none() {
                self.report.warnings.push(format!("{asset}: placeholder picture (the shaded terrain); the game's import renders it"));
            }
        }
        self.wrote(asset);
        Ok(())
    }

    fn one(&mut self, e: &Entry) -> Result<(), String> {
        let asset = e.asset.as_str();
        match &e.how {
            How::Copy(f) => {
                let b = self.read_game_file(f)?;
                self.write(asset, &b)
            }
            How::Terrain(f) => {
                let b = self.terrain(f)?;
                self.write(asset, &b)
            }
            How::Image { from, alpha, .. } => self.image(asset, from, alpha.as_deref()),
            How::Recolour { from, hue, min_saturation, value, thicken, at_width } => {
                let src = images::open(&self.out(from))?;
                let img = images::recolour(&src, *hue, *min_saturation, *value, *thicken, *at_width);
                images::save_png(&image::DynamicImage::ImageRgba8(img), &self.out(asset))?;
                self.wrote(asset);
                Ok(())
            }
            How::Sound { from, filter, level } => {
                let ffmpeg = if self.opts.ffmpeg.is_empty() { Err("ffmpeg not used".to_string()) } else { self.sound(asset, from, filter, *level) };
                let Err(err) = ffmpeg else { return Ok(()) };
                // No ffmpeg: the built-in mixer (a WAV in the .ogg file; the
                // game reads either by its content).
                let inputs: Result<Vec<Vec<u8>>, String> = from.iter().map(|f| std::fs::read(self.out(f)).map_err(|e| format!("{f}: {e}"))).collect();
                match inputs.and_then(|i| crate::sound::render(&i, filter, *level)) {
                    Ok(wav) => {
                        self.write(asset, &wav)?;
                        self.report.warnings.push(format!("{asset}: made by the built-in mixer ({err})"));
                    }
                    Err(e2) => {
                        // The first input, unchanged, so the game still has a sound there.
                        let b = std::fs::read(self.out(&from[0])).map_err(|e| format!("{}: {e}", from[0]))?;
                        self.write(asset, &b)?;
                        self.report.warnings.push(format!("{asset}: {err}; {e2}; copied {} instead", from[0]));
                    }
                }
                Ok(())
            }
            How::NormalMap(from) => {
                let src = images::open(&self.out(from))?;
                if let Some(m) = tt_assets::normal_map::normal_map(&src) {
                    images::save_png(&image::DynamicImage::ImageRgb8(m), &self.out(asset))?;
                    self.wrote(asset);
                }
                Ok(())
            }
            How::Picture { mission, game_file, .. } => self.picture(asset, mission, game_file.as_deref()),
            How::Mission(f) => {
                let (text, notes) = convert::mission_file(f, &self.read_game_file(f)?, &|v: &str, dir: &str| self.reference(v, dir))?;
                self.report.warnings.extend(notes.into_iter().map(|n| format!("{f}: {n}")));
                self.write(asset, text.as_bytes())
            }
            How::Ifl(f) => {
                let text = self.ifl(f)?;
                self.write(asset, text.as_bytes())
            }
            How::Dml { from, keep } => {
                let text = self.dml(from, *keep)?;
                self.write(asset, text.as_bytes())
            }
        }
    }

    /// Copies OpenTT's own files.
    fn ours(&mut self) -> Result<(), String> {
        for asset in &self.mapping.ours {
            self.step(Phase::Bundled, asset)?;
            let b = match &self.opts.ours {
                Some(dir) => {
                    let src = dir.join(asset);
                    std::fs::read(&src).map_err(|e| format!("{} (built-in file): {e}", src.display()))?
                }
                None => match crate::OURS.iter().find(|(a, _)| a == asset) {
                    Some((_, Some(b))) => b.to_vec(),
                    _ => return Err(format!("{asset} (built-in file) is not built into this program; pass the directory that has it (--ours)")),
                },
            };
            self.write(asset, &b)?;
        }
        Ok(())
    }

    /// Required files that are missing are not skipped (the import fails
    /// on them).
    fn skip_reason(&mut self, e: &Entry) -> Option<String> {
        let inputs: Vec<&String> = match &e.how {
            How::Recolour { from, .. } | How::NormalMap(from) => vec![from],
            How::Sound { from, .. } => from.iter().collect(),
            _ => Vec::new(),
        };
        if let Some(i) = inputs.iter().find(|i| self.report.skipped.iter().any(|s| &s.asset == **i)) {
            return Some(format!("made from {i}, which was not imported"));
        }
        if !e.optional() || matches!(e.how, How::Picture { .. }) {
            return None;
        }
        let required = e.how.required_files();
        let absent: Vec<String> = e.how.game_files().into_iter().filter(|f| crate::locate::game_file(&self.opts.game, f).is_none()).collect();
        if !absent.iter().any(|f| required.contains(f)) {
            return None;
        }
        absent.iter().for_each(|f| self.report.not_found(f));
        let absent: Vec<&str> = absent.iter().filter(|f| required.contains(f)).map(String::as_str).collect();
        Some(if crate::mapping::tutorial_slide(&e.asset) { "optional, not found: tutorial disabled".into() } else { format!("optional, not found: {}", absent.join(", ")) })
    }

    /// Runs the whole import.
    pub fn run(mut self) -> Result<Report, String> {
        if !is_install(&self.opts.game) {
            return Err(format!("{}: not a ThinkTanks install (no game/data/missions, game/server/scripts)", self.opts.game.display()));
        }
        std::fs::create_dir_all(&self.opts.out).map_err(|e| format!("{}: {e}", self.opts.out.display()))?;
        let phase = |h: &How| match h {
            How::Recolour { .. } | How::Sound { .. } => 1,
            How::NormalMap(_) => 2,
            How::Picture { .. } => 3,
            _ => 0,
        };
        let normal_maps = self.opts.normal_maps;
        let wanted = |e: &&Entry| normal_maps || !matches!(e.how, How::NormalMap(_));
        self.total = self.mapping.ours.len() + self.mapping.files.iter().filter(wanted).count();
        self.ours()?;
        let mut errors = Vec::new();
        for p in 0..4 {
            for e in self.mapping.files.iter().filter(|e| phase(&e.how) == p).filter(wanted) {
                self.step(Phase::of(e), &e.asset)?;
                if let Some(reason) = self.skip_reason(e) {
                    self.report.skip(&e.asset, reason);
                    continue;
                }
                let r = match (&e.how, &self.opts.pictures) {
                    (How::Picture { mission, camera, .. }, Some(render)) => {
                        let job = PictureJob { asset: e.asset.clone(), mission: mission.clone(), camera: *camera };
                        match render(&self.opts.out, &job) {
                            Ok(()) => {
                                self.wrote(&e.asset);
                                Ok(())
                            }
                            Err(err) => {
                                // No renderer here (no GPU, ...): as without one.
                                self.report.warnings.push(format!("{}: not rendered ({err})", e.asset));
                                self.one(e)
                            }
                        }
                    }
                    _ => self.one(e),
                };
                match (r, &e.how) {
                    // A mission that cannot be read: left out (the game lists the
                    // missions it has), the import goes on; a compiled copy next
                    // to it is not read.
                    (Err(err), How::Mission(f)) => {
                        let dso = crate::locate::game_file(&self.opts.game, &format!("{f}.dso")).map_or(String::new(), |_| format!("; the install also has {f}.dso, a compiled copy the import does not read"));
                        self.report.skip(&e.asset, format!("mission left out, its file could not be read ({err}){dso}"));
                    }
                    (Err(err), _) => errors.push(err),
                    (Ok(()), _) => {}
                }
            }
        }
        if errors.is_empty() {
            // Only a complete import marks the data as this version's.
            std::fs::write(self.opts.out.join(OURS_VERSION), env!("CARGO_PKG_VERSION")).map_err(|e| format!("{OURS_VERSION}: {e}"))?;
            Ok(self.report)
        } else {
            Err(errors.join("\n"))
        }
    }
}

/// Size of the rendered mission pictures (the menu's).
pub const PICTURE_SIZE: (u32, u32) = (1528, 528);

/// Imports with the built-in mapping.
pub fn import(opts: &Options) -> Result<Report, String> {
    let mapping = Mapping::builtin();
    Importer::new(opts, &mapping).run()
}

/// Why [`install`] did not install.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallError {
    /// [`Options::cancel`] was set.
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Cancelled => f.write_str(CANCELLED),
            InstallError::Failed(e) => f.write_str(e),
        }
    }
}

/// `dir` with `suffix` appended to its name (`gamedata` → `gamedata.tmp`).
pub fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "gamedata".into());
    name.push(suffix);
    dir.with_file_name(name)
}

/// [`import`] all or nothing, as the game's import screen runs it: into
/// `<out>.tmp`, which replaces `out` only when the import succeeded (the
/// previous `out` is removed then); a failed or cancelled import removes
/// what it wrote and leaves `out` as it was.
pub fn install(opts: &Options) -> Result<Report, InstallError> {
    let target = opts.out.clone();
    let tmp = sibling(&target, ".tmp");
    let old = sibling(&target, ".old");
    let failed = |e: String| InstallError::Failed(e);
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| failed(format!("{}: {e}", parent.display())))?;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    let mut staged = opts.clone();
    staged.out = tmp.clone();
    let report = match import(&staged) {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(if e == CANCELLED || opts.cancelled() { InstallError::Cancelled } else { failed(e) });
        }
    };
    if opts.cancelled() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(InstallError::Cancelled);
    }
    if let Some(p) = &opts.progress {
        p(&Progress { phase: Phase::Finishing, done: report.written, total: report.written, file: String::new() });
    }
    let _ = std::fs::remove_dir_all(&old);
    if target.exists() {
        std::fs::rename(&target, &old).map_err(|e| failed(format!("{}: {e}", target.display())))?;
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        // Put the previous data back.
        let _ = std::fs::rename(&old, &target);
        return Err(failed(format!("{}: {e}", target.display())));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(report)
}

#[cfg(test)]
mod tests {
    /// An import of an older version gets this version's own files (every
    /// start, until an import stamps it); an up-to-date one is left alone.
    #[test]
    fn refresh_ours_rewrites_an_older_versions_files() {
        let out = std::env::temp_dir().join(format!("tt-refresh-ours-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join(super::OURS_VERSION), "0.1.2").unwrap();
        let (asset, bytes) = crate::OURS.iter().find_map(|(a, b)| b.map(|b| (*a, b))).unwrap();
        assert!(super::refresh_ours(&out).unwrap());
        assert_eq!(std::fs::read(out.join(asset)).unwrap(), bytes);
        assert_eq!(std::fs::read_to_string(out.join(super::OURS_VERSION)).unwrap(), "0.1.2", "still the old import's");
        assert!(super::refresh_ours(&out).unwrap(), "still stale");
        std::fs::write(out.join(super::OURS_VERSION), env!("CARGO_PKG_VERSION")).unwrap();
        assert!(!super::refresh_ours(&out).unwrap(), "up to date: nothing written");
        let _ = std::fs::remove_dir_all(&out);
    }
}
