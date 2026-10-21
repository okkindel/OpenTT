# Provenance: missions/: mission files, terrains and menu pictures

Where each file of this directory comes from (paths relative to it).
Licence, the upscaling and the layout: `docs/layout.md` (§ Provenance).

This table lists OpenTT's own files of this directory; the importer builds the rest of the asset directory from your game install (`docs/import.md`).

| file | source: how |
|---|---|
| `catalog.ron` | OpenTT's own: the menus' map list (order, worlds, the mission files that are no maps of the list; docs/modding.md) |
| `poweruptest.ron` | OpenTT's own "Power-up Test" mission (written as a mission file and converted) |
| `poweruptest.ter` | OpenTT's own: generated (a flat square floor in a square bowl; one texture, terrain/dev/devgrid.png) |
| `tt1_5.ron` | OpenTT's own map "The Neverending Stream": generated, written as RON; the game's textures, skies and shapes |
| `tt1_5.ter` | OpenTT's own map "The Neverending Stream": generated |
| `tt1_5_macro.png` | OpenTT's own map "The Neverending Stream": its terrain's macro colour map (`macro_texture`), generated |
| `tt2_5.ron` | OpenTT's own map "Night of the Living Brains": generated, written as RON; the game's textures, skies and shapes, our Halloween props (models/props/halloween/, CC0), moon and cloud sprites |
| `tt2_5.ter` | OpenTT's own map "Night of the Living Brains": generated |
| `tt2_5_macro.png` | OpenTT's own map "Night of the Living Brains": its terrain's macro colour map (`macro_texture`), generated, then made a 256-colour palette PNG (no visible change in game; 0.9 MB -> 0.13 MB) |
| `tt3_5.ron` | OpenTT's own map "The Dark Cortex": generated, written as RON; the game's textures, skies and shapes, our dark tower and its crashed starfighter (models/props/red/wreck_fighter, CC0) |
| `tt3_5.ter` | OpenTT's own map "The Dark Cortex": generated |
| `tt3_5_macro.png` | OpenTT's own map "The Dark Cortex": its terrain's macro colour map (`macro_texture`), generated, 512x512 (2 m per texel) |
| `tt4_1.ron` | OpenTT's own map "Lunar Lobe" (the Moon): generated, written as RON; the Moon's own textures, sky and music, the game's shapes of the power-ups and pads |
| `tt4_1.ter` | OpenTT's own map "Lunar Lobe" (the Moon): generated |
| `tt4_1_macro.png` | OpenTT's own map "Lunar Lobe" (the Moon): its terrain's macro colour map (`macro_texture`), generated |
| `tt4_2.ron` | OpenTT's own map "Cortex Craters" (the Moon): generated, written as RON; the Moon's own textures, sky and music, the game's shapes of the power-ups and pads |
| `tt4_2.ter` | OpenTT's own map "Cortex Craters" (the Moon): generated |
| `tt4_2_macro.png` | OpenTT's own map "Cortex Craters" (the Moon): its terrain's macro colour map (`macro_texture`), generated |
