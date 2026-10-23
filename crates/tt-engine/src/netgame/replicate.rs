//! Replication: moves up, snapshots and events down, puppets on the client.
//!
//! **Server**, per remote client in the game (`ReplicasDone` received):
//! * moves arrive with redundancy ([`tt_net::input`]); one per tick is
//!   applied to the control tank (the last one repeats when starved, the
//!   queue keeps at most 8);
//! * after each tick, a snapshot (all tanks and balls, quantised, delta
//!   coded against the newest snapshot the client acked, highest priority
//!   first within the budget) goes on the unreliable lane;
//! * objects (tank info, balls, triggers, the control tank's cartridge) go
//!   on the reliable lane when they change, the world's cosmetic events and
//!   projectile spawns on the event lane, all stamped with the server tick.
//!
//! **Client**: snapshots and messages fill a timeline; the render time runs
//! `D` ticks behind the newest snapshot (adaptive, 2–8 ticks) and, every
//! tick, the puppets are set to the state interpolated at the render time
//! and the messages up to it are applied (objects created and removed,
//! projectiles spawned and simulated locally, events replayed through the
//! normal cosmetic handlers).

use std::collections::{HashMap, HashSet, VecDeque};

use glam::{Affine3A, Mat3, Quat, Vec3};
use tt_game::{Event, FlagId, HitObject, Move, ProjectileId, TankId, TriggerId, TriggerKind, World};
use tt_net::input::{InputReceiver, InputSender};
use tt_gameplay::EntityId;

use super::proto::{NetHit, Rep};
use super::snapshot::{self, Header, NetMove, QFlag, QTank, State};
use crate::game::GameSim;
use crate::host::EngineHost;

/// Snapshot budget in bytes (the rest of the packet stays for reliable data).
pub const SNAPSHOT_BUDGET: usize = 760;
/// Sent snapshots kept per client (baselines).
const RING: usize = 32;
/// Received snapshots kept by the client.
const CLIENT_RING: usize = 64;
/// Server move queue cap (§2.5).
const MAX_QUEUE: usize = 8;
/// Interpolation delay bounds, ticks (§2.4).
const D_MIN: f64 = 2.0;
const D_MAX: f64 = 8.0;
/// Extrapolate at most this many ticks past the newest state, then freeze.
const EXTRAPOLATE: f64 = 3.0;
/// Within this distance (or with no control tank) a tank gains priority 1 per snapshot, else 0.25.
const NEAR: f32 = 150.0;

// ---- server ------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct TriggerInfo {
    class: String,
    def: String,
    xf: [f32; 12],
    on: bool,
    state: u8,
}

struct Sent {
    id: u32,
    seq: u16,
    state: State,
}

/// Replication state of one remote client (server side).
#[derive(Default)]
pub struct ServerRep {
    pub in_game: bool,
    next_id: u32,
    ring: VecDeque<Sent>,
    acked: u32,
    tanks: HashMap<u32, (String, String, i32)>,
    flags: HashSet<u32>,
    triggers: HashMap<u32, TriggerInfo>,
    cartridge: Option<(u32, String)>,
    priority: HashMap<u32, f32>,
    moves: InputReceiver,
    queue: VecDeque<(u32, Move)>,
    last_move: Move,
    ack_move: u32,
    starved: i32,
    /// Bytes of snapshots sent (stats).
    pub snapshot_bytes: u64,
    pub snapshots: u64,
    /// (newest move seq, the client's render tick when it sent it).
    view: Option<(u32, f64)>,
    /// The client's view delay V in ticks (smoothed): how far behind the
    /// tick its moves are applied at the tanks it draws are (lag compensation).
    pub view_delay: Option<f32>,
}

impl ServerRep {
    pub fn debug_sizes(&self) -> Vec<(&'static str, usize)> {
        vec![("baselines", self.ring.len()), ("tank infos", self.tanks.len()), ("priorities", self.priority.len()), ("triggers", self.triggers.len()), ("move queue", self.queue.len())]
    }

    /// Moves from an input message.
    pub fn receive_input(&mut self, body: &[u8]) {
        let mut r = tt_net::bits::BitReader::new(body);
        let Ok(moves) = self.moves.receive::<NetMove>(&mut r) else { return };
        // The client's render tick when it sent its newest move (lag compensation).
        if let (Some(&(newest, _)), Ok(true)) = (moves.last(), r.read_bool()) {
            if let Ok(v) = r.read_bits(32) {
                self.view = Some((newest, v as f64 / 16.0));
            }
        }
        for (seq, m) in moves {
            self.queue.push_back((seq, m.0));
            while self.queue.len() > MAX_QUEUE {
                self.queue.pop_front();
            }
        }
    }

    fn next_move(&mut self) -> Move {
        match self.queue.pop_front() {
            Some((seq, m)) => {
                self.ack_move = seq;
                self.last_move = m;
                self.starved = 0;
                m
            }
            None => {
                self.starved += 1;
                self.last_move
            }
        }
    }
}

/// Server state shared by all clients: this tick's messages.
#[derive(Default)]
pub struct ServerShared {
    known_projectiles: HashSet<u32>,
    /// (tick, message) produced by the last world tick.
    pub tick_reps: Vec<(u32, Rep)>,
}

impl ServerShared {
    pub fn known_count(&self) -> usize {
        self.known_projectiles.len()
    }
}

