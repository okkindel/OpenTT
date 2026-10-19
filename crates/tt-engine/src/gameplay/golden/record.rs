//! The golden recorder: hooks (server → client commands, local client
//! effects, world events, audio) and the per-tick snapshot. See the module
//! documentation of [`super`] for the line formats.

use crate::audio::Audio;
use crate::host::EngineHost;
use crate::Engine;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tt_audio::Handle;
use tt_game::world::Event;
use crate::gameplay::server_host::ServerGlue;
use tt_gameplay::server::ClientId;
use tt_gameplay::{HudMsg as ClientMsg, ServerMsg};

/// Every how many ticks the full tank state is written (`tanks …`).
pub const FULL_TANKS_EVERY: u64 = 32;

/// The lines, shared with the audio wrapper (which has no host access).
#[derive(Default)]
pub struct Out {
    pub lines: Vec<String>,
    /// Prefix of every line: the engine tick being run.
    pub tick: u64,
}

impl Out {
    pub fn push(&mut self, s: String) {
        self.lines.push(format!("{:6} {s}", self.tick));
    }
}

pub type Shared = Arc<Mutex<Out>>;

fn push(out: &Shared, s: String) {
    out.lock().unwrap().push(s);
}

/// FNV-1a 64.
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut h = tt_assets::FNV_START;
    tt_assets::fnv1a(&mut h, bytes);
    h
}

fn q(s: &str) -> String {
    format!("{s:?}")
}

/// A tagged string written `\x01<id> text` as its text; anything else as is.
pub fn text(s: &str) -> String {
    match s.strip_prefix('\u{1}') {
        Some(rest) => rest.split_once(' ').map_or("", |(_, tail)| tail).to_string(),
        None => s.to_string(),
    }
}

/// Arguments as quoted texts, trailing empty ones dropped.
fn list(args: &[String]) -> String {
    let mut v: Vec<String> = args.iter().map(|a| text(a)).collect();
    while v.last().is_some_and(|s| s.is_empty()) {
        v.pop();
    }
    v.iter().map(|s| q(s)).collect::<Vec<_>>().join(" ")
}

pub struct Golden {
    pub out: Shared,
    /// Server client id → number.
    clients: HashMap<u32, u32>,
    /// The server's teams (id, object name), as the server last said.
    teams: Vec<(u32, &'static str)>,
    /// Server object ids as a remote client sees them → number.
    remote_clients: HashMap<String, u32>,
    next_client: u32,
    remote_teams: HashMap<String, u32>,
    /// Last value per keyed line, by category.
    prev: HashMap<&'static str, BTreeMap<String, String>>,
    /// World tanks already reported (`spawn`).
    tanks: HashSet<u32>,
    world: String,
}

impl Default for Golden {
    fn default() -> Self {
        Golden::new()
    }
}

impl Golden {
    pub fn new() -> Golden {
        Golden { out: Arc::new(Mutex::new(Out::default())), clients: HashMap::new(), teams: Vec::new(), remote_clients: HashMap::new(), next_client: 0, remote_teams: HashMap::new(), prev: HashMap::new(), tanks: HashSet::new(), world: String::new() }
    }

    pub fn shared(&self) -> Shared {
        self.out.clone()
    }

    pub fn push(&self, s: String) {
        push(&self.out, s);
    }

    pub fn set_tick(&self, tick: u64) {
        self.out.lock().unwrap().tick = tick;
    }

    /// The lines recorded since the last call.
    pub fn drain(&self) -> Vec<String> {
        std::mem::take(&mut self.out.lock().unwrap().lines)
    }

    fn number(&mut self, id: u32) -> u32 {
        if let Some(n) = self.clients.get(&id) {
            return *n;
        }
        self.next_client += 1;
        self.clients.insert(id, self.next_client);
        self.next_client
    }

    /// Numbers the members of `ClientGroup` and `BotClientGroup` (join
    /// order), as the server keeps them.
    pub fn refresh(&mut self, srv: &ServerGlue) {
        for c in srv.group.iter().chain(&srv.bots) {
            self.number(c.0);
        }
        self.teams = srv.teams.iter().map(|(t, n)| (t.0, *n)).collect();
    }

