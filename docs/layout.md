# Repository and asset layout

What lives where, how files are named and how the game refers to them.
`crates/tt-assets/tests/tidy.rs` keeps the asset directory to these rules.

## Repository

| Path | What |
|---|---|
| `assets/` | Everything the game loads at run time, nothing else (below). `--assets DIR` / `$TT_ASSETS` point the game at another copy. |
| `gamedata/` | The asset directory imported from a Think Tanks install (`tt-import`, or the game's first-run screen; § Imported game data). Ignored. |
| `crates/` | The Rust workspace (`README.md` lists the crates). Generators of asset files are examples of the crates (§ Generators). |
| `mods/` | The example mods that ship with the game (`docs/modding.md` § Example mods); other folders here are a developer's own mods (ignored). |
| `docs/` | Design notes and specs; `docs/archive/` holds reports of finished stages. |
| `target/` | Build output (ignored). |

The game install is not part of the repository.

## `assets/`

Grouped by kind. A shape's textures sit next to the shape: DTS materials
name their textures without a directory, and the renderer looks for them
beside the `.dts`.

| Directory | Files | What |
|---|---:|---|
| `data/` | 11 | The game's data in RON, one file per domain: `tanks.ron`, `weapons.ron` (projectiles, reticles), `pickups.ron` (power-ups, the power-up weapon roll), `effects.ron` (explosions, particles, emitters), `map_objects.ron` (pads, the volcano, the ball), `ai.ron`, `sounds.ron` (descriptions, game, interface and music sounds), `cameras.ron`, `markers.ron`, `campaign.ron` (the solo levels), `binds.ron` (default key bindings). |
| `missions/` | 55 | One set per mission id: `<id>.ron` (the mission's objects), `<id>.ter` (its terrain), `<id>.png` (its menu picture). `target` (the solo target range) has no picture. |
| `models/tanks/` | 61 | Player tanks (`tank01`..`05`), bots (`bot01`..`03`), the brain and brain jars, their skins, treads (`ltread.ifl`, `rtread.ifl`, `tanktread0N`), the muzzle flash. |
| `models/projectiles/` | 13 | Shot sprites (`*Projectile.png`) and the Mortar rocket (`mortarrocket.dts` + textures). |
| `models/effects/` | 24 | Explosions (`defaultexplosion`, `bounceexplosion`, `AreaExplosion` shapes, `tankexplosion.ifl` frames), smoke, the bounce shot's bump sprite. |
| `models/reticles/` | 12 | The aiming reticles of each weapon. |
| `models/items/` | 108 | Power-ups, health and reload pads (`powerup`, `health`, `recharge`), boost and jump pads, the scrum ball and goal, with their animated textures. |
| `models/volcano/` | 10 | The volcano, its rocks and eruption sprite. |
| `models/props/{green,blue,red}/` | 38 | Scenery of each world: rocks, trees, bones. |
| `models/markers/` | 1 | The editor's marker shape (`octahedron.dts`, a pre-v19 DTS the game never draws; the marker defs name it). |
| `sky/` | 18 | Sky boxes: one list per sky (`sky_day.dml`, `skyblue.dml`, `skyred.dml`, five textures each: four sides, top). |
| `terrain/{green,blue,red}/` | 24 | Terrain textures of each world (the `.ter` files name them). |
| `terrain/detail/` | 1 | The detail texture. |
| `sound/sfx/` | 27 | Sound effects of the world (shots, explosions, collisions, engines, pick-ups, the volcano). |
| `sound/voice/` | 8 | Announcer lines of the power-ups (`voc*`). |
| `sound/ui/` | 9 | Menu and solo-tally sounds (`sounds.ron`, `interface`). |
| `sound/music/` | 7 | Music segments (`sounds.ron`, `music`). |
| `ui/backgrounds/` | 9 | The marquee: the tutorial slides (`Marquee1`..`8`), the backdrop. |
| `ui/icons/` | 8 | Team dots, tally heads, lives and brain icons (menu and HUD). |
| `ui/hud/` | 2 | The radar base and the lag icon. |
| `ui/credits.txt` | 1 | The credits text (`{product}`: the product name). |

Every top-level directory has a `SOURCES.md`: the provenance of each of
its files (§ Provenance). Counts exclude it.

### Naming

* Directories: lowercase `snake_case`, grouped by kind, then by world
  (`green`, `blue`, `red`) where the game has one set per world.
* File names are lowercase, all of them (`tank01.png`, `volcanoexplosion.png`,
  `skyblue.dml`, `missions/tt1_1.ron`), whatever case the game's file has; only
  the `SOURCES.md` tables (and a mod's `README.md`) keep capitals. The importer
  writes the same lowercase names (its mapping maps the game's path, in the
  game's case, to the lowercase asset path). Every lookup of an asset path
  ignores case, so game data imported by an older build, whose files have the
  game's mixed case (`Tank01.png`, `missions/TT1_1.ron`), and mods with
  mixed-case names keep working. The tidy test (`file_names_are_lowercase`)
  keeps `assets/` and the example mods lowercase. Rename a file together with
  the data that names it.
* Images are PNG; transparency is the image's own alpha channel (the game's
  separate `name.alpha.jpg` masks were merged in). Normal maps of the "normal
  maps" option are `name.normal.png` next to `name.png`.
* Sounds are Ogg Vorbis, except five very short clips kept as 16-bit PCM
  WAV (`sfx/dryfire.wav`, `ui/mousedown.wav`, `ui/mouseover.wav`,
  `ui/spcount2.wav`, `ui/ticktoc.wav`, 25–140 ms): as Vorbis they would be
  2–4x larger and decode longer (whole Vorbis blocks), which changes
  `alxGetWaveLen`.
* Text data is RON (`.ron`).

### References

Data and code name an asset by its **asset path**: the path relative to
`assets/`, exactly as on disk, with its extension (`models/tanks/tank01.dts`,
`sound/sfx/explode.ogg`, `missions/tt1_1.ter`, `terrain/detail/detail1.png`).
`tt_assets::resolve` turns one into a file. Terrain files name their materials
by asset path (`terrain/green/fterrain01.png`).

Missions are named by id (`tt1_1`, the file's stem, lowercase like the
file) wherever a name is enough (the menus, the solo campaign,
`--mission`, the console's `map`; `TT1_1` is the same id);
`tt_assets::layout::mission` gives the file, `missions/tt1_1.ron`, which is
what the server's session (`Session::mission_file`), the network messages
and the mission rotation use. Network protocol 10 carries the lowercase
names; a host-screen mission saved by an older build
(`missions/TT1_1.ron`) is read as the lowercase one.

The game's virtual paths (`game/data/shapes/...`, `game/client/ui/...`,
`~/data/...`) and the `.mis` mission names are gone. `--mission` and `map`
accept such paths too. Network protocol 6 carries the new paths (older builds
are refused with a version mismatch).

The user data directory is a separate tree (§ User data). Files written
there shadow asset files of the same path (`tt_engine::fs`).

### Generators

Files the repository generates, and the tools that write them:

| Files | Tool |
|---|---|
| `assets/missions/tt{1,2,3}_5.ter`, `assets/missions/tt{1,2,3}_5.ron` | `cargo run -p tt-assets --example new_maps` |
| `assets/missions/poweruptest.ter` | `cargo run -p tt-assets --example powerup_terrain` |
| `assets/missions/<id>.png` | `cargo run --release -p tt-engine --example mission_picture -- <id> --camera … --out assets/missions/<id>.png` (poses in `missions/SOURCES.md`) |
| `crates/tt-engine/icon/opentt-*.png`, `opentt.ico` (the application icon, from `opentt-source.png`, supplied by the author) | `cargo run --release --manifest-path crates/tt-engine/icon/gen/Cargo.toml` (`icon/README.md`: use, desktop entry, install) |
| `assets/models/projectiles/MortarRocket*` | `cargo run -p tt-assets --example mortar_rocket -- assets/models/projectiles` |
| `assets/models/**/*.normal.png`, `assets/terrain/**/*.normal.png` | `cargo run --release -p tt-assets --example normal_maps -- assets` (also their `SOURCES.md` rows) |
| every file made from the game install (all but OpenTT's own) | (`docs/import.md`): rebuilds them from an install; `crates/tt-import/mapping.ron` (from the `SOURCES.md` tables: `tt-import mapping`) says how |

The data files of `assets/data/` (`tanks.ron`, `weapons.ron`,
`pickups.ron`, `effects.ron`, `map_objects.ron`, `ai.ron`, `sounds.ron`,
`cameras.ron`, `markers.ron`, `campaign.ron`, `binds.ron`) are not
generated: they are the project's own authored data, edited by hand; the
importer copies them (`ours`) and reads no scripts of the install.

The mission files of the game's maps are what `tt-import` makes of the
install's `.mis` files: our class and field names (`tt_import::names`),
asset paths for the file references; an import reproduces them exactly
(`tt-import compare`).

## Imported game data

A copy without the game's data (a fresh checkout of the public
repository, a release build) gets it from the player's own Think Tanks
install, imported into a `gamedata/` directory (`docs/import.md`). The
game looks for the asset directory in this order
(`tt_assets::layout::default_dir`):

1. `--assets DIR`, else `$TT_ASSETS`;
2. the first `gamedata/` that holds the game data (the marker files of
   `tt_assets::testdata::MARKERS`): in the working directory, in the
   source checkout the game was built in, next to the executable, in the
   user data directory (§ User data: `<user data>/gamedata`);
3. `./assets` when it has `missions/`, else the repository's `assets/`.

When the directory found lacks the game data, the game opens its
first-run screen instead of the menus: the player picks the Think Tanks
folder, the game imports it and starts. The game's own import writes to
`tt_assets::layout::import_target`: the source checkout's `gamedata/`
when it runs from its checkout, else `gamedata/` next to the executable
when that directory is writable, else `gamedata/` in the user data
directory (with `--assets DIR` or `$TT_ASSETS`, into that directory). It
writes `<target>.tmp/` and renames it into place only when the import
succeeded. Settings → Game data shows where the data came from and
imports it again.

## User data

Everything the game writes goes to the user data directory
(`tt_assets::layout::user_data_dir`), or `--data DIR`:

| OS | User data directory |
|---|---|
| Linux (and other Unix) | `$XDG_DATA_HOME/opentt`, else `~/.local/share/opentt` |
| Windows | `%APPDATA%\opentt` |
| macOS | `~/Library/Application Support/opentt` |

(`opentt` is `tt_assets::product::DATA_DIR`: the build's `TT_PRODUCT_DATA_DIR`.)
The game's own import goes into `gamedata/` next to the executable only when
that directory is writable and not inside a macOS `.app` bundle (a portable
install); otherwise (`Program Files`, `/Applications`, a system package) into
`<user data>/gamedata` (`tt_assets::layout::import_target`). The directory is
flat; the file names are constants of `tt_shell::save`. A data directory of the
earlier name, `thinktanks-rs`, in the same place is renamed to `opentt` once at
startup when `opentt` does not exist yet; with both, `opentt` is used and the
old one is left untouched (logged; `tt_engine::userdata`, not with `--data`).

| Path | What | Format |
|---|---|---|
| `settings.ron` | Every pref (video, audio, controls, player, host and server settings, recent servers). | `tt_shell::Settings` |
| `players.ron` | The player profiles and their stats. | `tt_shell::save::PlayersFile` |
| `hiscores.ron` | The solo top 10. | `tt_shell::save::HiScoresFile` |
| `progress.ron` | The level select's highest level per player . | `tt_shell::Progress` |
| `banlist.ron` | The server's bans (GUID, IP, end in Unix seconds). | `tt_shell::banlist::BanList` |
| `logs/console.log` | The console log of the last session (rewritten each start). | text |
| `screenshots/` | `doScreenShot` (Ctrl+P): `screenshot_00001.png`, ... (never overwritten: the next free number). | PNG |
| `mods/` | The player's mods: one folder each (`docs/modding.md`). Also looked for in `mods/` of the source checkout (development, the example mods) and where an install keeps the mods it ships (next to the executable, `share/<data dir>/mods`, the `.app`'s `Resources/mods`). Never touched by an import. | the asset tree |
| `gamedata/` | The game's own import, when the executable's directory cannot hold it (above). | the asset tree |
| `legacy/` | Files of an older layout, kept as backups under their old relative path; the game never reads them. Delete at will. | as they were |

### Settings

A setting has a key `section.field`, lower-case `snake_case`:
`video.max_fps`, `graphics.ssao`, `server.max_players`; indexed settings
put the index last (`server.bot.3`, `server.bot_name.3`,
`audio.channel_volume.7`). The settings the game uses are the typed fields
of `tt_shell::Settings`, and their key is exactly their place in
`settings.ron` (`video: (max_fps: 60)` is `video.max_fps`); every other
setting (engine tuning nobody edits) is kept in its `other` map under its
key. The sections are `tt_shell::settings::SECTIONS`: `player`, `video`,
`graphics`, `audio`, `input`, `host`, `net`, `server` (the typed ones),
`opengl`, `shapes`, `terrain`, `lighting`, `shell`, `demo`, `hud`, ... and
`misc`. `game_data` says where the imported game data came from
(Settings → Game data): `game_data.source` (the Think Tanks folder) and
`game_data.imported` (when: `YYYY-MM-DD HH:MM` UTC). `mods.enabled` lists
the enabled mods' folder names in load order (Options → Mods,
`docs/modding.md`).

The same keys are what the dedicated console and `--cmd` take
(`set server.max_players 12`, `get server.*`, `server.max_players = 12`)
and what the game code reads (`Prefs::get("video.detail_level")`, typed
accessors for what the menus edit). The game's run-time state is not
in this store: it is typed state of the server (`tt_gameplay::server::state`:
the game setup, the session, the round, the campaign), not reachable by
name. The player list is `players.*`.

Older saves named settings differently; `tt_shell::legacy` is the only
place that knows those names. It imports the old prefs files and rewrites
the `other` map of a `settings.ron` written by an earlier build when it
is read (the next save writes the new keys); the typed sections did not
change.

### Migration from older layouts

The game and earlier builds of OpenTT kept these files at the game's relative
paths. At every start, before anything reads the directory,
`tt_engine::userdata::migrate` moves an older layout into this one (logged to
the console as `user data: ...`):

| Old path | New path | How |
|---|---|---|
| `game/client/binds.ron` (earlier port, typed) | `binds.ron` | moved as it is |
| `console.log` | `legacy/console.log` | the previous session's log, archived |
| `*.png` at the top | `screenshots/` | moved (a taken name gets the next free number) |

Rules:

* Each step runs only when its new file is missing. When both the old and
  the new file exist, **the new file wins**: it is not touched, and the old
  one is archived to `legacy/<old path>` (`.1`, `.2`, ... appended when that
  name is taken), never merged.
* Nothing is overwritten or deleted. Files are created under free names or
  renamed; a file that cannot be converted stays where it is, the problem is
  logged as a warning, and the next start tries again.
* Afterwards the old directories `game/client`, `game/server`, `game`,
  `common/server`, `common` are removed when empty. Files the game does not
  know keep their directory (logged).
* A missing data directory is a fresh install: nothing is created until
  the game writes its first file. A second run finds nothing to do.
* Paths are matched case-insensitively, as the game's file system does.

`TT_MIGRATE_DIR=<dir> cargo test -p tt-engine --test user_data` runs the
migration on a copy of a real data directory.

## Provenance

All files of `assets/` and `archive/` come from the owner's own licensed copy of
Think Tanks, or are OpenTT's own work made from them. The EULA forbids
redistribution: never push this repository to a public remote.

Textures marked "Real-ESRGAN" were upscaled 4x with realesrgan-ncnn-vulkan
20220424 (Real-ESRGAN release v0.2.5.0, ubuntu zip sha256
`e5aa6eb131234b87c0c51f82b89390f5e3e642b7b70f2b9bbe95b6a285a40c96`), model
realesrgan-x4plus (terrain textures wrap-padded before upscaling so they tile
seamlessly; alpha channels and `.alpha` masks upscaled separately as greyscale,
the masks later merged into the alpha channel of their image). Images with both
sides ≤ 16 px were copied unchanged.

Each `SOURCES.md` table gives, per file, the game path it comes from (or OpenTT's
tool that made it) and how. The data tables of `data/` are the project's own
authored data (`data/SOURCES.md`).
