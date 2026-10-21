# Roadmap

Plans for the next releases, roughly in order. Nothing here is promised
for a particular version; [CHANGELOG.md](CHANGELOG.md) lists what has been
released.

## Map voting and map rotation

* At the end of a round the players vote for the next map: the server
  offers three, the one with the most votes is played next.
* A dedicated server has a map rotation: a list of missions in
  `settings.ron`, also set and shown from the server console.

## A leaderboard on the master server

* The master server already asks the listed servers for their state (for
  `/stats`); it also collects a simple ranking of the players: kills,
  goals and rounds played.
* The ranking is part of `/stats`; the game may also show it in the menu.
* Open question, to settle first: how a player is identified. A name
  alone is easy to take over by someone else.

## Winter maps

* New maps with a winter theme.
