# The asset importer (`tt-import`)

The game's asset directory (`assets/`, `docs/layout.md`) is mostly the game's
data in OpenTT's layout and formats. A public copy of OpenTT may hold only
OpenTT's own files; a player builds the rest from their own ThinkTanks install:

```sh
cargo run --release -p tt-engine                 # no game data: the first-run screen imports it
cargo run --release -p tt-engine -- --import "/path/to/ThinkTanks"   # the same import, no window
cargo run --release -p tt-import -- --game "/path/to/ThinkTanks" --out gamedata
cargo run --release -p tt-engine                 # finds ./gamedata (or --assets DIR, TT_ASSETS=DIR)
```

The game imports by itself (§ The game's import); `tt-import` is the
same import as a command, for developers.

It reads no scripts of the install: the game's data tables (`data/`: the defs,
the solo campaign, the power-ups, the client sounds, the music, the key
bindings) are the project's own authored files, copied like the other own files.

Crate: `crates/tt-import` (library + binary). Data: `mapping.ron` in the
crate, built into the binary.

## Options

| Option | |
|---|---|
| `--game DIR` | the game install (required) |
| `--out DIR` | the asset directory to write (required; created, files overwritten) |
| `--ours DIR` | where our own files are (default: the copies built into the program from the repository's `assets/`, `tt_import::OURS`) |
| `--normal-maps` / `--no-normal-maps` | make the `*.normal.png` of the "normal maps" option (default on) |
| `--upscale-cmd CMD` | run an external upscaler on the textures the repository has upscaled (below) |
| `--ffmpeg PATH` | ffmpeg for OpenTT's sounds made from the game's (default `ffmpeg` on `PATH`; `""`: not used). Without it the built-in mixer makes them (below) |
| `-v` | list every file written |

Developer commands:

| Command | |
|---|---|
| `tt-import compare --out DIR [--assets assets] [--upscaled]` | compares an imported directory with `assets/` (§ Verification) |
| `tt-import mapping [--check]` | writes `mapping.ron` from `assets/*/SOURCES.md` |

## OpenTT's own files and the derived ones

`mapping.ron` lists every file of `assets/` (the `SOURCES.md` tables excepted)
exactly once, either as made from the game install or as OpenTT's own (`ours`).
A test (`tests/import.rs`) checks both: the mapping is what the `SOURCES.md`
tables say, and it covers the tree.

OpenTT's own files (`ours`, 34; copied as they are):

| Files | What |
|---|---|
| `data/*.ron` | the game's data tables in the engine's RON schema (`tt_assets::data::defs`, `tt_gameplay::client::binds`): tanks, projectiles, power-ups, explosions, bot drivers, cameras, sound descriptions, the solo campaign, the power-up table, the client sounds, the music, the default key bindings; authored and maintained in this repository |
| `missions/tt1_5.{ron,ter}`, `tt2_5`, `tt3_5` | OpenTT's maps (`new_maps` example) |
| `missions/poweruptest.{ron,ter}` | the Power-up Test arena |
| `missions/catalog.ron` | the menus' map list (order, worlds, hidden mission files; `docs/modding.md`) |
| `models/projectiles/mortarrocket.dts`, `mortarrocket.png`, `mortarrockethot.png` | the Mortar's rocket (`mortar_rocket` example) |
| `models/projectiles/scatterprojectile.png` | the Scatter shot's pellet (drawn from scratch) |
| `models/reticles/scatterreticle.png` | the Scatter shot's cone brackets (drawn from scratch) |
| `sound/voice/voc{Area,Bounce,Health,Mortar,Reload,Scatter,Speedy}.ogg` | the Piper announcer lines |
| `ui/credits.txt` | the credits |

Also OpenTT's own, outside `assets/`: the fonts (`crates/tt-gui/assets/fonts`, OFL),
the code, `mapping.ron`, the `SOURCES.md` tables.

Everything else (416 files) is made from the game install.

## How each file is made

| `how` | Files | |
|---|---:|---|
| `Copy` | 88 | `.dts` shapes, `.ogg`/`.wav` sounds: byte for byte |
| `Image` | 148 | `.jpg`/`.png` → PNG (RGB, or RGBA with the alpha channel); with `alpha`: the `.alpha.jpg` mask's luma merged in as the alpha channel (when the sizes match), as the game used to draw them. Decoded with the `image` crate, the game's own decoder. `upscaled: true` (145): the repository's copy is the 4x Real-ESRGAN upscale (§ Upscaling) |
| `Ifl` | 10 | texture animations: each frame name → the frame's asset file name (`.jpg` → `.png`); everything else kept |
| `Dml` | 3 | sky lists: the five box textures by their asset names, the two never drawn cloud layers dropped |
| `Terrain` | 12 | `.ter`: the material names → asset paths (`terrain/green/fterrain01.png`); the heightfield, blend maps and editor scripts kept |
| `Mission` | 16 | `.mis` → `missions/<id>.ron` |
| `Recolour` | 3 | OpenTT's Mortar / Scatter shot textures from imported ones (HSV hue set, saturation raised, value scaled; the Mortar reticle's ring thickened by a max filter run at the 128 px reference width, so it keeps its share of the image at any resolution, and made one colour) |
| `Sound` | 4 | OpenTT's Mortar / Scatter shot sounds from imported ones, with ffmpeg (the filter graphs of `sound/SOURCES.md`; a first pass measures the loudness or peak, the second sets it). Without ffmpeg the built-in mixer (`tt_import::sound`) runs the same graphs: Vorbis decoded with lewton, linear resampling, RBJ biquads for `bass`/`highpass`/`lowpass`, the level by BS.1770 loudness (libebur128's K-weighting) or sample peak; it writes 16-bit mono WAV into the `.ogg` file (the game reads a sound by its content). Same length and level (within about 0.5 dB), not ffmpeg's bytes |
| `NormalMap` | 113 | `tt_assets::normal_map` of the imported texture (512 px; skipped with `--no-normal-maps`) |