    /// A client reference (a server client id) on a server.
    fn client(&mut self, s: &str) -> String {
        match s.parse::<u32>() {
            Ok(0) => "-".into(),
            Ok(id) => format!("C{}", self.number(id)),
            Err(_) if s.is_empty() || s == "-1" => "-".into(),
            Err(_) => format!("?{}", q(s)),
        }
    }

    /// A client of the server by id.
    fn client_id(&mut self, c: Option<ClientId>) -> String {
        match c {
            Some(c) => format!("C{}", self.number(c.0)),
            None => "-".into(),
        }
    }

    /// A client reference in a message a remote client received (the
    /// server's object id).
    fn remote_client(&mut self, s: &str) -> String {
        if s.is_empty() || s == "0" || s == "-1" {
            return "-".into();
        }
        if let Some(n) = self.remote_clients.get(s) {
            return format!("C{n}");
        }
        self.next_client += 1;
        self.remote_clients.insert(s.to_string(), self.next_client);
        format!("C{}", self.next_client)
    }

    fn who(&mut self, remote: bool, s: &str) -> String {
        if remote {
            self.remote_client(s)
        } else {
            self.client(s)
        }
    }

    fn team(&mut self, remote: bool, s: &str) -> String {
        if remote {
            if s.is_empty() || s == "0" || s == "-1" {
                return "-".into();
            }
            let n = self.remote_teams.len() as u32 + 1;
            let n = *self.remote_teams.entry(s.to_string()).or_insert(n);
            return format!("Team{n}");
        }
        match self.teams.iter().find(|(id, n)| id.to_string() == s || n.eq_ignore_ascii_case(s)) {
            Some((_, n)) => n.to_string(),
            None if s.is_empty() || s == "0" || s == "-1" => "-".into(),
            None => format!("?{}", q(s)),
        }
    }

    /// A server → client command (name already as text, `args` after the
    /// name) in its `ServerMsg` form: (line kind, body).
    pub fn decode(&mut self, remote: bool, name: &str, args: &[String]) -> (&'static str, String) {
        let a = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        let t = |i: usize| text(a(i));
        let body = match name {
            "ServerMessage" => {
                let (kind, tmpl) = (t(0), t(1));
                let r = |i: usize| a(i + 2);
                let rt = |i: usize| t(i + 2);
                match kind.as_str() {
                    "MsgClientJoin" => format!(
                        "ClientJoin id={} name={} team={} guid={} score={} cum={} ai={} admin={} super={} text={}",
                        self.who(remote, r(1)),
                        q(&rt(0)),
                        self.team(remote, r(2)),
                        q(r(3)),
                        q(r(4)),
                        q(r(5)),
                        q(r(6)),
                        q(r(7)),
                        q(r(8)),
                        q(&tmpl)
                    ),
                    "MsgClientDrop" => format!("ClientDrop id={} name={} text={}", self.who(remote, r(1)), q(&rt(0)), q(&tmpl)),
                    "MsgClientKilled" => format!("ClientKilled victim={} killer={} text={}", q(&rt(0)), q(&rt(1)), q(&tmpl)),
                    "MsgClientScoreChanged" => format!("ScoreChanged id={} score={} cum={}", self.who(remote, r(2)), q(r(0)), q(r(1))),
                    "MsgTeamScoreChanged" => format!("TeamScoreChanged team={} score={} cum={}", self.team(remote, r(2)), q(r(0)), q(r(1))),
                    "MsgAddTeam" => format!("AddTeam team={} name={} score={} cum={}", self.team(remote, r(0)), q(&rt(1)), q(r(2)), q(r(3))),
                    _ => format!("Server kind={} text={} args=[{}]", q(&kind), q(&tmpl), list(args.get(2..).unwrap_or(&[]))),
                }
            }
            "ChatMessage" => format!("Chat from={} voice={} pitch={} text={} args=[{}]", self.who(remote, a(0)), q(&t(1)), q(a(2)), q(&t(3)), list(args.get(4..).unwrap_or(&[]))),
            "TTClient" => {
                let kind = t(0);
                let rest: Vec<String> = args.get(1..).unwrap_or(&[]).iter().map(|s| text(s)).collect();
                return match ClientMsg::decode(&kind, &rest) {
                    Some(m) => ("hud", format!("{m:?}")),
                    None => ("hud", format!("unknown {} [{}]", q(&kind), list(&rest))),
                };
            }
            _ => {
                let l = list(args);
                if l.is_empty() {
                    name.to_string()
                } else {
                    format!("{name} {l}")
                }
            }
        };
        ("msg", body)
    }

