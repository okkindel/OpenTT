# Running a server

## Start

```sh
opentt --dedicated --mission tt2_3 --port 28000 -- -game TS
```

* `--mission`: a map id (`tt2_3`), or a rotation: `GREEN`, `BLUE`, `RED`,
  `MOON` (one world's maps) or `ALL` (the green, blue and red worlds'),
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
| `server.admin_password` | empty | lets a player become admin in the game |
| `server.max_bots` | 6 | bots; `server.bot.N`, `server.bot_name.N`, `server.bot_team.N` set each one's tank, name and team |
| `server.replace_bots` | 1 | a joining player takes a bot's place |
| `server.min_health_packs` | 2 | health pick-ups kept on a map (0–3) |
| `server.display_on_master` | `Always` | `Never` keeps it off the master server |
| `server.max_chat_len` | 120 | longest chat line |
| `server.flood_protection_enabled` | 1 | mutes players who spam the chat |
| `server.kick_ban_time` | 300 | seconds a kicked player stays out |
| `server.ban_time` | 1800 | seconds of a ban |
| `server.connection_error` | (text) | shown to a player whose game does not match |

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
`set KEY VALUE`, `get KEY` (`get server.*`), `help`, `quit`.

## Versions

Players can join only with the same version of OpenTT as the server: both
sides must run exactly the same game simulation. The server tells the
browser its version, and the browser shows a server of another version
greyed out ("0.1.2: cannot join"). Update the server together with the
players.