References inside converted files (`~/data/shapes/tanks/tank01.dts`,
`./TT1_1.ter`, `game/data/terrains/FTerrain/FTerrain01`, a texture without its
extension, a sound named `.wav` that is an `.ogg`) become asset paths through
the mapping (`Importer::reference`). Every file the importer writes has a
lowercase name (`docs/layout.md` § Naming) whatever the game's case
(`mapping.ron` maps `game/data/shapes/tanks/Tank01.png` to
`models/tanks/tank01.png`; `tt-import mapping` refuses a `SOURCES.md` row that
is not lowercase). Game data imported by an older build has the game's
mixed-case names; the game still reads it (lookups ignore case), and importing
again replaces it with the lowercase names.

An import writes only the asset directory: the mods (`mods/` of the user
data directory, `docs/modding.md`) are never touched.

Order: OpenTT's own files (the data tables among them); the upscaler; the recoloured textures
and the sounds; the normal maps; the pictures.

## Upscaling

The repository's textures are 4x Real-ESRGAN upscales (`docs/layout.md` §
Provenance); they cannot be redistributed or reproduced bit for bit.
`--upscale-cmd` runs a local upscaler on each of the 145 textures marked
`upscaled`:

```sh
tt-import --game ... --out ... \
  --upscale-cmd "realesrgan-ncnn-vulkan -i {in} -o {out} -n realesrgan-x4plus"
```

`{in}` / `{out}` are PNG files; the command is split on whitespace (no
shell). As for the repository's: the alpha channel is upscaled separately
as a greyscale image, terrain textures are wrap-padded (1/8 of a side) so
they tile, the result must be an integer multiple of the input. The
recoloured textures and normal maps are made after the upscale. The game
draws either resolution (the renderer resamples its texture layers).

## Verification

It is ignored unless `TT_ORIGINAL_DIR` is set when the tests are built:

Result: no unexpected differences.