    fn keyed(&mut self, cat: &'static str, cur: BTreeMap<String, String>) {
        let prev = self.prev.entry(cat).or_default();
        let mut out = Vec::new();
        for (k, v) in &cur {
            if prev.get(k) != Some(v) {
                out.push(format!("{k} {v}"));
            }
        }
        for k in prev.keys() {
            if !cur.contains_key(k) {
                out.push(format!("{k} removed"));
            }
        }
        *prev = cur;
        for l in out {
            self.push(l);
        }
    }
}

/// `GameEnd`'s obfuscated stats (`strfrip(strswiz(synch_time @ "echo", 12),
/// "<counts> <kills> <deaths>")`) in clear.
fn game_end(synch: &str, msg: &str) -> String {
    let key = tt_game::obfuscation::strswiz(&format!("{synch}echo"), "12");
    let stats = tt_game::obfuscation::strfrap_decode(&key, msg).unwrap_or_default();
    let w: Vec<&str> = stats.split(' ').collect();
    let g = |i: usize| w.get(i).copied().unwrap_or("");
    format!("GameEnd counts={} kills={} deaths={}", q(g(0)), q(g(1)), q(g(2)))
}

/// A message as the texts of its script form (the command name, the
/// arguments: tags as their texts, templates not expanded).
fn texts(msg: &ServerMsg) -> (&'static str, Vec<String>) {
    let c = msg.lower();
    (c.name, c.args.iter().map(|v| v.text().to_string()).collect())
}

/// Hook: a server message to client `target` on a server.
pub fn on_message(h: &mut EngineHost, target: Option<ClientId>, msg: &ServerMsg) {
    let Some(mut g) = h.golden.take() else { return };
    g.refresh(&h.srv);
    let (name, args) = texts(msg);
    let to = g.client_id(target);
    let (kind, body) = if let ServerMsg::GameEnd { stats } = msg {
        let synch = target.and_then(|t| h.srv.game_end_keys.get(&t).cloned()).unwrap_or_default();
        ("msg", game_end(&synch, stats))
    } else {
        g.decode(false, name, &args)
    };
    g.push(format!("{kind} {to} {body}"));
    h.golden = Some(g);
}

/// Hook: a remote client received a server message.
pub fn on_received(h: &mut EngineHost, msg: &ServerMsg) {
    let Some(mut g) = h.golden.take() else { return };
    let (name, args) = texts(msg);
    let (kind, body) = if let ServerMsg::GameEnd { stats } = msg {
        let synch = h.cl.synch_time_text();
        ("msg", game_end(&synch, stats))
    } else {
        g.decode(true, name, &args)
    };
    g.push(format!("{kind} me {body}"));
    h.golden = Some(g);
}

/// Hook: the K/D/A or ping list a remote client received (server ids).
pub fn on_received_list(h: &mut EngineHost, kind: &str, entries: &[(u32, String)]) {
    let Some(mut g) = h.golden.take() else { return };
    let e: Vec<String> = entries.iter().map(|(id, v)| format!("{}={v}", g.remote_client(&id.to_string()))).collect();
    g.push(format!("msg me {kind} {}", e.join(" ")));
    h.golden = Some(g);
}

/// Hook: a client-side effect applied in this process (the local client).
pub fn on_hud(h: &mut EngineHost, target: Option<ClientId>, msg: &ClientMsg) {
    let Some(mut g) = h.golden.take() else { return };
    g.refresh(&h.srv);
    let to = g.client_id(target);
    g.push(format!("hud {to} {msg:?}"));
    h.golden = Some(g);
}

/// Hook: the world events of an authority tick.
pub fn on_world_events(h: &mut EngineHost, events: &[Event]) {
    let Some(g) = h.golden.as_ref() else { return };
    for e in events {
        let l = match e {
            Event::Killed { tank, killer } => match killer {
                Some(k) => format!("death T{} by T{}", tank.0, k.0),
                None => format!("death T{}", tank.0),
            },
            Event::TankRemoved { tank } => format!("gone T{}", tank.0),
            Event::PowerUpEntered { trigger, tank, def } => format!("pup P{} enter T{} {def}", trigger.0, tank.0),
            Event::PowerUpLeft { trigger, tank, def } => format!("pup P{} leave T{} {def}", trigger.0, tank.0),
            Event::PowerUpSwitched { trigger, on } => format!("pup P{} {}", trigger.0, if *on { "on" } else { "off" }),
            Event::PadUsed { trigger, tank, .. } => format!("pad P{} T{}", trigger.0, tank.0),
            Event::FlagPickedUp { flag, tank } => format!("flag F{} taken T{}", flag.0, tank.0),
            Event::FlagDropped { tank, flag } => format!("flag F{} dropped T{}", flag.0, tank.0),
            Event::FlagOutOfBounds { flag } => format!("flag F{} out", flag.0),
            Event::VolcanoErupted { trigger, rocks } => format!("volcano P{} {rocks}", trigger.0),
            _ => continue,
        };
        g.push(l);
    }
}

/// The world's tanks with their trace ids: tt-game's ids in world order;
/// on a remote client the server's (network) ids in their order (the
/// client creates its puppets in no fixed order), replicated tanks only.
fn tanks<'a>(h: &EngineHost, game: &'a crate::game::GameSim) -> Vec<(u32, &'a tt_game::Tank)> {
    if game.role != crate::game::Role::Client {
        return game.world.tanks.iter().map(|t| (t.id.0, t)).collect();
    }
    let Some(rep) = h.netgame.client_rep() else { return Vec::new() };
    let mut v: Vec<(u32, &tt_game::Tank)> = rep.tanks.iter().filter_map(|(net, (_, tid))| game.world.tank(*tid).map(|t| (*net, t))).collect();
    v.sort_by_key(|x| x.0);
    v
}

