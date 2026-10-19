# Carpet Bombing (example)

An example of a **gameplay** mod made of data only: no code, just one data
file. With the Mortar power-up, one pull of the trigger fires three quick
volleys of nine shells each, and 27 blasts carpet a patch of ground about
35 × 20 m close up and 60 × 25 m far out. The mod ships with the game and
is listed in Options → Mods, but it is not enabled. Tick its box and
restart to try it.

## What is in the folder

```text
carpet-bombing/
  mod.ron            name, version, author, description
  README.md          this file (not a game file)
  SOURCES.md         where the files come from (not a game file)
  data/weapons.ron   replaces the game's data/weapons.ron
```

A data file of a mod replaces the **whole** file, so `data/weapons.ron` is a
full copy of the game's. Only the `MortarProjectile` block differs (the
note at the top of the file lists the changes too):

| Field | Game | Here | Why |
|---|---|---|---|
| `pellet_count` | (1) | 9 | nine shells a shot |
| `spread_ring` | (0) | 1 | the shells fly in a cone (one in the middle, eight on a ring) instead of a flat fan; on the Mortar's high arc the cone spreads them sideways and also short and long, so they cover a patch, not a line |
| `spread_angle` | (0) | 18 | the cone is 18° wide |
| `spread_inner` | (1) | 0.5 | every other ring shell flies at half the angle: the patch fills in |
| `spread_jitter` | (0) | 1 | the ring turns a little with each shot, so each shot lands between the shells of the one before |
| `volley_size` | 1 | 3 | three shots a pull of the trigger (a volley) |
| `reload_ms` | 1800 | 250 | 0.25 s between the shots of a volley |
| `volley_delay_ms` | 1800 | 1500 | 1.5 s before the next pull |
| `rounds` | 6 | 12 | 12 shots a pick-up: four pulls |
| `damage` | 8 | 3 | 27 shells a pull: each blast hurts less... |
| `blast_radius` | 11 | 8 | ...and is smaller, so the carpet is not certain death |

Values in brackets are the defaults the game uses when a field is not
written. Each shot of a volley uses one of the `rounds`, however many
shells it fires.

Because it changes a data file, the mod changes the game: in a network game
the server and every player need the same mod enabled, or joining is
refused with a message naming the mod. Bots use the modded Mortar too.

## Make your own from it

1. Options → Mods → Open mods folder opens your own mods folder.
2. Copy this whole `carpet-bombing` folder there and give the copy a new
   name, such as `my-weapons`. Do not edit the folder that ships with the
   game: an update replaces it. Your copy is yours.
3. In the copy's `mod.ron`, change `name` and `author`, and delete the line
   `example: true`.
4. Change the values you want in `data/weapons.ron`. After a game update,
   start again from the game's current `data/weapons.ron` (in the game data
   folder): a mod's data file replaces the game's completely, so an old copy
   would undo the update's changes to the weapons.
5. In the game: Options → Mods, enable your mod, Restart now.

The modding guide (`docs/modding.md`) describes every data file.
