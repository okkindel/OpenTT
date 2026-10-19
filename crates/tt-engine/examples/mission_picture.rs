//! Offscreen mission pictures (the menu's `assets/missions/<id>.png`)
//! rendered by the game's renderer with the default graphics.
//!
//! ```text
//! cargo run --release -p tt-engine --example mission_picture -- \
//!     [--assets DIR] <mission> [--time SECONDS] [--no-triggers] [--ssaa N]
//!     ( --camera X Y Z YAW_DEG PITCH_DEG HFOV_DEG --out FILE.png [--size WxH]
//!     | --poses FILE | --serve )
//! ```
//! `<mission>` is a name (`tt1_1`, looked up in `<assets>/missions/`) or a
//! `.ron` path. The scene is built like the game's: the mission's
//! objects (the conditional ones as with no game type, like tt-viewer) with
//! the server defs (`data/*.ron`) and the mission's own;
//! power-ups, reload arches and boost/jump pads are drawn as the game draws
//! them (animated dynamic shapes at `--time`, default 1 s: lamps dark between flashes, all switched
//! on, with their dynamic lights). Renderer settings are the game's
//! defaults: `RenderSettings` with the extras of
//! `tt_shell::prefs::GRAPHICS_DEFAULTS` (glass, HDR, bloom, normal maps,
//! SSAO, AO, soft shadows), anisotropy 16, default detail. The view is the
//! game's: `tt_engine::gfx::render_game_view` with the camera's horizontal
//! FOV turned into the vertical one for the aspect (`vertical_fov`).
//!
//! Camera: position in mission units, yaw in degrees (0 = +Y/north,
//! 90 = +X/east), pitch in degrees (negative looks down), horizontal FOV in
//! degrees. `--ssaa N` renders N× larger and box-filters down (default 2).
//!
//! * `--camera … --out f.png --size 1528x528`: one picture;
//! * `--poses FILE`: one picture per line `OUT.png X Y Z YAW PITCH HFOV [WxH]`
//!   (default size `--size`, 1528x528), all with one GPU/renderer set-up;
//! * `--heights FILE`: also writes the terrain heights on an 8-unit grid
//!   over -1024..1024 (257 rows of y, 257 columns of x; for camera searches);
//! * `--serve`: the same lines read from stdin; after each picture
//!   `ok OUT.png` is printed (for camera searches driven by a script).
//!
//! Example (tt1_1's menu picture; all poses are in assets/missions/SOURCES.md):
//! ```text
//! cargo run --release -p tt-engine --example mission_picture -- tt1_1 \
//!     --camera -28.370 68.900 140.137 97.411 -1.937 49.345 --out assets/missions/tt1_1.png
//! ```

use std::io::{BufRead, Write};
use std::path::PathBuf;
use tt_engine::mission_picture::Pictures;

struct Args {
    root: PathBuf,
    mission: String,
    time: f32,
    triggers: bool,
    ssaa: u32,
    size: (u32, u32),
    camera: Option<[f32; 6]>,
    out: Option<PathBuf>,
    poses: Option<PathBuf>,
    serve: bool,
    heights: Option<PathBuf>,
}

fn parse_size(v: &str) -> Result<(u32, u32), String> {
    let (w, h) = v.split_once('x').ok_or("size needs WxH")?;
    Ok((w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?))
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut a = Args { root: tt_assets::layout::default_dir(), mission: String::new(), time: 1.0, triggers: true, ssaa: 2, size: (1528, 528), camera: None, out: None, poses: None, serve: false, heights: None };
    while let Some(s) = it.next() {
        match s.as_str() {
            "--assets" => a.root = it.next().ok_or("--assets needs a value")?.into(),
            "--time" => a.time = it.next().and_then(|v| v.parse().ok()).ok_or("--time needs seconds")?,
            "--no-triggers" => a.triggers = false,
            "--ssaa" => a.ssaa = it.next().and_then(|v| v.parse().ok()).ok_or("--ssaa needs a factor")?,
            "--size" => a.size = parse_size(&it.next().ok_or("--size needs WxH")?)?,
            "--out" => a.out = Some(it.next().ok_or("--out needs a path")?.into()),
            "--poses" => a.poses = Some(it.next().ok_or("--poses needs a file")?.into()),
            "--serve" => a.serve = true,
            "--heights" => a.heights = Some(it.next().ok_or("--heights needs a file")?.into()),
            "--camera" => {
                let mut c = [0.0; 6];
                for v in &mut c {
                    *v = it.next().and_then(|s| s.parse().ok()).ok_or("--camera needs X Y Z YAW PITCH HFOV")?;
                }
                a.camera = Some(c);
            }
            _ if a.mission.is_empty() => a.mission = s,
            _ => return Err(format!("unexpected argument {s}")),
        }
    }
    if a.mission.is_empty() {
        return Err("usage: mission_picture <mission> (--camera X Y Z YAW PITCH HFOV --out F | --poses FILE | --serve)".into());
    }
    a.ssaa = a.ssaa.clamp(1, 4);
    Ok(a)
}

/// `OUT.png X Y Z YAW PITCH HFOV [WxH]`.
/// A parsed pose line: output file, X Y Z YAW PITCH HFOV, size.
type Pose = (PathBuf, [f32; 6], (u32, u32));

fn parse_pose(line: &str, default: (u32, u32)) -> Result<Option<Pose>, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 7 {
        return Err(format!("bad pose line: {line}"));
    }
    let mut c = [0.0f32; 6];
    for (i, v) in c.iter_mut().enumerate() {
        *v = parts[i + 1].parse().map_err(|_| format!("bad number in: {line}"))?;
    }
    let size = match parts.get(7) {
        Some(s) => parse_size(s)?,
        None => default,
    };
    Ok(Some((PathBuf::from(parts[0]), c, size)))
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let mut pics = Pictures::new(&args.root, &args.mission, args.time, args.triggers, args.ssaa)?;
    if let Some(h) = &args.heights {
        // Terrain heights on an 8-unit grid over -1024..1024 (rows: y, columns: x).
        let mut text = String::new();
        for j in 0..=256 {
            let row: Vec<String> = (0..=256).map(|i| format!("{:.2}", pics.scene.terrain_height(i as f32 * 8.0 - 1024.0, j as f32 * 8.0 - 1024.0))).collect();
            text += &row.join(" ");
            text.push('\n');
        }
        std::fs::write(h, text).map_err(|e| e.to_string())?;
    }
    if let Some(cam) = args.camera {
        let out = args.out.clone().ok_or("--camera needs --out")?;
        pics.render(&out, cam, args.size)?;
        println!("saved {}", out.display());
    }
    if let Some(p) = &args.poses {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        for line in text.lines() {
            if let Some((out, cam, size)) = parse_pose(line, args.size)? {
                pics.render(&out, cam, size)?;
            }
        }
    }
    if args.serve {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        for line in stdin.lock().lines() {
            let line = line.map_err(|e| e.to_string())?;
            match parse_pose(&line, args.size) {
                Ok(Some((out, cam, size))) => match pics.render(&out, cam, size) {
                    Ok(()) => writeln!(stdout, "ok {}", out.display()),
                    Err(e) => writeln!(stdout, "err {e}"),
                },
                Ok(None) => writeln!(stdout, "skip"),
                Err(e) => writeln!(stdout, "err {e}"),
            }
            .map_err(|e| e.to_string())?;
            stdout.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