fn bits(v: f32) -> String {
    format!("{:x}", v.to_bits())
}

/// The end-of-tick snapshot.
pub fn snapshot(e: &mut Engine) {
    let Some(mut g) = e.host.golden.take() else { return };
    g.refresh(&e.host.srv);
    // New tanks.
    let mut new_tanks = Vec::new();
    if let Some(game) = e.host.game.as_ref() {
        for (id, t) in tanks(&e.host, game) {
            if g.tanks.insert(id) {
                let owner = game.entities.of_tank(t.id).and_then(|e| e.owner);
                new_tanks.push((id, owner, t.data.name.clone(), format!("{},{},{}", bits(t.pos.x), bits(t.pos.y), bits(t.pos.z))));
            }
        }
    }
    let remote = e.host.game.as_ref().is_some_and(|g| g.role == crate::game::Role::Client);
    for (id, owner, db, pos) in new_tanks {
        let c = if remote { "-".to_string() } else { g.client_id(owner.map(ClientId)) };
        g.push(format!("spawn T{id} {c} {db} {pos}"));
    }
    // Gameplay records.
    let mut scores = BTreeMap::new();
    let mut teams = BTreeMap::new();
    if let Some(srv) = e.host.srv.get() {
        for c in srv.recorded() {
            let f: Vec<String> = srv.record_fields(c).into_iter().map(|(k, v)| format!("{k}={}", q(&v))).collect();
            let k = srv.kda.get(c);
            let n = g.client_id(Some(c));
            scores.insert(format!("score {n}"), format!("{} kda={}/{}/{}", f.join(" "), k.kills, k.deaths, k.assists));
        }
        for t in srv.teams.iter().flatten() {
            let f: Vec<String> = t.rec.trace_fields().into_iter().map(|(k, v)| format!("{k}={}", q(&v))).collect();
            teams.insert(format!("team {}", t.color.name()), f.join(" "));
        }
    }
    g.keyed("score", scores);
    g.keyed("team", teams);
    // Round and mission.
    let mut round: Vec<String> = e.host.srv.state().round_fields().into_iter().map(|(n, v)| format!("{n}={}", q(&v))).collect();
    round.push(format!("sim.time_scale={}", q(&e.host.cl.time_scale_text())));
    g.keyed("round", BTreeMap::from([("round".to_string(), round.join(" "))]));
    let solo = match e.host.shell.run.as_ref() {
        Some(r) => format!(
            "seq={} total={} lives={} spscore={} kills={}/{}/{} brains={}/{} deaths={} boss={} start={}",
            r.mission_seq, r.mission_seq_total, r.lives, q(&tt_gameplay::server::records::score_text(r.score)), r.bronze_kills, r.silver_kills, r.gold_kills, r.brains, r.max_brains, r.deaths, r.boss_tank_killed, r.start_level
        ),
        None => "none".into(),
    };
    g.keyed("solo", BTreeMap::from([("solo".to_string(), solo)]));
    let content = e.host.canvas.content_name();
    let menu = match e.menu.as_ref() {
        Some(m) => format!("screen={:?} pause={} content={content}", m.state.screen(), m.pause.is_some()),
        None => format!("screen=- content={content}"),
    };
    g.keyed("menu", BTreeMap::from([("menu".to_string(), menu)]));
    if let Some(rep) = e.host.netgame.client_rep() {
        g.keyed("pred", BTreeMap::from([("pred".to_string(), format!("mispredicted={}", rep.pred.stats.mispredicted))]));
    }
    // The HUD model.
    let now = e.host.now_ms_f64() as u64;
    let mut view = BTreeMap::new();
    match e.hud_view((1280, 720), now, None) {
        None => {
            view.insert("view hud".to_string(), "off".to_string());
        }
        Some(v) => {
            let mut f = |k: &str, s: String| {
                view.insert(format!("view {k}"), s);
            };
            f("hud", "on".into());
            f("chat", format!("{:?}", v.chat));
            f("chat_input", format!("{:?}", v.chat_input));
            f("name_colors", format!("{:?}", v.name_colors));
            f("center_print", format!("{:?}", v.center_print));
            f("bottom_print", format!("{:?}", v.bottom_print));
            f("clock", format!("{:?}", v.clock));
            f("score", format!("{:?}", v.score));
            f("lives", format!("{:?}", v.lives));
            f("brains", format!("{:?}", v.brains));
            f("lag", format!("{:?}", v.lag));
            f("radar", format!("{:?}", v.radar.as_ref().map(|r| r.params)));
            f("radar_heading", format!("{:?}", v.radar.as_ref().map(|r| r.heading_deg)));
            f("player_list", format!("{:?}", v.player_list));
        }
    }
    g.keyed("view", view);
    // RNG states and tanks.
    let srng = e.host.rng.seed();
    let mut w = format!("W srng={srng}");
    let mut full = String::new();
    if let Some(game) = e.host.game.as_ref() {
        let mut s = String::new();
        for (id, t) in tanks(&e.host, game) {
            s += &format!(
                " T{}:{},{},{},{},{},{},{}{}{}",
                id,
                bits(t.pos.x),
                bits(t.pos.y),
                bits(t.pos.z),
                bits(t.yaw_n),
                bits(t.pitch_n),
                bits(t.damage),
                t.ammo,
                if t.is_corpse { "c" } else { "" },
                if t.deleted { "d" } else { "" }
            );
        }
        w += &format!(" wrng={} t={} h={:016x}", game.world.rng.seed(), game.world.time_ms, fnv(s.as_bytes()));
        full = s;
    }
    if w != g.world {
        g.push(w.clone());
        g.world = w;
    }
    let tick = g.out.lock().unwrap().tick;
    if tick % FULL_TANKS_EVERY == 0 {
        g.keyed("tanks", BTreeMap::from([("tanks".to_string(), full)]));
    }
    // Saves.
    let saves = scan_saves(e.host.fs.data_dir());
    g.keyed("save", saves.into_iter().map(|(p, b)| (format!("save {p}"), format!("len={} fnv={:016x}", b.len(), fnv(&b)))).collect());
    e.host.golden = Some(g);
}

