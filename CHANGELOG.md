# Changelog

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
