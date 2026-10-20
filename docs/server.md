# Running a server

## Start

```sh
opentt --dedicated --mission tt2_3 --port 28000 -- -game TS
```

* `--mission`: a map id (`tt2_3`), or a rotation: `GREEN`, `BLUE`, `RED`
  (one world's maps) or `ALL`, starting on a random map.
* `-game`: `B` battle (default), `TB` team battle, `S` scrum, `TS` team scrum.
* `--port N`: the UDP port, 28000 by default. When it is taken (another server,
  or the game's), the server tries the next ones up to 28009 and prints the port
  it got: `Server listening on 0.0.0.0:28001`. The server browser looks for LAN
  games on all of 28000–28009.
* `--data DIR`: the folder of the server's settings (`settings.ron`) and
  bans, so several servers on one machine keep their own.
* `--master-server HOST[:PORT]`: list the server on a master server for
  internet play; `--port-mapping` asks the router to forward the port
  (UPnP / NAT-PMP), otherwise forward it on the router yourself.

## Settings

Set them in the console (`set server.max_players 8`, `get server.*` lists
them) or in `settings.ron`; they are saved when the server quits.

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

## Console

`status`, `say TEXT`, `kick PLAYER`, `ban PLAYER`, `map MISSION`,
`set KEY VALUE`, `get KEY` (`get server.*`), `help`, `quit`.

## Versions

Players can join only with the same version of OpenTT as the server: both
sides must run exactly the same game simulation. The server tells the
browser its version, and the browser shows a server of another version
greyed out ("0.1.2: cannot join"). Update the server together with the
players.
