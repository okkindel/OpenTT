# Mods

A mod is a folder of files laid out like the game data. When it is
enabled, each of its files replaces the game's file at the same path:
a texture, a sound, a model, a sky, a menu picture, a data file, a map.
A mod can also add files the game does not have, such as new maps.

Code: `tt_assets::mods` (reading mods, the overlay), `tt_engine::mods`
(enabled mods, the network rules), `tt_shell::catalog` (the map list).

## Where mods go

Each mod is one folder in `mods/` of the user data directory:

| OS | Mods folder |
|---|---|
| Linux (and other Unix) | `$XDG_DATA_HOME/opentt/mods`, else `~/.local/share/opentt/mods` |
| Windows | `%APPDATA%\opentt\mods` |
| macOS | `~/Library/Application Support/opentt/mods` |

(`opentt` is the build's data directory name,
`tt_assets::product::DATA_DIR`; `--data DIR` moves the whole user data
directory, mods included.) The game also looks in `mods/` of the source
checkout it was built in (development) and where an install keeps the mods
it ships (the example mods below):

| Install | Mods that ship with it |
|---|---|
| `.tar.gz`, `.zip` | `mods/` next to the executable |
| `.deb` | `/usr/share/<data dir>/mods` (`<prefix>/share/<data dir>/mods` for a binary in `<prefix>/bin`) |
| AppImage | `usr/share/<data dir>/mods` inside the image |
| macOS `.app` | `Contents/Resources/mods` of the bundle |

(`tt_assets::layout::installed_mod_dirs`.) A mod found in more than one of
these places is taken from the first, so a copy in your own mods folder
replaces a shipped mod of the same folder name. Options → Mods → Open mods folder opens the user's folder (and
creates it).

Importing the game data again (Options → Game data → Re-import) never
touches the mods: they live outside the imported data.

## Folder layout

```text
mods/
  my-mod/                    the mod's id: its folder name
    mod.ron                  optional: name, version, author, description
    models/tanks/tank01.png  replaces the Light tank's texture
    sound/...
    missions/my_map.ron      a new map
    missions/my_map.ter
    missions/my_map.png
```

The folders below the mod mirror the game data: `models/`, `missions/`,
`sound/`, `ui/`, `sky/`, `terrain/`, `data/` (`docs/layout.md` describes
each). Name the files in lowercase, as the game data's own are
(`docs/layout.md` § Naming; the example mods do). Paths are matched
without regard to case, like every asset path, so a mod with mixed-case
names (`missions/MyMap.ron`) works as well.
Files directly in the mod's folder (`mod.ron`, a readme, a preview
picture) are not game files.

### `mod.ron`

Every field is optional:

```ron
(
  name: "HD Tank Skins",
  version: "1.2",
  author: "Krevitz",
  description: "Sharper camouflage for the three tanks.",
  // affects_gameplay: true,
)
```

`example: true` marks the example mods that ship with the game (an
"Example" tag in Options → Mods); a mod of your own leaves it out.

`name` defaults to the folder name. `affects_gameplay` is detected: a mod
with files under `data/` or map files (`missions/*.ron`, `missions/*.ter`)
changes the game; textures, sounds, models, skies, menu art and map
pictures are cosmetic. Set `affects_gameplay: true` to mark a mod as
changing the game when the detection cannot see it; `false` cannot hide
data or map files.

## Example mods

Two example mods ship with the game. Options → Mods lists them with an
"Example" tag; they are not enabled until you tick their box. Both are made
of plain files only, and each has a `README.md` that walks through its
folder:

| Folder | Mod | Shows |
|---|---|---|
| `desert-camo/` | Desert Camo (example) | a cosmetic mod: four tank textures (`models/tanks/tank01.png` ... `tank04.png`) and their normal maps (`tank01.normal.png` ...) replace the game's: a sand, dark earth and olive camouflage fitted to each tank model, painted for this project |
| `carpet-bombing/` | Carpet Bombing (example) | a gameplay mod made of data only: its `data/weapons.ron` turns the Mortar into a carpet bomb, three shots of nine shells a pull (the README lists each changed field and why) |

They live in the repository's `mods/` and in each package where the game
looks for an install's mods (§ Where mods go). To start a mod of your own
from one:

