# Provenance: data/: defs and game tables in RON

Where each file of this directory comes from (paths relative to it).
Licence, the upscaling and the layout: `docs/layout.md` (§ Provenance).

Every file here is authored data of this project: the game's tables in the
engine's own RON schema (`tt_assets::data::defs`, the key bindings of
`tt_gameplay::client::binds`), one file per domain, maintained by hand in
this repository. The importer copies them into the game data as they are;
it reads no scripts of the game install.

This table lists OpenTT's own files of this directory; the importer builds the rest of the asset directory from your game install (`docs/import.md`).

| file | source: how |
|---|---|
| `ai.ron` | authored data of this project: the bot drivers |
| `binds.ron` | authored data of this project: the default key bindings |
| `cameras.ron` | authored data of this project: the observer camera and the tanks' chase camera |
| `campaign.ron` | authored data of this project: the solo campaign's levels |
| `effects.ron` | authored data of this project: explosions, particles and particle emitters |
| `map_objects.ron` | authored data of this project: boost and jump pads, the volcano, the ball |
| `markers.ron` | authored data of this project: the mission markers |
| `pickups.ron` | authored data of this project: the power-ups and the power-up weapon roll |
| `sounds.ron` | authored data of this project: sound descriptions, the game's sounds, the interface sounds and the music |
| `tanks.ron` | authored data of this project: the tanks, their sounds, effects and shared physics |
| `weapons.ron` | authored data of this project: projectiles and reticles |
