# Running a server

## Start

```sh
opentt --dedicated --mission tt2_3 --port 28000 -- -game TS
```

* `--mission`: a map id (`tt2_3`), then the maps of `server.map_rotation`
  (below) or that map alone; or a whole rotation: `GREEN`, `BLUE`, `RED`,
  `MOON` (one world's maps) or `ALL` (every world's),
  starting on a random map.
* `-game`: `B` battle (default), `TB` team battle, `S` scrum, `TS` team scrum.
* `--port N`: the UDP port, 28000 by default. When it is taken (another
  server, or ThinkTanks itself), the server tries the next ones up to
  28009 and prints the port it got: `Server listening on 0.0.0.0:28001`.
  The server browser looks for LAN games on all of 28000–28009.
* `--data DIR`: the folder of the server's settings (`settings.ron`) and
  bans, so several servers on one machine keep their own.
* `--master-server HOST[:PORT]`: list the server on these master servers
  for internet play instead of `net.master_server` (which holds the
  default master, below); `--master-server ""` lists it nowhere.
  `--port-mapping` asks the router to forward the port (UPnP / NAT-PMP),
  otherwise forward it on the router yourself.
  Several masters: comma-separated (`--master-server a.org,b.org:28101`).
  A master on the same host (Docker) needs `tt-master --local-address PUBLIC_IP`.

## Settings

Set them in the console (`set server.max_players 8`, `get server.*` lists
them) or in `settings.ron`; they are saved when the server quits
(`docker stop` / Ctrl+C save like `quit`; a second one exits at once).

| Setting | Default | |
|---|---|---|
| `server.name` | `OpenTT Server` | the name in the server list |
| `server.max_players` | 10 | players (bots not counted) |
| `server.password` | empty | a password to join; empty: none |
| `server.max_bots` | 6 | bots; `server.bot.N`, `server.bot_name.N`, `server.bot_team.N` set each one's tank, name and team |
| `server.replace_bots` | 1 | a joining player takes a bot's place |
| `server.min_health_packs` | 2 | health pick-ups kept on a map (0–3) |
| `server.display_on_master` | `Always` | `Never` keeps it off the master server |
| `server.max_chat_len` | 120 | longest chat line |
| `server.flood_protection_enabled` | 1 | mutes players who spam the chat |
| `server.kick_ban_time` | 300 | seconds a kicked player stays out |
| `server.ban_time` | 1800 | seconds of a ban |
| `server.connection_error` | (text) | shown to a player whose game does not match |
| `server.map_rotation` | empty | the maps played in turn, as mission ids (`["tt1_1", "tt2_3"]` in `settings.ron`) or `GREEN`, `BLUE`, `RED`, `MOON`, `ALL` for whole worlds (`["ALL"]`); empty: the `--mission` map only |
| `server.map_rotation_random` | 0 | 1: the rotation's maps come in a random order (`true` in `settings.ron`) |

## Master server

`net.master_server` is the list of master servers (comma-separated
`HOST[:PORT]`, port 28100 by default) that hosted games are listed on and
the Internet list asks. A new install starts with the project's default
master; an existing one gets it added once. Removing it
from the list (Play > On-line > Join > Internet, or
`set net.master_server ...`) keeps it removed; `net.default_masters_added`
records the defaults already added.

To run your own master server:

```sh
opentt-master --port 28100 --stats-port 8080 --stats-user admin
```

* `--port N`: the UDP port (28100 by default).
* `--status SECONDS`: print the counters this often (60; 0 = never).
* `--local-address IP`: list game servers on the master's own host
  (Docker) under this IP.
* `--stats-port N`: serve `GET /stats` on TCP port N: JSON with the
  master's version and uptime, its counters and the listed servers (name,
  game type, mission, players, bots, humans, max players, version, seconds
  since the last heartbeat). The master asks the listed servers for their
  info every 30 s for it; `"info": null` until a server answers.
* `--stats-user NAME`: `/stats` needs HTTP Basic auth as NAME with the
  password in the environment variable `OPENTT_MASTER_STATS_PASSWORD`
  (the master does not start without it). Basic auth sends the password
  readably: over the internet, put the endpoint behind a TLS reverse proxy
  (nginx, Caddy) and keep the port itself closed in the firewall.

## Console

`status`, `say TEXT`, `kick PLAYER`, `ban PLAYER`, `map MISSION`,
`set KEY VALUE`, `get KEY` (`get server.*`), `help`, `quit`, and for the
map rotation:

* `rotation`: the maps in turn, the one being played marked, and whether
  they come in turn or in a random order;
* `rotation set tt1_1 tt2_3 ...`, `rotation add ID`, `rotation remove ID`:
  change it (kept in `server.map_rotation`, saved when the server quits);
* `rotation random on|off`: a random order, or in turn again (kept in
  `server.map_rotation_random`);
* `next`: the round ends now (scores shown) and the next map comes.

## Map rotation and votes

When a round ends, the next map of the rotation starts (after the last,
the first again). In a random order (`server.map_rotation_random`), a
random map of the rotation other than the one just played starts instead
(with one map, it again), and a dedicated server starts on a random one. A hosted game picks its rotation on the Host
screen: a click on a map adds it, a click on a chosen one takes it out, a
double click keeps that map only; the first one starts the game. "Random
order" below the list plays the maps in a random order.

Players can vote for a map of the rotation: F10 lists them (or type
`/vote NAME` in the chat, a map's name or id), F12 votes yes, F11 no
(Options > Controls lists the keys; `binds.ron` changes them). The player
who calls the vote votes yes. It passes as soon as more than half of the
players on the server vote yes: the round ends and that map starts, the
rotation goes on after it. It fails when that can no longer happen, after
30 seconds, or when the round ends first. Bots neither vote nor count. A
player calls a vote at most once every 3 minutes, and not in his first 3
minutes on the server; one vote runs at a time.

## Versions

Versions are `major.minor.patch`. Players and servers play together when
the first two numbers match: 0.4.0 joins a 0.4.1 server, 0.5.0 does not.
A patch release (the last number) only changes one side's things (menus,
graphics, input, the master server, packages), never the game simulation,
the network protocol, the game data or the maps; anything that changes the
game itself comes in a new minor version. The server tells the browser its
version, and the browser shows a server of another minor version greyed out
("0.3.1: cannot join"). Update the server and the players to the same minor
version; patch updates can come at any time.

The rule holds from 0.4.0: earlier versions need exactly the same
version.