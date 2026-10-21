# Provenance: sky/: sky boxes

Where each file of this directory comes from (paths relative to it).
Licence, the upscaling and the layout: `docs/layout.md` (§ Provenance).

This table lists OpenTT's own files of this directory; the importer builds the rest of the asset directory from your game install (`docs/import.md`).

| file | source: how |
|---|---|
| `darkcortex.dml` | OpenTT's own (the red map tt3_5 "The Dark Cortex"): generated (a low overcast of dark red-grey clouds lit from below by the lava, a red glow along the horizon; no third-party input); the five faces north, east, south, west, top |
| `darkcortex01.png` | OpenTT's own (the red map tt3_5 "The Dark Cortex"): generated (a low overcast of dark red-grey clouds lit from below by the lava, a red glow along the horizon; no third-party input), 512x512 |
| `darkcortex02.png` | OpenTT's own (the red map tt3_5 "The Dark Cortex"): generated (a low overcast of dark red-grey clouds lit from below by the lava, a red glow along the horizon; no third-party input), 512x512 |
| `darkcortex03.png` | OpenTT's own (the red map tt3_5 "The Dark Cortex"): generated (a low overcast of dark red-grey clouds lit from below by the lava, a red glow along the horizon; no third-party input), 512x512 |
| `darkcortex04.png` | OpenTT's own (the red map tt3_5 "The Dark Cortex"): generated (a low overcast of dark red-grey clouds lit from below by the lava, a red glow along the horizon; no third-party input), 512x512 |
| `darkcortex05.png` | OpenTT's own (the red map tt3_5 "The Dark Cortex"): generated (a low overcast of dark red-grey clouds lit from below by the lava, a red glow along the horizon; no third-party input), 512x512 |
| `eclipse.png` | OpenTT's own (the red map tt3_5 "The Dark Cortex"): generated (no third-party input), 256x256 RGBA, a sky sprite (`Sky.sky_sprites`): the eclipsed sun, a black disc in a bright corona with streamers; 8 degrees to the square's edge |
| `halloween_cloud.png` | OpenTT's own (the Halloween map tt2_5): generated (no third-party input), 256x256 RGBA, a sky sprite (`Sky.sky_sprites`): a patch of the blue sky's night cloud (its colours picked to match, faint streaks, a ragged soft edge) laid over the pale moon painted into `skyblue03.png` (south, az 175.6, el 20), so the blood-red moon is the only one; 8.5 degrees to the square's edge |
| `halloween_moon.png` | OpenTT's own (the Halloween map tt2_5): generated (no third-party input), 256x256 RGBA, a sky sprite (`Sky.sky_sprites`): a big blood-red moon with darker maria (3D noise), a soft limb; 5 degrees to the square's edge |
| `moon.dml` | OpenTT's own (the Moon world): generated (star field, Milky Way; the Earth and the Sun are sky sprites of their own, `moon_earth.png`, `moon_sun.png`; no third-party input); the five faces north, east, south, west, top |
| `moon01.png` | OpenTT's own (the Moon world): generated (star field, Milky Way; the Earth and the Sun are sky sprites of their own, `moon_earth.png`, `moon_sun.png`; no third-party input), 1024x1024 |
| `moon02.png` | OpenTT's own (the Moon world): generated (star field, Milky Way; the Earth and the Sun are sky sprites of their own, `moon_earth.png`, `moon_sun.png`; no third-party input), 1024x1024 |
| `moon03.png` | OpenTT's own (the Moon world): generated (star field, Milky Way; the Earth and the Sun are sky sprites of their own, `moon_earth.png`, `moon_sun.png`; no third-party input), 1024x1024 |
| `moon04.png` | OpenTT's own (the Moon world): generated (star field, Milky Way; the Earth and the Sun are sky sprites of their own, `moon_earth.png`, `moon_sun.png`; no third-party input), 1024x1024 |
| `moon05.png` | OpenTT's own (the Moon world): generated (star field, Milky Way; the Earth and the Sun are sky sprites of their own, `moon_earth.png`, `moon_sun.png`; no third-party input), 1024x1024 |
| `moon_earth.png` | OpenTT's own (the Moon world): generated (no third-party input), 1024x1024 RGBA, a sky sprite (`Sky.sky_sprites`, drawn by the renderer at infinite distance): the Earth, painted (noise continents, crisp clouds, the light in soft steps from the Sun in the south-west), a sharp limb and a thin halo; 5 degrees to the square's edge |
| `moon_sun.png` | OpenTT's own (the Moon world): generated (no third-party input), 1024x1024 RGBA, a sky sprite (`Sky.sky_sprites`, drawn by the renderer at infinite distance): the Sun, a white disc (0.7 degrees) with a tight glare; 4 degrees to the square's edge |
