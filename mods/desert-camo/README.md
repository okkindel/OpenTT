# Desert Camo (example)

An example of a **cosmetic** mod: it repaints the tanks in a desert
camouflage of sand, dark earth and olive drab, fitted to each tank: worn
edges, dusty undersides, panel seams with rivets, rubber road wheels. It ships with the game and is listed in
Options → Mods, but it is not enabled. Tick its box and restart to try it.

## What is in the folder

```text
desert-camo/
  mod.ron                    name, version, author, description
  README.md                  this file (not a game file)
  SOURCES.md                 where the files come from (not a game file)
  models/tanks/tank01.png    replaces the game's models/tanks/tank01.png
  models/tanks/tank02.png    ... and so on, one texture per tank
  models/tanks/tank03.png
  models/tanks/tank04.png
  models/tanks/tank01.normal.png   the relief of tank01.png (seams, rivets,
  ...                              scratches) for the "normal maps" option
  models/tanks/tank04.normal.png
```

A texture's normal map is the file next to it named `NAME.normal.png`. A
mod that replaces a texture but ships no normal map shows it flat (the
game's normal map is not used for a mod's texture).

Every file below the mod's folder replaces the game's file at the same path
(upper and lower case do not matter). Files directly in the folder are not
game files.

## Make your own from it

1. Options → Mods → Open mods folder opens your own mods folder.
2. Copy this whole `desert-camo` folder there and give the copy a new
   name, such as `my-camo`. Do not edit the folder that ships with the
   game: an update replaces it. Your copy is yours.
3. In the copy's `mod.ron`, change `name` and `author`, and delete the line
   `example: true`.
4. Paint over the PNGs, or replace them with your own pictures. Any size
   works; keep the file names. Delete the `.normal.png` files if your
   paint no longer matches their seams and rivets.
5. In the game: Options → Mods, enable your mod, Restart now.

A copy that keeps the folder name `desert-camo` takes the place of the
shipped example. The modding guide (`docs/modding.md`) lists every folder
and data file a mod can replace.
