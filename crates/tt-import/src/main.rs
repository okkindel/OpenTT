//! `tt-import`: see the crate documentation ([`tt_import`]) and `docs/import.md`.

use std::path::PathBuf;
use std::process::ExitCode;
use tt_import::mapping::{self, Mapping};
use tt_import::{verify, Options};

const USAGE: &str = "\
usage: tt-import --game GAME_DIR --out ASSETS_DIR [--ours DIR] [--no-normal-maps]
                 [--ffmpeg PATH] [-v]
       tt-import compare --out IMPORTED_DIR [--assets DIR]
       tt-import mapping [--assets DIR] [--check]

import     builds the asset directory ASSETS_DIR from the game install
           (the directory with game/) and the built-in files (built in from
           the repository's assets/; --ours DIR: from DIR)
--normal-maps / --no-normal-maps
           generate the normal maps (*.normal.png; on by default)
           The mission pictures are the game's (the new maps get a
           shaded-terrain placeholder); the game's own import
           (the game executable's --import GAME_DIR) renders them
--ffmpeg   ffmpeg for the sounds made from the game's (default ffmpeg;
           without it the built-in mixer makes them)
compare    compares an imported directory with assets/ (docs/import.md)
mapping    writes crates/tt-import/mapping.ron from assets/*/SOURCES.md
           (--check: only compares)";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tt-import: {e}");
            ExitCode::FAILURE
        }
    }
}

fn crate_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name)
}

fn run(args: Vec<String>) -> Result<(), String> {
    let (cmd, rest) = match args.first().map(String::as_str) {
        Some(c @ ("compare" | "mapping")) => (c.to_string(), args[1..].to_vec()),
        _ => ("import".to_string(), args),
    };
    let mut game: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut assets = tt_import::repo_assets();
    let mut opts = Options::new("", "");
    let mut check = false;
    let mut it = rest.into_iter();
    while let Some(a) = it.next() {
        let mut val = |what: &str| it.next().ok_or(format!("{a} needs {what}"));
        match a.as_str() {
            "--game" => game = Some(val("a directory")?.into()),
            "--out" => out = Some(val("a directory")?.into()),
            "--ours" => opts.ours = Some(val("a directory")?.into()),
            "--assets" => assets = val("a directory")?.into(),
            "--normal-maps" => opts.normal_maps = true,
            "--no-normal-maps" => opts.normal_maps = false,
            "--pictures" => return Err("--pictures: the mission pictures are rendered by the game: run the game with --import GAME_DIR (docs/import.md)".into()),
            "--ffmpeg" => opts.ffmpeg = val("a path")?,
            "--check" => check = true,
            "-v" | "--verbose" => opts.verbose = true,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    match cmd.as_str() {
        "import" => {
            opts.game = game.ok_or(format!("--game is required\n{USAGE}"))?;
            opts.out = out.ok_or(format!("--out is required\n{USAGE}"))?;
            let report = tt_import::import(&opts)?;
            for w in &report.warnings {
                eprintln!("warning: {w}");
            }
            println!("{} files written to {}", report.written, opts.out.display());
            Ok(())
        }
        "compare" => {
            let out = out.ok_or("--out is required")?;
            let m = verify::compare(&out, &assets, &Mapping::builtin());
            for l in verify::summary(&m) {
                println!("{l}");
            }
            let bad = m.iter().filter(|x| !verify::EXPECTED.contains(&x.class)).count();
            if bad == 0 {
                println!("ok: no unexpected differences");
                Ok(())
            } else {
                Err(format!("{bad} unexpected differences"))
            }
        }
        "mapping" => {
            let m = mapping::from_sources(&assets)?;
            let text = m.to_ron()?;
            let path = crate_file("mapping.ron");
            if check {
                let old = std::fs::read_to_string(&path).unwrap_or_default();
                return if old == text { Ok(()) } else { Err(format!("{} is not what assets/*/SOURCES.md describe", path.display())) };
            }
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("{}: {} files from the game, {} built in", path.display(), m.files.len(), m.ours.len());
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}
