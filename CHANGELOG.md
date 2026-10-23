# Changelog

## 0.4.0

* Map rotation: a server plays a list of maps, one after the other. Pick
  it in the Host dialog; dedicated servers set `server.map_rotation` and
  have the console commands `rotation` and `next` (`docs/server.md`).
* Map votes: in a multiplayer game, F10 (or "Vote for a map" in the pause
  menu, or `/vote <map>`) proposes a map of the rotation; F12 votes yes,
  F11 no. When most players agree, the round ends and that map starts.
  A player can call a vote every 3 minutes.
* Readable Pads, a cosmetic mod that comes with the game (off): a glowing
  arrow over each boost pad points where it launches you, rings rise off
  jump pads. Mods can draw their own models over power-ups and pads
  (`docs/modding.md`).
* Up to 10 bots in a game; the server list shows how many of the players
  are human.
* Wide screens (21:9, 32:9) show more at the sides instead of cutting off
  the tank and the view upwards.
* Alt+Tab no longer leaves the controls stuck.
* Quick and Custom games no longer say "This server is full" after
  hosting a small server; with "Replace bots" on, bots make room for
  joining players.
* Players and servers of the same minor version (0.4.x) play together:
  patch releases need no update of the server.

## 0.3.1

* A default master server: the Internet list works out of the box, and
  games hosted with "List on the master server" (on by default) now show
  up there. Existing installs get it added once; it can be removed in
  Join → Internet → "Master servers" and then stays removed.
* The master server's `--stats-port N`: `GET /stats` as JSON (counters and
  the listed games with name, map, mode, players, bots, version), open or
  with Basic auth (`--stats-user`, password from
  `OPENTT_MASTER_STATS_PASSWORD`). See `docs/server.md`.
* Hosting asks the router to forward the port (UPnP / NAT-PMP) by default
  on new installs; the checkbox stays.
* Join: the team switch fits the buttons next to it; the server details
  show its version number.
* The Linux `.deb` is named like the other downloads
  (`opentt-<version>-linux-x86_64.deb`).
* A roadmap: `ROADMAP.md`.
* Cleaner code base: large files split into modules, parameter structs
  instead of long argument lists, no comments pointing at files outside
  the repository.

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