1. Options → Mods → Open mods folder.
2. Copy the example's folder there (from the game's install, or from
   `mods/` of the source) under a new folder name.
3. In the copy's `mod.ron`, set `name` and `author` and delete
   `example: true`.
4. Change its files, then enable it in Options → Mods and restart.

Do not edit the shipped folders: an update replaces them. A copy that keeps
an example's folder name replaces that example.

Carpet Bombing's `data/weapons.ron` is a whole copy of the game's (a data
file replaces the whole file); a test (`crates/tt-assets/tests/example_mods.rs`)
fails when it differs from the game's `assets/data/weapons.ron` in more than
the mod's own changes, and `TT_BLESS_EXAMPLE_MODS=1 cargo test -p tt-assets
--test example_mods` rewrites it after the game's file changed.

## Enabling mods and the load order

Options → Mods lists the installed mods. The checkbox enables a mod; the
enabled ones are listed first, in load order. When two enabled mods have
the same file, the one lower in the list wins (Move up / Move down change
the order; a mod enabled last goes to the bottom). The choice is saved in
`settings.ron` as `mods.enabled` (the folder names in load order) and
applies when the game starts: Restart now restarts the game.

The overlay applies to everything the game loads (textures, models,
sounds, terrains, skies, maps, data files, menu art): it is installed in
the engine's file index (`tt_engine::fs::GameFs::with_mods`) and in the
lookups every loader uses (`tt_assets::find_ci`, `tt_assets::resolve`,
`tt_assets::data::load`). A file next to a mod's file that the mod does
not have (a texture next to a mod's model) comes from the next mod down
or the game.

## Replacing a texture, a sound or a model

Put the replacement at the same path, with the same file name and
extension:

- Textures are PNG (`models/tanks/tank01.png`). Any size works; keep the
  alpha channel where the game's texture has one. A replaced texture's
  normal map (`<name>.normal.png`) is not used unless the mod has one too.
- Sounds are Ogg Vorbis or WAV under the game's name
  (`sound/weapons/...ogg`).
- Models are DTS shapes (`models/...dts`). A model keeps its textures by
  name: they are looked up next to it, in the mod first.

Look up the exact paths in the game data folder (`docs/layout.md` §
Imported game data says where it is) or in the `SOURCES.md` tables of the
repository's `assets/`.

## Data files

The data files under `data/` are RON, one file per domain:

| File | What |
|---|---|
| `tanks.ron` | the tanks (speed, armour, turret, default cartridge, driver), their sounds and effects, shared tank physics |
| `weapons.ron` | the cartridges (projectiles: damage, speed, rounds, reload, explosions) and their aiming reticles |
| `pickups.ron` | the power-ups and the power-up weapon roll (which cartridges a weapon pick-up gives, and how often) |
| `effects.ron` | explosions, particles and particle emitters |
| `map_objects.ron` | boost and jump pads, the volcano, the ball |
| `ai.ron` | the bot drivers |
| `sounds.ron` | sound descriptions, the game's sounds, the interface sounds, the music |
| `cameras.ron` | the observer camera and the tanks' chase camera |
| `markers.ron` | the mission editor's markers |
| `campaign.ron` | the solo campaign's levels |
| `binds.ron` | the default key bindings |

Each file is a list of definitions ("defs"): `ClassDef(name: "...", parent:
"...", field: value, ...)`. A def with a `parent` starts as a copy of the
parent's fields. A def names others by name (a tank its cartridge and
driver, a cartridge its explosions); a name that is not declared is
reported in the log (`data: ...`) and left unset. A mod's data file
replaces the whole file: copy the game's file into the mod and change what
you need. Definitions are not merged one by one (a later version may add
that). Every data file change makes the mod a gameplay mod.

### Fields

The fields a modder changes most, by class (times in milliseconds where the
name ends in `_ms`, distances in metres, angles in degrees; a field left
out takes the default in brackets). Every class and field is in
`crates/tt-assets/src/data/defs.rs` (`tt_assets::data::defs`).

| Class | Field | What |
|---|---|---|
| `ProjectileDef` (`weapons.ron`) | `damage` | damage of a hit (1) |
| | `velocity`, `inherit_velocity` | launch speed (m/s, 30); the share of the tank's speed it keeps (1) |
| | `blast_radius` | radius of the blast that hurts tanks near the hit (0: direct hits only) |
| | `bounces`, `bounce_factor` | bounces before it explodes (0); speed kept at a bounce (1) |
| | `gravity_scale` | how hard it falls (1) |
| | `rounds` | shots the cartridge holds (a pick-up's, or the default cartridge's ammunition) |
| | `volley_size`, `reload_ms`, `volley_delay_ms` | shots a pull of the trigger fires (a volley; 0), the time between them (1000), and from the last shot until the next pull (0) |
| | `keep_previous_weapon`, `can_be_kept` | picking it up keeps the cartridge in use for later (0); this cartridge can be kept that way (1) |
| | `sprite_scale` | size of the projectile's image and of its collision sphere (1) |
| | `max_life_ms` | flight time limit (0: none) |
| | `pellet_count`, `spread_angle`, `spread_ring`, `spread_inner`, `spread_jitter`, `spread_exponent` | several shells a shot: how many (1), the fan or cone width, a cone instead of a fan, the inner ring, the turn of the ring each shot, the spacing |
| | `launch_pitch_min/max`, `launch_range_min/max` | a high-angle launch (the Mortar): elevation and reach with the barrel highest / lowest |
| | `tank_explosion`, `own_tank_explosion`, `bounce_explosion`, `other_explosion` | the `ExplosionDef`s of its hits |
| | `reticle`, `emitter`, `fire_sound`, `resource`, `display_shape` | its aiming reticle, smoke trail, fire sound, image, and a model drawn instead of the image |
| | `sync_ms`, `sync_lead_ms` | kept from the game's data, unused |
| `TankDef` (`tanks.ron`) | `max_speed`, `accel_rate`, `decel_rate`, `coast_decel_rate`, `turn_rate` | driving |
| | `turret_yaw_range`, `turret_pitch_range`, `turret_pitch_center` | how far the turret turns |
| | `damage_scale`, `armor_scale`, `fire_scale` | damage dealt, damage taken, rate of fire |
| | `default_projectile`, `ai`, `fx`, `misc`, `camera` | its cartridge, bot driver, effects, shared physics and chase camera defs |
| | `shape`, `brain_shape` | the tank's and its brain jar's models |
| `PowerUpDef` (`pickups.ron`) | `kind` | `weapon`, `health`, `ammo` (the reload arch), `goal`, `brain` |
| | `starts_on`, `min_off_ms`, `max_off_ms` | on at the start (1); after a pick-up it comes back after a time between the two |
| | `shape`, `sound`, `sound_off`, `activate_sequence` | its model, sounds, and the model's animation played when used |
| `BoostDef` (`map_objects.ron`) | `kind`, `num_states`, `velocity1..4`, `min_duration1..4` | a pad's kick (`additive`, `override`, `overridedir`) and its states |
| `FlagDef` (`map_objects.ron`) | `bounce_terrain`, `bounce_static`, `unstuck_time`, `hard_collision_sound`, `collision_interval_ms`, ... | the scrum ball |
| `ExplosionDef` (`effects.ron`) | `resource`, `start_size`, `end_size`, `duration_ms`, `sound` | an image (growing from `start_size` to `end_size` over `duration_ms`) or a model, and its sound |
| `SoundDef` (`sounds.ron`) | `file`, `description` | a sound file and its `SoundDescDef` |
| `SoundDescDef` (`sounds.ron`) | `channel`, `volume`, `is_looping`, `is_3d`, `reference_distance`, `max_distance` | how it plays; `channel` is the volume slider it follows: `sim` (effects), `gui`, `message`, `music` |
| `ReticleDef` (`weapons.ron`) | `image`, `size`, `tick_count`, `start_time`, `time_step`, `show_spot`, `spot_image`, `ring_image` | the aiming dots along the flight path |

## Adding a map

A map is three files in `missions/`, named by the map's id:

| File | What |
|---|---|
| `<id>.ron` | the map's objects (required) |
| `<id>.ter` | its terrain |
| `<id>.png` | the menu picture (optional; without one the menu shows the map's name) |

Every `missions/*.ron` of the game data and of the enabled mods is a map
of the map list (Custom game, Host, Quick Play), except the ones the map
list hides (below).

### The mission file

A mission file is RON: optional `defs` the map declares, then the
object tree, normally one `Group` named `MissionGroup`. The quickest
start is a copy of a map of the game data (`missions/tt1_5.ron`, a map
made for this game) under the new id. The parts that matter:

```ron
(
  objects: [
    Group(
      name: "MissionGroup",
      music_track: "Lush",                  // Lush, Spooky, Frantic or Lunar
      children: [
        Info(
          name: "MissionInfo",
          display_name: "My Map",           // the name in the menus
          desc0: "One line about the map.",
          extra: [("world", "2")],          // 1 green, 2 blue, 3 red, 4 the Moon, 0 special
          gravity: 0.45,                    // optional: the share of the normal gravity (1)
        ),
        MissionArea(name: "MissionArea", area: (-216, -216, 432, 432), flight_ceiling: 200, flight_ceiling_range: 0, floor: 90),
        Sky(name: "Sky", material_list: "sky/sky_day.dml", ...),
        Sun(name: "Sun", direction: (0.57735, 0.57735, -0.57735), ...),
        Terrain(
          name: "Terrain",
          position: (-1024, -1024, 0),
          terrain_file: "missions/my_map.ter",
          detail_texture: "terrain/detail/detail1.png",
          square_size: 8,
          block_shift: 7,
          ...
        ),
        Group(name: "PlayerDropPoints", children: [
          Group(name: "GreenPlayerDropPoints", children: [SpawnPoint(position: (x, y, z), ...), ...]),
          Group(name: "BluePlayerDropPoints", children: [...]),
        ]),
        PowerUp(name: "pup1", position: (x, y, z), def: "TestPowerup", ...),  // or "Heal", "Reload"
        Boost(name: "Jump1", ...),           // boost and jump pads
        Prop(shape: "models/props/green/rockgreen01.dts", position: (x, y, z), ...),
      ],
    ),
  ],
)
```

Asset references are paths in the game data (`models/...`,
`sky/...`, `missions/<id>.ter`); a path starting with `./` is relative to
the mission file's folder. Copy the objects you need from a map of the
game data: each class's fields are those of `tt_assets::data::mission`.

### The map's world and the map list

The map list comes from `missions/catalog.ron` (in the game data) and the
mission files found:

```ron
(
  missions: [
    (id: "tt1_1", world: 1),
    ...
  ],
  hidden: ["target"],                       // mission files that are no maps of the list
)
```

Maps the catalog lists come first, in its order; the other maps follow by
world (special ones last) and id. A map's world is its catalog entry's,
else its `MissionInfo` field `world`, else 0. Worlds 1 to 4 decide the
colour the menus show; worlds 1 to 3 are the maps Quick Play draws from
(the Moon's, world 4, are in the lists and the `MOON` rotation); world 0 maps
(test arenas) are only in the lists. A mod may have its own
`missions/catalog.ron`: catalogs are merged (an entry of the same id
changes the fields it sets — `world`, `name`, `music` — new entries are
added after the game's, `hidden` lists add up).

### The terrain

The terrain file is a heightfield (128 × 128 squares of 8 m, a 1024 m
block that repeats) with up to eight materials (`terrain/...png`) and
their blend maps. Write one with `tt_assets::terrain::Terrain` (`new`,
`to_bytes`) into your mod's `missions/`. For the objects, start from the
mission file of a similar map (`missions/tt1_5.ron` and so on) and move
the spawns, power-ups and scenery to the new terrain.

### The picture

`missions/<id>.png`, about 3:1 (the game's are 1528 × 528). The engine can
render one from a camera pose (position, yaw and pitch in degrees, the
horizontal field of view; `tt_engine::mission_picture`).

## Multiplayer

Network games need every player to run the same game:

- A server announces its gameplay mods (name, version and a hash of the
  mod's gameplay files) and its map in the LAN and internet server lists,
  which show such servers as `[modded]` and name the mods.
- On joining, the client compares the server's gameplay mods with its own
  enabled ones before anything is loaded. A difference ends the join with a
  message naming each mod: missing, installed but not enabled, a different
  version, or enabled here but not on the server. A map the client does not
  have is refused by name.
- Cosmetic mods may differ between players: each player sees their own
  textures, sounds and models.
- The checks of the game data stay as a safety net: the def hash
  ("Game data differs from the server's") and the map's hash (its mission
  file, terrain and the models placed in it). A cosmetic mod that replaces
  a model placed in a map changes that map's hash, so it counts there.
