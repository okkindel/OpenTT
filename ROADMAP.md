# Roadmap

Plans for the next releases, roughly in order. Nothing here is promised
for a particular version; [CHANGELOG.md](CHANGELOG.md) lists what has been
released.

## A leaderboard on the master server

* The master server already asks the listed servers for their state (for
  `/stats`); it also collects a simple ranking of the players: kills,
  goals and rounds played.
* The ranking is part of `/stats`; the game may also show it in the menu.
* Open question, to settle first: how a player is identified. A name
  alone is easy to take over by someone else.

## Winter maps

* New maps with a winter theme.

## Network protocol clean-up (0.5)

Changes that need a new minor version, since 0.4.x clients and servers
must keep understanding each other:

* The player-join message still carries two admin flags, always off now
  that the in-game admin login is gone; drop them.
