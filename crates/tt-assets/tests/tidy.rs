//! The asset directory stays tidy (`docs/layout.md`): every reference to an
//! asset names an existing file with its exact spelling, and every file of
//! the directory is used.
//!
//! References are followed from the roots the game loads by name — the data
//! files (`tt_assets::data`), the asset paths written in the crates' (and generators') Rust
//! sources (a `{…}` in a format string matches any text, a `*` in a file
//! pattern too) and in the RON files — through what the files themselves
//! name: terrain materials (`.ter`), sky lists (`.dml`), shape materials
//! (`.dts`, case-insensitive: the names are inside the binary), animated
//! texture lists (`.ifl`) and the `X.normal.png` normal map next to a used
//! texture. Only the `SOURCES.md` provenance tables are present without
//! being used.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use tt_assets::data::defs as dbf;

/// The top-level directories of the asset directory.
const TOP: &[&str] = &["data", "missions", "models", "sky", "terrain", "sound", "ui"];

/// Asset-path literals of the crates' tests that name made-up files (a
/// temporary asset directory, a parser round trip).
const FIXTURES: &[&str] = &[
    "missions/a.ron",
    "missions/foo.ron",
    "models/x.dts",
    "sound/music/a.ogg",
    "sound/voice/default/dryfire.wav",
    // Mod overlays (tt_assets::mods, tt-engine tests/mods.rs).
    "models/tanks/tank01",
    "models/tanks/tank01.png",
    "models/tanks/tread.png",
    // The example mods' files (tests/example_mods.rs).
    "models/tanks/tank02.png",
    "sound/boom.ogg",
    "sound/readme",
    "data/rule_pack_marker.ron",
    "missions/modarena.ron",
    "missions/offmap.ron",
    "missions/oldarena.ron",
];

