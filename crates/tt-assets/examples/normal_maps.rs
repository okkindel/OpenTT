//! Generates the normal maps of the "normal maps" graphics option
//! ([`tt_assets::normal_map`]): for every shape and terrain texture in the
//! asset directory, `NAME.normal.png` next to `NAME.png`. Puts their
//! provenance rows into `assets/models/SOURCES.md` and
//! `assets/terrain/SOURCES.md` (replacing the old `.normal.png` rows).
//!
//! ```sh
//! cargo run --release -p tt-assets --example normal_maps -- assets
//! ```

use std::path::{Path, PathBuf};
use tt_assets::normal_map;

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "assets".into()));
    let mut count = 0;
    for sub in ["models", "terrain"] {
        let dir = root.join(sub);
        let list = normal_map::textures(&dir);
        let rel = |p: &Path| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/");
        let mut rows = Vec::new();
        for src in &list {
            let Some(map) = image::open(src).ok().and_then(|img| normal_map::normal_map(&img)) else { continue };
            let dst = normal_map::path_of(src);
            map.save(&dst).expect("write normal map");
            rows.push(format!("| `{}` | {sub}/{}: {} |", rel(&dst), rel(src), normal_map::provenance()));
            println!("{sub}/{}", rel(&dst));
        }
        count += rows.len();
        // The directory's provenance table: the old normal-map rows replaced.
        let sources = dir.join("SOURCES.md");
        let old = std::fs::read_to_string(&sources).unwrap_or_default();
        let mut lines: Vec<String> = old.lines().filter(|l| !l.contains(".normal.png` | ")).map(String::from).collect();
        let head = lines.iter().position(|l| l == "|---|---|").map_or(lines.len(), |i| i + 1);
        let mut body = lines.split_off(head);
        body.extend(rows);
        body.sort_by_key(|l| l.split('`').nth(1).unwrap_or_default().to_lowercase());
        lines.extend(body);
        std::fs::write(&sources, lines.join("\n") + "\n").expect("write SOURCES.md");
    }
    eprintln!("{count} normal maps");
}