fn v3(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn net_hit(h: &HitObject) -> NetHit {
    match h {
        HitObject::Terrain => NetHit::Terrain,
        HitObject::Tank(t) => NetHit::Tank(t.0),
        HitObject::Static(s) => NetHit::Static(*s as u32),
    }
}

fn server_tick(w: &World) -> u32 {
    (w.time_ms / tt_game::TICK_MS) as u32
}

/// After the server's world tick: the events and new projectiles for the
/// clients (`events` = what `game::tick` got from the world).
pub fn capture(shared: &mut ServerShared, w: &World, events: &[Event]) {
    // The tick that just ran (time_ms was advanced at its end).
    let tick = server_tick(w).saturating_sub(1);
    let mut brains: HashMap<u32, String> = HashMap::new();
    let mut fires: HashMap<u32, u32> = HashMap::new();
    for e in events {
        if let Event::BrainEjected { projectile: Some(p), shape, .. } = e {
            brains.insert(p.0, shape.clone());
        }
        if let Event::Fired { tank, .. } = e {
            if let Some(t) = w.tank(*tank) {
                fires.insert(tank.0, t.fire_count);
            }
        }
    }
    // New projectiles: their state after this tick is the client's state
    // at the start of the next one.
    let mut live = HashSet::new();
    for p in &w.projectiles {
        if p.deleted {
            continue;
        }
        live.insert(p.id.0);
        // Also the ones that exploded in their first tick (point blank): the
        // shooter matches its predicted shot by them.
        if !shared.known_projectiles.contains(&p.id.0) {
            shared.tick_reps.push((
                tick + 1,
                Rep::Spawn {
                    net: p.id.0,
                    def: p.data.name.clone(),
                    pos: v3(p.pos),
                    vel: v3(p.vel),
                    source: p.source.map_or(0, |s| s.0),
                    scale: p.damage_scale,
                    brain: brains.get(&p.id.0).cloned().unwrap_or_default(),
                    // Every projectile of the shot (one, or a scatter shot's
                    // pellets) carries the shot's number.
                    fire: p.source.filter(|_| p.cosmetic_since.is_none()).and_then(|s| fires.get(&s.0)).copied().unwrap_or(0),
                },
            ));
        }
    }
    shared.known_projectiles = live;
    for e in events {
        let rep = match e {
            Event::Fired { tank, projectile, def } => {
                // Cartridges with their own fire sound name themselves.
                let own = w.projectile(*projectile).is_some_and(|p| !p.data.fire_sound.is_empty());
                Rep::Fired { tank: tank.0, def: if own { def.clone() } else { String::new() } }
            }
            Event::DryFired { tank } => Rep::DryFired { tank: tank.0 },
            Event::ProjectileBounced { projectile, .. } => match w.projectile(*projectile) {
                Some(p) => {
                    shared.tick_reps.push((tick + 1, Rep::Bounce { net: projectile.0, pos: v3(p.pos), vel: v3(p.vel) }));
                    continue;
                }
                None => continue,
            },
            Event::ProjectileExploded { projectile, pos, normal, hit, def, source } => {
                Rep::Explode { net: projectile.0, def: def.clone(), pos: v3(*pos), normal: v3(*normal), hit: net_hit(hit), source: source.map_or(0, |s| s.0) }
            }
            Event::Collided { tank, other, speed } if *speed > 0.5 => Rep::Collided { tank: tank.0, other: net_hit(other), speed: *speed },
            Event::PowerUpSwitched { trigger, on } => Rep::PowerUpSwitched { trigger: trigger.0, on: *on },
            Event::PadUsed { trigger, tank, .. } => Rep::PadUsed { trigger: trigger.0, tank: tank.0 },
            Event::VolcanoErupted { trigger, rocks } => Rep::Volcano { trigger: trigger.0, rocks: *rocks },
            Event::UserThreadPlayed { trigger, slot, scale } => Rep::UserThread { trigger: trigger.0, slot: *slot, scale: *scale },
            _ => continue,
        };
        shared.tick_reps.push((tick, rep));
    }
}

/// Before the server's world tick: each remote client's next move to its tank.
pub fn apply_moves(rep: &mut ServerRep, tank: Option<TankId>, w: &mut World) {
    if !rep.in_game {
        return;
    }
    let m = rep.next_move();
    // V = the tick this move is applied at − the client's render tick when
    // it made it (the newest move's render tick, minus the moves since).
    if let Some((newest, view)) = rep.view {
        let behind = newest.wrapping_sub(rep.ack_move);
        if behind <= 64 {
            let now = (w.time_ms / tt_game::TICK_MS) as f64;
            // +1: snapshot tick s carries the poses of the tick that ran at
            // time (s − 1)·32, the key of the world's pose history.
            let v = (now + 1.0 - (view - behind as f64)).clamp(0.0, 64.0) as f32;
            rep.view_delay = Some(rep.view_delay.map_or(v, |old| old * 0.9 + v * 0.1));
        }
    }
    if let Some(t) = tank {
        w.set_move(t, m);
    }
}

fn trigger_info(t: &tt_game::Trigger) -> TriggerInfo {
    let (class, state) = match &t.kind {
        TriggerKind::PowerUp(_) => ("PowerUp", 0),
        TriggerKind::Boost(b) => ("Boost", b.state.min(255) as u8),
        TriggerKind::Volcano(_) => ("Volcano", 0),
    };
    let m = t.transform.matrix3;
    let p = t.transform.translation;
    TriggerInfo {
        class: class.into(),
        def: t.def_name().to_string(),
        xf: [m.x_axis.x, m.x_axis.y, m.x_axis.z, m.y_axis.x, m.y_axis.y, m.y_axis.z, m.z_axis.x, m.z_axis.y, m.z_axis.z, p.x, p.y, p.z],
        on: t.on,
        state,
    }
}

/// Everything one client gets after a server tick: (reliable, events,
/// snapshot bytes). `seq` = the packet sequence the snapshot will travel in;
/// `acked` = packet seqs the client acked since the last call.
pub fn build(rep: &mut ServerRep, g: &GameSim, control: Option<TankId>, shared: &ServerShared, seq: u16, acked: &[u16]) -> (Vec<Rep>, Vec<(u32, Rep)>, Vec<u8>) {
    let w = &g.world;
    let tick = server_tick(w);
    let mut reliable = Vec::new();
    // Objects.
    let mut seen = HashSet::new();
    for t in &w.tanks {
        if t.deleted {
            continue;
        }
        seen.insert(t.id.0);
        let info = (t.data.name.clone(), t.name.clone(), t.team);
        if rep.tanks.get(&t.id.0) != Some(&info) {
            reliable.push(Rep::TankInfo { net: t.id.0, def: info.0.clone(), name: info.1.clone(), team: info.2 });
            rep.tanks.insert(t.id.0, info);
        }
    }
    let gone: Vec<u32> = rep.tanks.keys().filter(|n| !seen.contains(n)).copied().collect();
    for n in gone {
        rep.tanks.remove(&n);
        rep.priority.remove(&n);
        reliable.push(Rep::TankRemove { net: n });
    }
    let fl: HashSet<u32> = w.flags.iter().map(|f| f.id.0).collect();
    for f in &w.flags {
        if rep.flags.insert(f.id.0) {
            reliable.push(Rep::FlagAdd { net: f.id.0, def: f.data.name.clone() });
        }
    }
    let gone: Vec<u32> = rep.flags.iter().filter(|n| !fl.contains(n)).copied().collect();
    for n in gone {
        rep.flags.remove(&n);
        reliable.push(Rep::FlagRemove { net: n });
    }
    let mut trig = HashSet::new();
    for t in &w.triggers {
        trig.insert(t.id.0);
        let info = trigger_info(t);
        if rep.triggers.get(&t.id.0) != Some(&info) {
            reliable.push(Rep::TriggerSet { net: t.id.0, class: info.class.clone(), def: info.def.clone(), xf: info.xf, on: info.on, state: info.state });
            rep.triggers.insert(t.id.0, info);
        }
    }
    let gone: Vec<u32> = rep.triggers.keys().filter(|n| !trig.contains(n)).copied().collect();
    for n in gone {
        rep.triggers.remove(&n);
        reliable.push(Rep::TriggerRemove { net: n });
    }
    let ctl = control.and_then(|c| w.tank(c));
    if let Some(t) = ctl {
        let name = t.current.as_ref().map(|p| p.name.clone()).unwrap_or_default();
        let c = (t.id.0, name);
        if rep.cartridge.as_ref() != Some(&c) {
            reliable.push(Rep::Cartridge { net: c.0, projectile: c.1.clone() });
            rep.cartridge = Some(c);
        }
    }
    // Snapshot.
    for s in acked {
        if let Some(e) = rep.ring.iter().find(|e| e.seq == *s) {
            rep.acked = rep.acked.max(e.id);
        }
    }
    let base_entry = rep.ring.iter().find(|e| e.id == rep.acked);
    let empty = State::default();
    let (base_id, base) = match base_entry {
        Some(e) => (e.id, &e.state),
        None => (0, &empty),
    };
    let mut cur = State::default();
    for t in &w.tanks {
        if !t.deleted {
            cur.tanks.insert(t.id.0, QTank::of(t));
        }
    }
    for f in &w.flags {
        cur.flags.insert(f.id.0, QFlag::of(f));
    }
    // The control tank's exact state for prediction (stage 5).
    cur.owner = ctl.map(|t| (t.id.0, snapshot::owner_words(t)));
    // Priority (§4.2): near tanks gain 1 per snapshot, far ones 0.25; the
    // control tank always first.
    let eye = ctl.map(|t| t.pos);
    for t in &w.tanks {
        let gain = match eye {
            Some(e) if (t.pos - e).length() > NEAR => 0.25,
            _ => 1.0,
        };
        *rep.priority.entry(t.id.0).or_default() += gain;
    }
    let mut order: Vec<u32> = cur.tanks.keys().copied().collect();
    order.sort_by(|a, b| {
        let pa = if Some(*a) == control.map(|c| c.0) { f32::MAX } else { rep.priority.get(a).copied().unwrap_or(0.0) };
        let pb = if Some(*b) == control.map(|c| c.0) { f32::MAX } else { rep.priority.get(b).copied().unwrap_or(0.0) };
        pb.total_cmp(&pa).then(a.cmp(b))
    });
    rep.next_id += 1;
    let depth = if rep.starved > 0 { -rep.starved } else { rep.queue.len() as i32 };
    let h = Header { id: rep.next_id, baseline: base_id, tick, ack_move: rep.ack_move, depth, control: control.map_or(0, |c| c.0), ammo: ctl.map_or(0, |t| t.ammo) };
    let (bytes, held, sent) = snapshot::encode(&h, &cur, base, &order, SNAPSHOT_BUDGET);
    for n in sent {
        rep.priority.insert(n, 0.0);
    }
    rep.ring.push_back(Sent { id: rep.next_id, seq, state: held });
    while rep.ring.len() > RING {
        rep.ring.pop_front();
    }
    rep.snapshot_bytes += bytes.len() as u64;
    rep.snapshots += 1;
    let events = shared.tick_reps.clone();
    (reliable, events, bytes)
}

// ---- client ------------------------------------------------------------------------

/// Replication state of our connection to a remote server.
pub struct ClientRep {
    snaps: HashMap<u32, State>,
    snap_order: VecDeque<u32>,
    newest_id: u32,
    newest_tick: u32,
    tanks_buf: HashMap<u32, VecDeque<(u32, QTank)>>,
    flags_buf: HashMap<u32, VecDeque<(u32, QFlag)>>,
    /// Render time in server ticks.
    pub render_tick: f64,
    started: bool,
    /// Interpolation delay `D`, ticks.
    pub delay: f64,
    client_ticks: u64,
    offsets: VecDeque<f64>,
    timeline: Vec<(u32, u64, Rep)>,
    next_seq: u64,
    infos: HashMap<u32, (String, String, i32)>,
    /// Server tank id -> (script object, client world tank).
    pub tanks: HashMap<u32, (EntityId, TankId)>,
    flag_infos: HashMap<u32, String>,
    pub flags: HashMap<u32, (EntityId, FlagId)>,
    pub triggers: HashMap<u32, TriggerId>,
    /// Trigger objects we created (runtime triggers such as the scrum goal).
    created_triggers: HashMap<u32, EntityId>,
    pub projectiles: HashMap<u32, ProjectileId>,
    exploded: HashSet<u32>,
    /// Def name hashes (`rep::db_hash`) -> names.
    db_names: HashMap<u32, String>,
    pub control_net: u32,
    pub control_obj: Option<EntityId>,
    pub control_announced: bool,
    ammo: i32,
    pub input: InputSender<NetMove>,
    /// Prediction of the control tank (stage 5).
    pub pred: super::predict::Predict,
    /// Snapshots received / undecodable (stats).
    pub snapshots: u64,
    pub bad_snapshots: u64,
}

impl Default for ClientRep {
    fn default() -> Self {
        ClientRep {
            snaps: HashMap::new(),
            snap_order: VecDeque::new(),
            newest_id: 0,
            newest_tick: 0,
            tanks_buf: HashMap::new(),
            flags_buf: HashMap::new(),
            render_tick: 0.0,
            started: false,
            delay: D_MIN,
            client_ticks: 0,
            offsets: VecDeque::new(),
            timeline: Vec::new(),
            next_seq: 0,
            infos: HashMap::new(),
            tanks: HashMap::new(),
            flag_infos: HashMap::new(),
            flags: HashMap::new(),
            triggers: HashMap::new(),
            created_triggers: HashMap::new(),
            projectiles: HashMap::new(),
            exploded: HashSet::new(),
            db_names: HashMap::new(),
            control_net: 0,
            control_obj: None,
            control_announced: false,
            ammo: 0,
            input: InputSender::new(),
            pred: Default::default(),
            snapshots: 0,
            bad_snapshots: 0,
        }
    }
}

impl ClientRep {
    /// The world was rebuilt: the puppets are gone with the old one.
    pub fn world_rebuilt(&mut self) {
        self.tanks.clear();
        self.flags.clear();
        self.created_triggers.clear();
        self.triggers.clear();
        self.control_obj = None;
    }

    /// Sizes of the internal collections (soak tests: nothing may grow without bound).
    pub fn debug_sizes(&self) -> Vec<(&'static str, usize)> {
        vec![
            ("snapshots", self.snaps.len()),
            ("tank buffers", self.tanks_buf.values().map(|b| b.len()).sum()),
            ("ball buffers", self.flags_buf.values().map(|b| b.len()).sum()),
            ("timeline", self.timeline.len()),
            ("tank infos", self.infos.len()),
            ("tanks", self.tanks.len()),
            ("projectiles", self.projectiles.len()),
            ("exploded", self.exploded.len()),
            ("prediction", self.pred.debug_size()),
        ]
    }

    /// A replication message (applied when the render time reaches `tick`).
    pub fn push(&mut self, tick: u32, rep: Rep) {
        // Our own predicted shot confirmed: adopt it now (no second copy).
        if let Rep::Spawn { net, source, fire, pos, .. } = &rep {
            if *fire != 0 && self.pred.predicting(*source) {
                if let Some(pid) = self.pred.confirm_shot(*fire, Vec3::from(*pos)) {
                    self.projectiles.insert(*net, pid);
                    return;
                }
            }
        }
        self.next_seq += 1;
        self.timeline.push((tick, self.next_seq, rep));
    }

    /// A snapshot message from the unreliable lane.
    pub fn receive_snapshot(&mut self, bytes: &[u8]) {
        let Ok(h) = snapshot::decode_header(bytes) else {
            self.bad_snapshots += 1;
            return;
        };
        if h.id <= self.newest_id && self.snaps.contains_key(&h.id) {
            return;
        }
        let empty = State::default();
        let base = if h.baseline == 0 {
            &empty
        } else {
            match self.snaps.get(&h.baseline) {
                Some(b) => b,
                None => {
                    self.bad_snapshots += 1;
                    return;
                }
            }
        };
        let Ok((h, st)) = snapshot::decode(bytes, base) else {
            self.bad_snapshots += 1;
            return;
        };
        self.snapshots += 1;
        self.input.ack(h.ack_move);
        if h.tick > self.newest_tick || !self.started {
            // Jitter: arrival time relative to the server tick.
            let off = self.client_ticks as f64 - h.tick as f64;
            self.offsets.push_back(off);
            while self.offsets.len() > 64 {
                self.offsets.pop_front();
            }
            let min = self.offsets.iter().copied().fold(f64::MAX, f64::min);
            let mut j: Vec<f64> = self.offsets.iter().map(|o| o - min).collect();
            j.sort_by(f64::total_cmp);
            let p95 = j.get((j.len() * 95 / 100).min(j.len().saturating_sub(1))).copied().unwrap_or(0.0);
            self.delay = (1.0 + 1.5 * p95 + 1.0).clamp(D_MIN, D_MAX);
            self.newest_tick = h.tick;
            self.control_net = h.control;
            self.ammo = h.ammo;
            if let Some((net, words)) = &st.owner {
                self.pred.owner_state(h.ack_move, *net, words.clone(), h.depth, (h.tick as i64 - 1) * tt_game::TICK_MS);
            }
        }
        for (n, t) in &st.tanks {
            let b = self.tanks_buf.entry(*n).or_default();
            if b.back().is_none_or(|(k, _)| *k < h.tick) {
                b.push_back((h.tick, *t));
            }
        }
        for (n, f) in &st.flags {
            let b = self.flags_buf.entry(*n).or_default();
            if b.back().is_none_or(|(k, _)| *k < h.tick) {
                b.push_back((h.tick, *f));
            }
        }
        self.newest_id = self.newest_id.max(h.id);
        self.snaps.insert(h.id, st);
        self.snap_order.push_back(h.id);
        while self.snap_order.len() > CLIENT_RING {
            if let Some(old) = self.snap_order.pop_front() {
                self.snaps.remove(&old);
            }
        }
    }

    fn advance(&mut self) {
        self.client_ticks += 1;
        if self.newest_tick == 0 {
            return;
        }
        let target = self.newest_tick as f64 - self.delay;
        if !self.started {
            self.render_tick = target;
            self.started = true;
            return;
        }
        // One tick per tick, ±5 % to hold the delay; snap when far off.
        let err = target - (self.render_tick + 1.0);
        if err.abs() > 10.0 {
            self.render_tick = target;
        } else {
            self.render_tick += 1.0 + (err * 0.1).clamp(-0.05, 0.05);
        }
    }
}

/// State of `buf` at render time `r`: interpolated between the two
/// snapshots around it, extrapolated with the velocity for a few ticks
/// past the newest.
fn sample(buf: &VecDeque<(u32, QTank)>, r: f64) -> Option<(QTank, Vec3, Quat, Vec3, f32, f32)> {
    let (first, last) = (buf.front()?, buf.back()?);
    let plain = |q: &QTank| (*q, q.pos(), q.rot(), q.vel(), q.yaw_n(), q.pitch_n());
    if r <= first.0 as f64 {
        return Some(plain(&first.1));
    }
    if r >= last.0 as f64 {
        let dt = (r - last.0 as f64).min(EXTRAPOLATE) as f32 * tt_game::DT;
        let q = last.1;
        let (mut s, p) = (plain(&q), q.pos());
        if !q.corpse() {
            s.1 = p + q.vel() * dt;
        }
        return Some(s);
    }
    let i = buf.iter().position(|(k, _)| *k as f64 > r)?;
    let (ka, a) = buf[i - 1];
    let (kb, b) = buf[i];
    let f = ((r - ka as f64) / (kb as f64 - ka as f64)) as f32;
    // Discrete parts (corpse, damage, flags) switch at the later state.
    let state = if f >= 0.5 { b } else { a };
    Some((state, a.pos().lerp(b.pos(), f), a.rot().slerp(b.rot(), f), a.vel().lerp(b.vel(), f), a.yaw_n() + (b.yaw_n() - a.yaw_n()) * f, a.pitch_n() + (b.pitch_n() - a.pitch_n()) * f))
}

fn sample_flag(buf: &VecDeque<(u32, QFlag)>, r: f64) -> Option<(QFlag, Vec3)> {
    let (first, last) = (buf.front()?, buf.back()?);
    if r <= first.0 as f64 {
        return Some((first.1, first.1.pos()));
    }
    if r >= last.0 as f64 {
        let dt = (r - last.0 as f64).min(EXTRAPOLATE) as f32 * tt_game::DT;
        return Some((last.1, last.1.pos() + last.1.vel() * dt));
    }
    let i = buf.iter().position(|(k, _)| *k as f64 > r)?;
    let (ka, a) = buf[i - 1];
    let (kb, b) = buf[i];
    let f = ((r - ka as f64) / (kb as f64 - ka as f64)) as f32;
    Some((if f >= 0.5 { b } else { a }, a.pos().lerp(b.pos(), f)))
}

fn prune<T>(buf: &mut VecDeque<(u32, T)>, r: f64) {
    while buf.len() > 2 && (buf[1].0 as f64) < r - 1.0 {
        buf.pop_front();
    }
}

/// Inside the client's `game::tick`, after the previous poses were saved:
/// every puppet takes its state at the render time.
pub fn apply_states(rep: &mut ClientRep, g: &mut GameSim) {
    // Forget projectiles the world deleted (exploded and faded).
    if rep.client_ticks.is_multiple_of(64) {
        let live: HashSet<ProjectileId> = g.world.projectiles.iter().filter(|p| !p.deleted).map(|p| p.id).collect();
        rep.projectiles.retain(|_, p| live.contains(p));
        if rep.exploded.len() > 1024 {
            rep.exploded.clear();
        }
    }
    let r = rep.render_tick;
    for (net, (_, tid)) in &rep.tanks {
        let Some(buf) = rep.tanks_buf.get_mut(net) else { continue };
        prune(buf, r);
        if rep.pred.predicting(*net) {
            continue;
        }
        let Some((q, pos, rot, vel, yaw, pitch)) = sample(buf, r) else { continue };
        if let Some(t) = g.world.tank_mut(*tid) {
            q.apply(t, pos, rot, vel, yaw, pitch);
            if *net == rep.control_net {
                t.ammo = rep.ammo;
            }
        }
    }
    for (net, (_, fid)) in &rep.flags {
        let Some(buf) = rep.flags_buf.get_mut(net) else { continue };
        prune(buf, r);
        let Some((q, pos)) = sample_flag(buf, r) else { continue };
        let carrier = (q.carrier != 0).then(|| rep.tanks.get(&q.carrier).map(|(_, t)| *t)).flatten();
        if let Some(f) = g.world.flags.iter_mut().find(|f| f.id == *fid) {
            f.carrier = carrier;
            if carrier.is_none() {
                f.pos = pos;
                f.vel = q.vel();
            }
        }
    }
    // Carried balls are attached by the world tick; the carrier flag on tanks.
    let carriers: HashMap<TankId, FlagId> = g.world.flags.iter().filter_map(|f| f.carrier.map(|c| (c, f.id))).collect();
    for (_, tid) in rep.tanks.values() {
        if let Some(t) = g.world.tank_mut(*tid) {
            t.carried_flag = carriers.get(tid).copied();
        }
    }
}

/// Prediction of the control tank, after the puppets are placed.
pub fn predict(rep: &mut ClientRep, g: &mut GameSim) {
    let own = rep.tanks.get(&rep.control_net).map(|(_, t)| (rep.control_net, *t));
    let ClientRep { pred, triggers, flags, db_names, .. } = rep;
    pred.tick(g, own, |n| triggers.get(&n).copied(), |n| flags.get(&n).map(|f| f.1), db_names);
    // The own corpse is no longer predicted, and the owner state does not
    // carry its death animation: it shakes, ejects the brain jar and flips
    // over at the render time like every other tank, in step with the
    // ejected jar's spawn.
    let Some((net, tid)) = own else { return };
    let Some((q, ..)) = rep.tanks_buf.get(&net).and_then(|b| sample(b, rep.render_tick)) else { return };
    if let Some(t) = g.world.tank_mut(tid).filter(|t| t.is_corpse) {
        q.apply_death(t);
    }
}

fn client_hit(rep: &ClientRep, g: &GameSim, h: NetHit) -> HitObject {
    match h {
        NetHit::Terrain => HitObject::Terrain,
        NetHit::Tank(n) => rep.tanks.get(&n).map(|(_, t)| HitObject::Tank(*t)).unwrap_or(HitObject::Terrain),
        NetHit::Static(s) => {
            if (s as usize) < g.world.statics.len() {
                HitObject::Static(s as usize)
            } else {
                HitObject::Terrain
            }
        }
    }
}

fn mission_trigger_for(g: &GameSim, mapped: &HashMap<u32, TriggerId>, def: &str, pos: Vec3) -> Option<TriggerId> {
    let used: HashSet<TriggerId> = mapped.values().copied().collect();
    g.world
        .triggers
        .iter()
        .filter(|t| !used.contains(&t.id) && t.def_name().eq_ignore_ascii_case(def))
        .map(|t| (t.id, (Vec3::from(t.transform.translation) - pos).length()))
        .filter(|(_, d)| *d < 0.5)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

fn xf_of(a: &[f32; 12]) -> Affine3A {
    Affine3A::from_mat3_translation(Mat3::from_cols(Vec3::new(a[0], a[1], a[2]), Vec3::new(a[3], a[4], a[5]), Vec3::new(a[6], a[7], a[8])), Vec3::new(a[9], a[10], a[11]))
}

/// Before the client's `game::tick` (the world exists, scripts may run):
/// advances the render time and applies the messages it reached.
pub fn pre_tick(h: &mut EngineHost) {
    let Some(rep) = h.netgame.client_rep_mut() else { return };
    rep.advance();
    if !rep.started || h.game.is_none() {
        return;
    }
    // Def names by hash (owner-state cartridges, spawns).
    if rep.db_names.is_empty() {
        let _ = db_name(rep, &h.defs, "#00000000");
    }
    let r = rep.render_tick;
    let mut due: Vec<(u32, u64, Rep)> = Vec::new();
    rep.timeline.retain(|e| {
        if (e.0 as f64) <= r {
            due.push(e.clone());
            false
        } else {
            true
        }
    });
    due.sort_by_key(|e| (e.0, e.1));
    for (_, _, m) in due {
        apply(h, m);
    }
    // Tanks whose first state the render time reached are created now.
    create_due(h);
    // The control object (initialControlSet from the first snapshot naming it).
    let Some(rep) = h.netgame.client_rep_mut() else { return };
    let ctl = rep.tanks.get(&rep.control_net).map(|(o, _)| *o);
    if ctl != rep.control_obj {
        rep.control_obj = ctl;
        if ctl.is_some() && !rep.control_announced {
            rep.control_announced = true;
            crate::gameplay::client::connection::initial_control_set(h);
        }
    }
}

fn create_due(h: &mut EngineHost) {
    let Some(rep) = h.netgame.client_rep_mut() else { return };
    let r = rep.render_tick;
    let ready: Vec<(u32, (String, String, i32))> = rep
        .infos
        .iter()
        .filter(|(n, _)| !rep.tanks.contains_key(n) && rep.tanks_buf.get(n).and_then(|b| b.front()).is_some_and(|(k, _)| *k as f64 <= r + 1.0))
        .map(|(n, i)| (*n, i.clone()))
        .collect();
    for (net, (db, name, team)) in ready {
        let Some(g) = h.game.as_mut() else { return };
        // As a new `Tank` object: at the unset fields' transform (0 0 1).
        let Some(obj) = g.add_tank(&db, "", glam::Vec3::new(0.0, 0.0, 1.0), glam::Mat3::IDENTITY, None) else {
            h.log.warn(&format!("net: no tank def {db}"));
            if let Some(rep) = h.netgame.client_rep_mut() {
                rep.infos.remove(&net);
            }
            continue;
        };
        let Some(tid) = g.tank_of(obj) else { continue };
        g.world.set_team(tid, team);
        g.world.set_player_name(tid, &name);
        if let Some(t) = g.world.tank_mut(tid) {
            t.puppet = true;
            t.controlled = true;
        }
        if let Some(rep) = h.netgame.client_rep_mut() {
            rep.tanks.insert(net, (obj, tid));
        }
    }
    let Some(rep) = h.netgame.client_rep_mut() else { return };
    let flags: Vec<(u32, String)> = rep.flag_infos.iter().filter(|(n, _)| !rep.flags.contains_key(n)).map(|(n, d)| (*n, d.clone())).collect();
    for (net, db) in flags {
        let obj = h.game.as_mut().and_then(|g| g.add_flag(&db, "", glam::Vec3::ZERO, None));
        let fid = obj.and_then(|o| h.game.as_ref().and_then(|g| g.entities.flag(o)));
        if let Some(rep) = h.netgame.client_rep_mut() {
            rep.flag_infos.remove(&net);
            if let (Some(obj), Some(fid)) = (obj, fid) {
                rep.flags.insert(net, (obj, fid));
            }
        }
        if let (Some(fid), Some(g)) = (fid, h.game.as_mut()) {
            if let Some(f) = g.world.flags.iter_mut().find(|f| f.id == fid) {
                f.puppet = true;
            }
        }
    }
}

/// A def name from the wire (`#hash`) as our name.
fn db_name(rep: &mut ClientRep, defs: &tt_assets::data::defs::DefSet, s: &str) -> String {
    let Some(h) = s.strip_prefix('#').and_then(|x| u32::from_str_radix(x, 16).ok()) else { return s.to_string() };
    if !rep.db_names.contains_key(&h) {
        for o in defs.objects().filter(|o| !o.name.is_empty()) {
            rep.db_names.insert(super::rep::db_hash(&o.name), o.name.clone());
        }
    }
    rep.db_names.get(&h).cloned().unwrap_or_default()
}

fn apply(h: &mut EngineHost, m: Rep) {
    let defs = h.defs.clone();
    let (Some(rep), Some(_)) = (h.netgame.client_rep_mut(), h.game.as_ref()) else { return };
    match m {
        Rep::TankInfo { net, def, name, team } => {
            if let Some(&(obj, tid)) = rep.tanks.get(&net) {
                let same_db = rep.infos.get(&net).is_some_and(|i| i.0.eq_ignore_ascii_case(&def));
                if same_db {
                    rep.infos.insert(net, (def, name.clone(), team));
                    if let Some(g) = h.game.as_mut() {
                        g.world.set_team(tid, team);
                        g.world.set_player_name(tid, &name);
                    }
                    return;
                }
                rep.tanks.remove(&net);
                if let Some(g) = h.game.as_mut() {
                    g.remove_entity(obj);
                }
            }
            if let Some(rep) = h.netgame.client_rep_mut() {
                rep.infos.insert(net, (def, name, team));
            }
        }
        Rep::TankRemove { net } => {
            rep.infos.remove(&net);
            rep.tanks_buf.remove(&net);
            if let Some((obj, _)) = rep.tanks.remove(&net) {
                if let Some(g) = h.game.as_mut() {
                    g.remove_entity(obj);
                }
            }
        }
        Rep::FlagAdd { net, def } => {
            rep.flag_infos.insert(net, def);
        }
        Rep::FlagRemove { net } => {
            rep.flag_infos.remove(&net);
            rep.flags_buf.remove(&net);
            if let Some((obj, _)) = rep.flags.remove(&net) {
                if let Some(g) = h.game.as_mut() {
                    g.remove_entity(obj);
                }
            }
        }
        Rep::TriggerSet { net, class, def, xf, on, state } => {
            let xf = xf_of(&xf);
            let pos = Vec3::from(xf.translation);
            let mut tid = rep.triggers.get(&net).copied();
            if tid.is_none() {
                let g = h.game.as_ref().unwrap();
                tid = mission_trigger_for(g, &rep.triggers, &def, pos);
            }
            if tid.is_none() {
                let p = format!("{} {} {}", pos.x, pos.y, pos.z);
                // As a new object of `class` at `position`.
                let db = h.defs.get(&def).map(|d| d.name.clone()).unwrap_or_else(|| def.clone());
                let o = tt_assets::mission::SceneObject { class: class.clone(), name: String::new(), def: false, parent: None, fields: vec![("position".into(), p), ("def".into(), db)], children: Vec::new() };
                let obj = h.game.as_mut().and_then(|g| g.add_trigger(&o));
                tid = obj.and_then(|o| h.game.as_ref().and_then(|g| g.entities.trigger(o)));
                if let (Some(obj), Some(rep)) = (obj, h.netgame.client_rep_mut()) {
                    rep.created_triggers.insert(net, obj);
                }
            }
            let Some(tid) = tid else { return };
            if let Some(rep) = h.netgame.client_rep_mut() {
                rep.triggers.insert(net, tid);
            }
            let Some(g) = h.game.as_mut() else { return };
            // A new def (power-up type switched).
            let cur = g.world.triggers.iter().find(|t| t.id == tid).map(|t| t.def_name().to_string());
            if cur.as_deref().is_some_and(|c| !c.eq_ignore_ascii_case(&def)) && h.defs.get(&def).is_some() {
                if let Some(e) = g.entities.of_trigger(tid).map(|e| e.id) {
                    g.set_def(e, &def);
                }
            }
            let Some(g) = h.game.as_mut() else { return };
            if let Some(t) = g.world.triggers.iter_mut().find(|t| t.id == tid) {
                t.transform = xf;
                t.on = on;
                if let TriggerKind::Boost(b) = &mut t.kind {
                    b.state = state as usize;
                }
            }
        }
        Rep::TriggerRemove { net } => {
            let tid = rep.triggers.remove(&net);
            if let Some(obj) = rep.created_triggers.remove(&net) {
                if let Some(g) = h.game.as_mut() {
                    g.remove_entity(obj);
                }
            } else if let (Some(tid), Some(g)) = (tid, h.game.as_mut()) {
                g.world.triggers.retain(|t| t.id != tid);
            }
        }
        Rep::Cartridge { net, projectile } => {
            let tid = rep.tanks.get(&net).map(|(_, t)| *t);
            let Some(g) = h.game.as_mut() else { return };
            let p = (!projectile.is_empty()).then(|| g.projectile(&projectile)).flatten();
            if let Some(t) = tid.and_then(|t| g.world.tank_mut(t)) {
                t.current = p;
            }
        }
        Rep::Spawn { net, def, pos, vel, source, scale, brain, fire } => {
            if fire != 0 && rep.pred.predicting(source) {
                rep.pred.stats.shots_unpredicted += 1;
            }
            let def = db_name(rep, &defs, &def);
            if rep.exploded.contains(&net) || rep.projectiles.contains_key(&net) {
                return;
            }
            let src = rep.tanks.get(&source).map(|(_, t)| *t);
            let Some(g) = h.game.as_mut() else { return };
            let Some(data) = g.projectile(&def) else { return };
            let id = g.world.spawn_projectile(data, Vec3::from(pos), Vec3::from(vel), src, scale);
            if !brain.is_empty() {
                let now = g.world.time_ms;
                if let Some(p) = g.world.projectiles.last_mut() {
                    p.cosmetic_since = Some(now);
                }
                if let Some(t) = src {
                    g.world.push_event(Event::BrainEjected { tank: t, projectile: Some(id), shape: brain });
                }
            }
            if let Some(rep) = h.netgame.client_rep_mut() {
                rep.projectiles.insert(net, id);
                if rep.projectiles.len() > 4096 {
                    rep.projectiles.clear();
                }
            }
        }
        Rep::Bounce { net, pos, vel } => {
            let Some(&pid) = rep.projectiles.get(&net) else { return };
            let Some(g) = h.game.as_mut() else { return };
            if let Some(p) = g.world.projectiles.iter_mut().find(|p| p.id == pid && p.is_live()) {
                p.pos = Vec3::from(pos);
                p.vel = Vec3::from(vel);
            }
        }
        Rep::Explode { net, def, pos, normal, hit, source } => {
            let def = db_name(rep, &defs, &def);
            rep.exploded.insert(net);
            if rep.exploded.len() > 4096 {
                rep.exploded.clear();
            }
            let pid = rep.projectiles.remove(&net);
            let src = rep.tanks.get(&source).map(|(_, t)| *t);
            let g = h.game.as_ref().unwrap();
            let hit = client_hit(rep, g, hit);
            let Some(g) = h.game.as_mut() else { return };
            let now = g.world.time_ms;
            let pos = Vec3::from(pos);
            let mut effect = true;
            let mut event_id = ProjectileId(0);
            if let Some(p) = pid.and_then(|pid| g.world.projectiles.iter_mut().find(|p| p.id == pid)) {
                event_id = p.id;
                if p.exploded_at.is_none() {
                    p.pos = pos;
                    p.vel = Vec3::ZERO;
                    p.exploded_at = Some(now);
                } else if (p.pos - pos).length() < 4.0 {
                    // The local copy exploded there already (its own effect).
                    effect = false;
                }
            }
            if effect {
                g.world.push_event(Event::ProjectileExploded { projectile: event_id, pos, normal: Vec3::from(normal), hit, def, source: src });
            }
        }
        Rep::Fired { tank, def } => {
            if rep.pred.predicting(tank) {
                return; // OpenTT's own shots are shown when predicted
            }
            let def = if def.is_empty() { def } else { db_name(rep, &defs, &def) };
            if let Some(&(_, t)) = rep.tanks.get(&tank) {
                h.game.as_mut().unwrap().world.push_event(Event::Fired { tank: t, projectile: ProjectileId(0), def });
            }
        }
        Rep::DryFired { tank } => {
            if rep.pred.predicting(tank) {
                return;
            }
            if let Some(&(_, t)) = rep.tanks.get(&tank) {
                h.game.as_mut().unwrap().world.push_event(Event::DryFired { tank: t });
            }
        }
        Rep::Collided { tank, other, speed } => {
            if let Some(&(_, t)) = rep.tanks.get(&tank) {
                let g = h.game.as_ref().unwrap();
                let other = client_hit(rep, g, other);
                h.game.as_mut().unwrap().world.push_event(Event::Collided { tank: t, other, speed });
            }
        }
        Rep::PowerUpSwitched { trigger, on } => {
            if let Some(&t) = rep.triggers.get(&trigger) {
                h.game.as_mut().unwrap().world.push_event(Event::PowerUpSwitched { trigger: t, on });
            }
        }
        Rep::PadUsed { trigger, tank } => {
            if let (Some(&tr), Some(&(_, t))) = (rep.triggers.get(&trigger), rep.tanks.get(&tank)) {
                h.game.as_mut().unwrap().world.push_event(Event::PadUsed { trigger: tr, tank: t, dv: Vec3::ZERO });
            }
        }
        Rep::Volcano { trigger, rocks } => {
            if let Some(&tr) = rep.triggers.get(&trigger) {
                h.game.as_mut().unwrap().world.push_event(Event::VolcanoErupted { trigger: tr, rocks });
            }
        }
        Rep::UserThread { trigger, slot, scale } => {
            // Played from the start at the client's own time of that tick.
            if let Some(&tr) = rep.triggers.get(&trigger) {
                h.game.as_mut().unwrap().world.play_user_thread(tr, slot, scale, true);
            }
        }
    }
}

/// The mission ended (`ReplicaReset`): every replicated object goes.
pub fn reset_client(h: &mut EngineHost) {
    let Some(rep) = h.netgame.client_rep_mut() else { return };
    let mut objs: Vec<EntityId> = rep.tanks.values().map(|(o, _)| *o).collect();
    objs.extend(rep.flags.values().map(|(o, _)| *o));
    objs.extend(rep.created_triggers.values().copied());
    let input = std::mem::take(&mut rep.input);
    let pred = std::mem::take(&mut rep.pred);
    *rep = ClientRep { input, ..ClientRep::default() };
    rep.pred.stats = pred.stats;
    if let Some(g) = h.game.as_mut() {
        for o in objs {
            g.remove_entity(o);
        }
    }
}

/// The input message for this tick (the newest moves with redundancy).
pub fn input_message(rep: &mut ClientRep, packet_seq: Option<u16>) -> Option<Vec<u8>> {
    rep.input.newest_seq()?;
    let mut w = tt_net::bits::BitWriter::new();
    w.write_bits(snapshot::MSG_INPUT as u64, 8);
    rep.input.encode(&mut w);
    // Our render tick (1/16 tick): the server derives our view delay from it.
    let view = rep.render_tick;
    w.write_bool(view > 0.0);
    if view > 0.0 {
        w.write_bits(((view * 16.0) as u64).min(u32::MAX as u64), 32);
    }
    if let Some(s) = packet_seq {
        rep.input.packet_sent(s);
    }
    Some(w.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(x: i32) -> QTank {
        QTank { pos: [x * 64, 0, 0], rot: snapshot::pack_quat(Quat::IDENTITY), ..Default::default() }
    }

    #[test]
    fn interpolation_and_extrapolation() {
        let buf: VecDeque<(u32, QTank)> = vec![(10, q(0)), (12, q(2))].into();
        let (_, p, ..) = sample(&buf, 11.0).unwrap();
        assert!((p.x - 1.0).abs() < 1e-4);
        let (_, p, ..) = sample(&buf, 5.0).unwrap();
        assert_eq!(p.x, 0.0);
        let mut moving = q(2);
        moving.vel = [32 * 10, 0, 0]; // 10 m/s
        let buf: VecDeque<(u32, QTank)> = vec![(12, moving)].into();
        let (_, p, ..) = sample(&buf, 13.0).unwrap();
        assert!((p.x - (2.0 + 10.0 * tt_game::DT)).abs() < 1e-3);
        let (_, p, ..) = sample(&buf, 30.0).unwrap();
        assert!((p.x - (2.0 + 3.0 * 10.0 * tt_game::DT)).abs() < 1e-3, "frozen after 3 ticks");
    }

    #[test]
    fn render_time_tracks_the_delay() {
        let mut r = ClientRep::default();
        for t in 1..200u32 {
            r.newest_tick = t;
            r.advance();
        }
        let lag = r.newest_tick as f64 - r.render_tick;
        assert!((lag - r.delay).abs() < 1.5, "lag {lag} delay {}", r.delay);
    }
}