fn root() -> PathBuf {
    tt_assets::layout::default_dir()
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The string literals of a Rust or RON text (escapes kept as written;
/// raw strings and chars are rare enough in this code base to ignore).
fn literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'\'' => {
                // A char literal ('"', '\'') or a lifetime: skip the char.
                if b.get(i + 1) == Some(&b'\\') {
                    i += 3;
                } else if b.get(i + 2) == Some(&b'\'') {
                    i += 2;
                }
                i += 1;
            }
            b'"' => {
                let start = i + 1;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                out.push(text[start..i.min(text.len())].to_string());
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// `s` as an asset path if it looks like one (`<top>/...`, optionally
/// `assets/<top>/...`).
fn asset_path(s: &str) -> Option<&str> {
    let s = s.strip_prefix("assets/").unwrap_or(s);
    let (top, rest) = s.split_once('/')?;
    (TOP.contains(&top) && !rest.is_empty() && !s.contains(char::is_whitespace) && !s.contains('\\')).then_some(s)
}

fn wildcard(pattern: &str, s: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), s.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

struct Walk {
    root: PathBuf,
    /// Every file of the asset directory (asset path -> file).
    files: BTreeMap<String, PathBuf>,
    used: BTreeSet<String>,
    todo: Vec<String>,
    problems: Vec<String>,
}

impl Walk {
    fn new() -> Walk {
        let root = root();
        let files = tt_assets::layout::files(&root).into_iter().collect();
        Walk { root, files, used: BTreeSet::new(), todo: Vec::new(), problems: Vec::new() }
    }

    fn mark(&mut self, rel: &str) {
        if self.used.insert(rel.to_string()) {
            self.todo.push(rel.to_string());
        }
    }

    /// A reference written in data or code: the exact file must exist.
    fn reference(&mut self, rel: &str, from: &str) {
        if self.files.contains_key(rel) {
            self.mark(rel);
        } else if self.root.join(rel).is_dir() {
        } else if let Some(f) = self.files.keys().find(|f| f.eq_ignore_ascii_case(rel)).cloned() {
            self.problems.push(format!("{from}: {rel:?} is spelled {f:?} on disk"));
            self.mark(&f);
        } else {
            self.problems.push(format!("{from}: {rel:?} does not exist"));
        }
    }

    /// A pattern (`{…}` / `*` / `?`): marks what it matches, if anything.
    fn pattern(&mut self, pattern: &str, from: &str) {
        let mut p = String::new();
        let mut depth = 0;
        for c in pattern.chars() {
            match c {
                '{' => {
                    if depth == 0 {
                        p.push('*');
                    }
                    depth += 1;
                }
                '}' => depth -= 1,
                _ if depth == 0 => p.push(c),
                _ => {}
            }
        }
        let hits: Vec<String> = self.files.keys().filter(|f| wildcard(&p, f)).cloned().collect();
        let dir = p.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        if hits.is_empty() && !dir.contains('*') && !self.root.join(dir).is_dir() {
            self.problems.push(format!("{from}: {pattern:?} matches no file"));
        }
        for h in hits {
            self.mark(&h);
        }
    }

    /// A name a binary file holds, looked up next to it ignoring case.
    fn sibling_ci(&mut self, dir: &str, name: &str, from: &str) -> Option<String> {
        let want = format!("{dir}/{name}");
        match self.files.keys().find(|f| f.eq_ignore_ascii_case(&want)).cloned() {
            Some(f) => {
                self.mark(&f);
                Some(f)
            }
            None => {
                self.problems.push(format!("{from}: names {name:?}, not found in {dir}/"));
                None
            }
        }
    }

    fn follow(&mut self, rel: &str) {
        let path = self.files[rel].clone();
        let dir = rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "ron" => {
                let text = std::fs::read_to_string(&path).unwrap();
                for l in literals(&text) {
                    if l.starts_with("~/") || l.starts_with("./") || l.to_ascii_lowercase().starts_with("game/") {
                        self.problems.push(format!("{rel}: {l:?} is not an asset path"));
                    } else {
                        // A list such as sky_sprites: "sky/a.png 8 9 5; sky/b.png ...".
                        for p in l.split(';').filter_map(|s| s.split_whitespace().next()).filter_map(asset_path) {
                            self.reference(p, rel);
                        }
                    }
                }
            }
            "ter" => {
                let t = tt_assets::terrain::Terrain::parse(&std::fs::read(&path).unwrap()).unwrap();
                for m in t.materials.iter().filter(|m| !m.is_empty()) {
                    self.reference(m, rel);
                }
            }
            "dml" => {
                let text = std::fs::read_to_string(&path).unwrap();
                for l in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    self.reference(&format!("{dir}/{l}"), rel);
                }
            }
            "ifl" => {
                let text = std::fs::read_to_string(&path).unwrap();
                for l in text.lines() {
                    if let Some(frame) = l.split_whitespace().next() {
                        self.reference(&format!("{dir}/{frame}"), rel);
                    }
                }
            }
            "dts" => {
                let data = std::fs::read(&path).unwrap();
                let shape = match tt_assets::dts::Shape::parse(&data) {
                    Ok(s) => s,
                    // `models/markers/octahedron.dts`: a pre-v19 editor
                    // marker (no textures; the game never draws it).
                    Err(_) if u16::from_le_bytes([data[0], data[1]]) < 19 => return,
                    Err(e) => panic!("{rel}: {e}"),
                };
                for m in &shape.materials {
                    let name = m.texture_name();
                    let ifl = m.is_ifl() || m.flags & tt_assets::dts::Material::IFL_MATERIAL != 0;
                    let file = if ifl {
                        format!("{name}.ifl")
                    } else {
                        let Some(f) = tt_assets::find_texture(&self.root.join(&dir).join(name)) else {
                            self.problems.push(format!("{rel}: material {name:?} has no texture"));
                            continue;
                        };
                        f.file_name().unwrap().to_string_lossy().into_owned()
                    };
                    self.sibling_ci(&dir, &file, rel);
                }
            }
            "png" | "jpg" => {
                if let Some(stem) = rel.strip_suffix(".png").or_else(|| rel.strip_suffix(".jpg")) {
                    let normal = format!("{stem}.normal.png");
                    if self.files.contains_key(&normal) {
                        self.mark(&normal);
                    }
                }
            }
            _ => {}
        }
    }

    /// Everything reachable from the data files and the Rust sources.
    fn run() -> Walk {
        let mut w = Walk::new();
        let root = w.root.clone();
        let rel = |p: PathBuf| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        let mut roots: Vec<String> = dbf::SETS.iter().map(|stem| rel(dbf::file_path(&root, stem))).collect();
        roots.push(rel(dbf::campaign_path(&root)));
        for r in roots {
            w.reference(&r, "tt_assets::data");
        }
        // The generators' and capture tools' sources where the repository has them
        // (not the script tools, which name the game's files).
        let mut sources = Vec::new();
        for dir in ["crates", "internal/gen", "internal/capture"].map(|d| repo().join(d)).into_iter().filter(|d| d.is_dir()) {
            rust_files(&dir, &mut sources);
        }
        for f in sources {
            let from = f.strip_prefix(repo()).unwrap_or(&f).display().to_string();
            for l in literals(&std::fs::read_to_string(&f).unwrap()) {
                let Some(p) = asset_path(&l) else { continue };
                if p.contains(['{', '*', '?']) {
                    w.pattern(p, &from);
                } else if !FIXTURES.contains(&p.to_ascii_lowercase().as_str()) {
                    w.reference(p, &from);
                }
            }
        }
        while let Some(rel) = w.todo.pop() {
            w.follow(&rel);
        }
        w
    }
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != "target" && n != "golden") {
                rust_files(&p, out);
            }
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn all_references_resolve() {
    tt_assets::require_game_data!();
    let w = Walk::run();
    assert!(w.problems.is_empty(), "{} broken references:\n{}", w.problems.len(), w.problems.join("\n"));
}

#[test]
fn assets_are_all_used() {
    tt_assets::require_game_data!();
    let w = Walk::run();
    let unused: Vec<&String> = w.files.keys().filter(|f| !w.used.contains(*f) && !f.ends_with("/SOURCES.md")).collect();
    assert!(unused.is_empty(), "{} files of the asset directory are used by nothing (delete them, or move them out of the asset directory):\n{}", unused.len(), unused.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
}

/// Every directory of the repository's `assets/` has its provenance
/// table, listing exactly its files (the full asset directory here; in a
/// checkout with OpenTT's own files only, those: the game data imported
/// elsewhere has no tables).
#[test]
fn every_file_has_its_provenance() {
    let root = repo().join("assets");
    let files: BTreeMap<String, PathBuf> = tt_assets::layout::files(&root).into_iter().collect();
    assert!(!files.is_empty(), "{}: no files", root.display());
    let w = Walk { root, files, used: BTreeSet::new(), todo: Vec::new(), problems: Vec::new() };
    let mut problems = Vec::new();
    let mut tables: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for top in TOP {
        let text = std::fs::read_to_string(w.root.join(top).join("SOURCES.md")).unwrap_or_default();
        let listed = text.lines().filter_map(|l| l.strip_prefix("| `")).filter_map(|l| l.split_once('`')).map(|(p, _)| p.to_string()).collect();
        tables.insert(top.to_string(), listed);
    }
    for f in w.files.keys().filter(|f| !f.ends_with("/SOURCES.md")) {
        let (top, rest) = f.split_once('/').unwrap_or(("", f));
        if !tables.get(top).is_some_and(|t| t.contains(rest)) {
            problems.push(format!("{f}: not in {top}/SOURCES.md"));
        }
    }
    for (top, listed) in &tables {
        for p in listed {
            if !w.files.contains_key(&format!("{top}/{p}")) {
                problems.push(format!("{top}/SOURCES.md lists {p}, which does not exist"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Directory names are lowercase `snake_case` (file names are lowercase:
/// [`file_names_are_lowercase`]).
#[test]
fn directory_names() {
    let w = Walk::new();
    let dirs: BTreeSet<&str> = w.files.keys().filter_map(|f| f.rsplit_once('/').map(|(d, _)| d)).collect();
    let bad: Vec<&&str> = dirs.iter().filter(|d| d.split('/').any(|c| !c.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'))).collect();
    assert!(bad.is_empty(), "{bad:?}");
    for f in w.files.keys() {
        assert!(TOP.contains(&f.split('/').next().unwrap()), "{f}: outside the top-level directories {TOP:?}");
    }
}

/// The example mods of the repository's `mods/` (`docs/modding.md` §
/// Example mods; the folders whose `mod.ron` says `example: true`): each
/// has a `mod.ron` by OpenTT, a `README.md` and a `SOURCES.md` listing
/// exactly its other files; its game files lie in the asset directory's
/// top-level directories, are named in lowercase and replace a file of the
/// game data (the game's name in any case).
#[test]
fn example_mods_are_tidy() {
    let examples: Vec<_> = tt_assets::mods::discover(&[repo().join(tt_assets::layout::MODS)]).into_iter().filter(|m| m.example).collect();
    assert!(examples.len() >= 2, "the example mods are missing from mods/");
    let game: Option<BTreeSet<String>> = tt_assets::testdata::present_in(&root()).then(|| tt_assets::layout::files(&root()).into_iter().map(|(r, _)| r.to_ascii_lowercase()).collect());
    let mut problems = Vec::new();
    for m in &examples {
        let text = std::fs::read_to_string(m.dir.join("SOURCES.md")).unwrap_or_default();
        let listed: BTreeSet<String> = text.lines().filter_map(|l| l.strip_prefix("| `")).filter_map(|l| l.split_once('`')).map(|(p, _)| p.to_string()).collect();
        let have: BTreeSet<String> = tt_assets::layout::files(&m.dir).into_iter().map(|(r, _)| r).filter(|r| r != "SOURCES.md").collect();
        if listed != have {
            problems.push(format!("{}: SOURCES.md lists {listed:?}, the folder has {have:?}", m.id));
        }
        for top in ["mod.ron", "README.md"] {
            if !have.contains(top) {
                problems.push(format!("{}: no {top}", m.id));
            }
        }
        if m.author != "OpenTT" {
            problems.push(format!("{}: author {:?}", m.id, m.author));
        }
        for (rel, _) in &m.files {
            if !TOP.contains(&rel.split('/').next().unwrap()) {
                problems.push(format!("{}/{rel}: outside the top-level directories {TOP:?}", m.id));
            }
            if *rel != rel.to_ascii_lowercase() {
                problems.push(format!("{}/{rel}: not lowercase", m.id));
            }
            // A normal map may be missing from the game data (`tt-import
            // --no-normal-maps`): its texture's file counts.
            let replaced = rel.to_ascii_lowercase().replace(".normal.png", ".png");
            if game.as_ref().is_some_and(|g| !g.contains(&rel.to_ascii_lowercase()) && !g.contains(&replaced)) {
                problems.push(format!("{}/{rel}: replaces no file of the game data", m.id));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Whether a file of the asset directory or of a mod may keep capitals:
/// only the provenance tables and a mod's README.
fn named_freely(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name == "SOURCES.md" || name == "README.md"
}

/// File names are lowercase (`docs/layout.md` § Naming), whatever case the
/// game's files have: the repository's `assets/` and the example mods (the
/// importer writes the same names: the mapping is made from the
/// `SOURCES.md` tables, `tt-import` tests).
#[test]
fn file_names_are_lowercase() {
    let mut bad: Vec<String> = tt_assets::layout::files(&repo().join("assets")).into_iter().map(|(r, _)| format!("assets/{r}")).collect();
    for m in tt_assets::mods::discover(&[repo().join(tt_assets::layout::MODS)]).into_iter().filter(|m| m.example) {
        bad.extend(tt_assets::layout::files(&m.dir).into_iter().map(|(r, _)| format!("mods/{}/{r}", m.id)));
    }
    bad.retain(|r| !named_freely(r) && *r != r.to_ascii_lowercase());
    assert!(bad.is_empty(), "{} file names are not lowercase (rename them and what names them):\n{}", bad.len(), bad.join("\n"));
}

/// Every material of every shape finds its texture (or its `.ifl`) beside
/// the shape: the names inside the DTS keep the game's case, the files are
/// lowercase, the lookup ignores case (`tt_assets::find_texture`).
#[test]
fn shape_materials_resolve_to_lowercase_files() {
    tt_assets::require_game_data!();
    let mut problems = Vec::new();
    let mut checked = 0;
    for (rel, path) in tt_assets::layout::files(&root()).into_iter().filter(|(r, _)| r.ends_with(".dts")) {
        let data = std::fs::read(&path).unwrap();
        // `models/markers/octahedron.dts`: a pre-v19 marker, no textures.
        let Ok(shape) = tt_assets::dts::Shape::parse(&data) else { continue };
        let dir = path.parent().unwrap();
        for m in &shape.materials {
            let name = m.texture_name();
            let ifl = m.is_ifl() || m.flags & tt_assets::dts::Material::IFL_MATERIAL != 0;
            let found = if ifl { tt_assets::find_ci(&dir.join(format!("{name}.ifl"))) } else { tt_assets::find_texture(&dir.join(name)) };
            match found {
                Some(f) => {
                    let file = f.file_name().unwrap().to_string_lossy().into_owned();
                    if file != file.to_ascii_lowercase() {
                        problems.push(format!("{rel}: material {name:?} -> {file}, not lowercase"));
                    }
                    checked += 1;
                }
                None => problems.push(format!("{rel}: material {name:?} has no file beside the shape")),
            }
        }
    }
    assert!(checked > 100, "only {checked} materials checked");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
