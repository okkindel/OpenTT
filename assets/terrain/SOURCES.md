# Provenance: terrain/: terrain textures

Where each file of this directory comes from (paths relative to it).
Licence, the upscaling and the layout: `docs/layout.md` (§ Provenance).

This table lists OpenTT's own files of this directory; the importer builds the rest of the asset directory from your game install (`docs/import.md`).

| file | source: how |
|---|---|
| `dev/devgrid.png` | (none): generated (the Power-up Test dev map, OpenTT's own) by `cargo run -p tt-assets --example powerup_terrain -- assets`: an orange dev grid over one 32 m material tile (1024 px): 8 m checker tiles, 1 m lines, dark grey 8 m lines |
