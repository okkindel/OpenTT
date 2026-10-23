# Readable Pads

A **cosmetic** mod that makes the boost and jump pads easy to spot and to
read from a distance, in the pads' own teal and cyan:

- **Boost pads**: a chunky semi-transparent arrow stands on edge like a
  sign over the pad, pointing where the pad launches you, and bobs
  gently: cyan faces with brighter rims, teal edges. On a pad that turns
  after each launch, the arrow turns with it.
- **Jump pads**: thin glowing rings like the pad's own rise off it one
  after another, growing a little and fading, a column of rings always
  in flight.

The pads themselves stay as they are: the same model and texture, the
same launch, the same trigger area. The mod only draws something above
them, so players with and without it can play together.

It comes with the game, off: tick its box in Options → Mods and restart.

## What is in the folder

```text
readable-pads/
  mod.ron                              name, version, author, description
  README.md                            this file (not a game file)
  SOURCES.md                           where the files come from (not a game file)
  models/items/boostpad.overlay.dts    drawn over the boost pad (models/items/boostpad.dts)
  models/items/jumppad.overlay.dts     drawn over the jump pad (models/items/jumppad.dts)
  models/items/padglow_arrow.png       the arrow's shades: faces, rims, edges
  models/items/padglow_ring1.ifl ...   the rising rings' fades (ring1 ... ring5, each
  models/items/padglow_ring5.ifl         a fifth of a cycle after the last):
  models/items/padglow_ring01.png ...    padglow_ring01.png ... padglow_ring15.png
```

The arrow's material is translucent and single-sided (the DTS material
flag `SINGLE_SIDED`, bit 9, with `TRANSLUCENT`): it is drawn from its
front faces only, so the solid arrow shows one see-through layer.

## Overlays

A power-up's or a pad's model `NAME.dts` can have an overlay: a model
`NAME.overlay.dts` in the same folder, drawn over it with the same
position, turn and tilt. The overlay is only drawn; the game model alone
decides where the pad launches you and what it collides with. The
overlay plays its own `ambient` sequence (its nodes, objects and animated
`.ifl` textures), and a node named like one of the model's follows that
node: the boost overlay's root node is named `direction`, like the node
the boost pad turns towards its launch direction, so the arrow always
points where the pad launches. Textures are looked up next to the
overlay, in the mod first.
