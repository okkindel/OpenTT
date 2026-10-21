# Changelog

## 0.3.0

* A new world, the Moon: two maps, "Lunar Lobe" and "Cortex Craters", with
  low gravity, a lunar base, a crash site and their own music.
* Three new maps replace the old fifth map of each world:
  * "The Neverending Stream" (green): a stream between the teams' banks;
  * "Night of the Living Brains" (blue): Halloween night, a stone circle
    on the ritual hill under a blood-red moon;
  * "The Dark Cortex" (red): a dark tower on a lava sea under an eclipse.
* After an update the game imports your copy of ThinkTanks again by itself
  when the new version needs it.
* The Reload arch's lightning lights up the ground around it.
* Tanks that fall below a map's floor are out of bounds; no more dark
  lines through the fog.
* Several master servers can be entered (Join → Internet → "Master servers").
  The Internet list is the default in Join.
* Team chat is on Y; the unused alternate fire bind is gone.
* Servers save their settings on Ctrl+C or `docker stop`; a slow master
  server lookup no longer stalls the game.
* The master server downloads are now named `opentt-master-server-*`
  (Linux and Windows); its new `--local-address` option lists game servers
  on the same host (Docker).

## 0.2.0

* Scrum: the ball can be passed: a hard ram knocks it loose, the ball keeps
  its speed and the other tank can take it; a light bump no longer knocks
  it loose.
* A destroyed tank shakes, its brain pops out, then it flips over.
* The main menu tells you when a new version is out.
* Servers announce their version; the server browser shows servers of
  another version as "cannot join".
* The master server is available on its own (`opentt-master-*` downloads)
  and logs when a game server joins or leaves the list.
* The Power-up Test map is a simple training ground: a flat floor with a
  grid, pick-ups in rows.
* A guide to running a server: `docs/server.md`.
* Smaller and simpler code base.

## 0.1.2

* Imports other editions of ThinkTanks: optional files (such as the
  tutorial's pictures) may be missing, more file formats are read, and a
  mission that cannot be read is skipped instead of stopping the import.
* After the import, a report lists what was skipped, with the log folder a
  click away.
* The tutorial is not offered when its pictures are missing.

## 0.1.1

* Windows and macOS: the game starts (0.1.0 closed at once there); a crash
  shows a message and writes `logs/crash.log`.

## 0.1.0

* First release.