| Class | Files | |
|---|---:|---|
| byte-identical | 129 + 32 | the 32 own files (the 12 data tables among them: copied, so identical by construction), and every converted mission (all 16), `.ter`, `.dts`, `.ifl`, `.dml`, every copied sound |
| pixel-identical | 3 | the team dots (not upscaled: `.jpg` + mask merged) |
| ¼ of the repository's size | 145 + 3 | the upscaled textures (and the recoloured ones made from them) |
| same set, same size | 113 | the normal maps |
| `picture placeholder` (expected) | 4 | OpenTT's own maps' pictures (the game's import renders them) |
| `sound regenerated` (expected) | 4 | the ffmpeg sounds: same length to the sample, loudness within 0.6 dB, not byte-identical (Vorbis encoder) |

The game runs from an imported directory unchanged: `opentt --assets DIR
--headless --play tt1_1 --no-sound --ticks 600` gives the same summary as with
`assets/`, and the golden traces pass (`TT_ASSETS=DIR cargo test --release -p
tt-engine --test golden`): the data are identical.

## The game's import

Without the game data (`tt_assets::testdata::MARKERS` missing from the
asset directory) the game opens a first-run screen instead of failing:
"Locate your ThinkTanks installation", a folder field, **Browse...** (the
system's folder dialog: `rfd`, the XDG desktop portal on Linux with a
zenity fallback; without one the path is typed) and **Import**. It says
that the files are converted on this computer and stay there.

* The folder is checked first (`tt_import::locate::resolve`): the install
  root (with `game/data/missions` and `game/server/scripts`), the folder
  above it, its `game/` folder or one inside it, or a file in it, are all
  taken, and so is a Mac application bundle (`<name>.app/Contents/Resources`);
  then every file the game cannot run without must be there
  (`tt_import::locate::missing_required`, in any accepted form, § Optional
  files). Otherwise the screen says what is missing (no `game` folder;
  `game/server/scripts` missing; "incomplete: 3 of the files the game
  needs are missing (...)"), and the report lists every missing file.
* The import runs on a background thread (`tt_engine::gamedata::ImportJob`)
  through `tt_import::install`: progress (phase, files done of the total,
  the current file) after every file, Cancel between files. It writes
  `<target>.tmp/` and renames it to the target only when it succeeded;
  a failed or cancelled import deletes it and leaves the previous data
  as it was. The target is `tt_assets::layout::import_target`
  (`docs/layout.md` § Imported game data).
* The mission pictures are rendered by the game (`Options::pictures`,
  `tt_engine::mission_picture` on an offscreen GPU device), as the
  repository's; without a GPU the install's pictures are used. (tt-import
  cannot use the game: the game uses tt-import.)
* Without ffmpeg the four sounds come from the built-in mixer (§ How each
  file is made), so the import never needs anything but the install.
* Our own files are built into the program (`tt_import::OURS`, from the
  repository's `assets/` at build time), the data tables (`data/`) among
  them, so the imported directory is complete.
* After the import (or a failed one) the screen shows its report: the
  counts, a scrollable list of what was skipped, not found or warned
  about, **Open log folder** and **Continue** (into the game) or **Back**.
  The full report is written to `<user data>/logs/import.log`
  (`tt_engine::gamedata::IMPORT_LOG`) and appended to `logs/console.log`;
  the new session's console log repeats it. A failure's message names
  the log.
* On success (after Continue) the game boots on the new data without a restart; the
  settings get `game_data.source` (the folder) and `game_data.imported`
  (the date). Settings → Game data shows both and **Re-import...** runs
  the same screen again (the running game is shut down first and booted
  again afterwards, also after Cancel).

`opentt --import DIR [--assets OUT]` runs the same import without a window
(progress on stderr) and exits.

## Optional files and other editions

The editions of ThinkTanks differ, so the import is tolerant:

* **Required** are the files the game cannot run without: models, textures,
  terrains, skies, missions, sound effects. A missing optional file is skipped
  and listed in the report (`Report::skipped`, `Report::missing`), as are files
  made from a skipped one. An image's `.alpha` mask is never required: without
  it the image is imported opaque, or with the alpha it has itself.
* **Other forms**: a file is found ignoring case, and an image under
  another image extension (`Marquee1.png` with its alpha inside, the Mac
  edition's, for `Marquee1.jpg` + `Marquee1.alpha.jpg`), a sound as
  `.ogg` or `.wav` (`tt_import::locate::game_file`).
* **The tutorial** is offered only when all eight slides were imported
  (`tt_menu::art::tutorial_available`); otherwise the player screen has
  no Tutorial button, a new player gets no tutorial prompt and the report
  says "optional, not found: tutorial disabled".
* **Mission files** are read as the game read them
  (`tt_import::convert::mission_text`): UTF-16 (LE/BE, with a byte order mark or
  recognised by its NUL bytes) is decoded, NUL padding removed, and text after
  the editor's `//--- OBJECT WRITE END ---` line ignored when the whole does not
  parse. A mission that still cannot be read is left out (the menus list the
  missions there are) and the import goes on.

## Open questions

* Tests of the other crates list values of the data tables (e.g.
  `tests/client_data.rs` the client sound declarations); they test the
  project's own data now.