/// Files of the data directory that are saves, by relative path.
pub fn scan_saves(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cs") || x.eq_ignore_ascii_case("ron")) {
                if let (Ok(rel), Ok(b)) = (p.strip_prefix(dir), std::fs::read(&p)) {
                    out.insert(rel.to_string_lossy().replace('\\', "/"), b);
                }
            }
        }
    }
    out
}

/// After the run (and `Engine::shutdown`): the last save changes and the
/// final text of every save.
pub fn finish(g: &mut Golden, data: &Path) {
    let saves = scan_saves(data);
    g.keyed("save", saves.iter().map(|(p, b)| (format!("save {p}"), format!("len={} fnv={:016x}", b.len(), fnv(b)))).collect());
    for (p, b) in saves {
        g.push(format!("file {p}"));
        for l in String::from_utf8_lossy(&b).lines() {
            g.push(format!("| {l}"));
        }
    }
}

/// Records every sound and music change on the way to the real audio.
pub struct GoldenAudio {
    pub inner: Box<dyn Audio>,
    pub out: Shared,
}

impl Audio for GoldenAudio {
    fn init_driver(&mut self) -> bool {
        self.inner.init_driver()
    }
    fn shutdown_driver(&mut self) {
        self.inner.shutdown_driver()
    }
    fn al_string(&self, name: &str) -> String {
        self.inner.al_string(name)
    }
    fn play_profile(&mut self, profile: &str, pos: Option<[f32; 3]>) -> Handle {
        match pos {
            Some(p) => push(&self.out, format!("snd {profile} at {},{},{}", bits(p[0]), bits(p[1]), bits(p[2]))),
            None => push(&self.out, format!("snd {profile}")),
        }
        self.inner.play_profile(profile, pos)
    }
    fn play(&mut self, h: Handle) -> Handle {
        self.inner.play(h)
    }
    fn create_source(&mut self, profile_or_desc: &str, file: Option<&str>, pos: Option<[f32; 3]>) -> Handle {
        push(&self.out, format!("src {profile_or_desc} {}", file.map(q).unwrap_or_default()));
        self.inner.create_source(profile_or_desc, file, pos)
    }
    fn stop(&mut self, h: Handle) {
        self.inner.stop(h)
    }
    fn stop_all(&mut self) {
        push(&self.out, "snd stopall".into());
        self.inner.stop_all()
    }
    fn source_f(&mut self, h: Handle, param: tt_audio::AlParam, v: f32) {
        self.inner.source_f(h, param, v)
    }
    fn source_3f(&mut self, h: Handle, param: tt_audio::AlParam, v: [f32; 3]) {
        self.inner.source_3f(h, param, v)
    }
    fn listener_f(&mut self, param: tt_audio::AlParam, v: f32) {
        self.inner.listener_f(param, v)
    }
    fn set_channel_volume(&mut self, channel: u32, v: f32) -> bool {
        self.inner.set_channel_volume(channel, v)
    }
    fn channel_volume(&self, channel: u32) -> f32 {
        self.inner.channel_volume(channel)
    }
    fn wave_len_ms(&mut self, file: &str) -> u32 {
        self.inner.wave_len_ms(file)
    }
    fn set_listener(&mut self, pos: [f32; 3], forward: [f32; 3], up: [f32; 3]) {
        self.inner.set_listener(pos, forward, up)
    }
    fn music_set_track(&mut self, name: &str) -> bool {
        push(&self.out, format!("music {name}"));
        self.inner.music_set_track(name)
    }
    fn music_stop(&mut self) {
        push(&self.out, "music stop".into());
        self.inner.music_stop()
    }
    fn update(&mut self) -> Vec<String> {
        self.inner.update()
    }
    fn status(&self) -> String {
        self.inner.status()
    }
    fn source_count(&self) -> usize {
        self.inner.source_count()
    }
    fn set_object(&mut self, class: &str, name: &str, fields: &[(String, String)]) {
        self.inner.set_object(class, name, fields)
    }
    fn profile_name(&self, name: &str) -> Option<String> {
        self.inner.profile_name(name)
    }
    fn music_name(&self, name: &str) -> Option<String> {
        self.inner.music_name(name)
    }
}
