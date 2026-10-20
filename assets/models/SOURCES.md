# Provenance: models/: shapes (.dts) and their textures, world sprites

Where each file of this directory comes from (paths relative to it).
Licence, the upscaling and the layout: `docs/layout.md` (§ Provenance).

This table lists OpenTT's own files of this directory; the importer builds the rest of the asset directory from your game install (`docs/import.md`).

| file | source: how |
|---|---|
| `projectiles/mortarrocket.dts` | (none): generated (Mortar power-up's rocket, OpenTT's own) by `cargo run -p tt-assets --example mortar_rocket -- assets/shapes/tanks`: a procedural DTS v24 mesh (body cylinder, ogive nose, nozzle bell, four swept fins; 2 m long, nose along +Y) |
| `projectiles/mortarrocket.png` | (none): generated (Mortar power-up's rocket, OpenTT's own) by the same example: a flat four-colour palette (olive-drab body, darker fins, gunmetal nose, near-black nozzle) |
| `projectiles/mortarrockethot.png` | (none): generated (Mortar power-up's rocket, OpenTT's own) by the same example: the self-illuminated nozzle mouth, white-yellow (255,236,170) centre to orange (255,150,40) rim |
| `projectiles/scatterprojectile.png` | (none) + (none): generated (Scatter shot power-up, OpenTT's own): Python PIL, a round pellet, radial gradient white-yellow core (255,252,215) -> yellow (255,205,45) -> orange (255,125,15); alpha channel from the mask (generated (Scatter shot power-up, OpenTT's own): Python PIL, a round soft mask (opaque to 36 % of the radius, then (1 - t)^2 to 80 %)), merged into one RGBA PNG |
| `props/dev/devbox.dts` | (none): generated (the Power-up Test dev map, OpenTT's own) by `cargo run -p tt-assets --example powerup_terrain -- assets`: a procedural DTS v24 box, 4 x 4 x 2 m standing on its origin, its mesh also the `Collision-1` detail |
| `props/dev/devbox.png` | (none): generated (the Power-up Test dev map, OpenTT's own) by `cargo run -p tt-assets --example powerup_terrain -- assets`: a light grey 4 m tile, 1 m grid lines, a dark edge |
| `reticles/scatterreticle.png` | (none): generated (Scatter shot power-up, OpenTT's own): Python PIL, drawn at 512 px and reduced to 128 (Lanczos): two bracket arcs ( ), 32 deg each side of horizontal, stroke 7 px (the default reticle's) centred 59.5 px from the middle so they run along the quad's edge, an inward tick 12 px long at each arc's middle; ice-cyan (175,238,255), alpha from the strokes |
