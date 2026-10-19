//! Game messages on the reliable lane of a tt-net connection
//! (docs/netcode.md §5.3, §5.4). One message per tt-net message: a kind
//! byte, then varints and texts (`tt_net::msg`). The gameplay messages are
//! typed ([`Msg::Game`] = `tt_gameplay::ServerMsg`, [`Msg::Cmd`] =
//! `tt_gameplay::ClientCmd`, plan §2.3, protocol 3); their tagged texts go
//! through the connection's text table ([`TextTx`] / [`TextRx`]), so a
//! template or a name crosses the wire once.

use tt_gameplay::wire::{get_client_cmd, get_server_msg, put_client_cmd, put_server_msg};
use tt_gameplay::{ClientCmd, ServerMsg};
use tt_net::bits::{BitReader, BitWriter};
use tt_net::msg::{get_count, get_str, put_str, TextRx, TextTx};
use tt_net::DecodeError;

/// Connect request payload (`setConnectArgs`; the join password is proven
/// in the tt-net handshake, `tt_net::auth`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectArgs {
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// S→C: a gameplay message.
    Game(ServerMsg),
    /// C→S: a client command.
    Cmd(ClientCmd),
    /// S→C, `transmitDefs(seq)`: the scripts the server `exec`'d to
    /// define its defs and the hash of the resulting table.
    DefManifest { seq: String, files: Vec<String>, hash: u64, count: u32 },
    /// C→S: defs loaded and verified (→ `onDefsDone(seq)`).
    DefsDone { seq: String },
    /// S→C, `startReplication()`: the mission to load and its hash.
    MissionInfo { path: String, hash: u64 },
    /// C→S: mission loaded and verified (→ `onReplicasReceived`).
    ReplicasDone,
    /// S→C, `resetReplication()`: drop the mission objects.
    ReplicaReset,
    /// S→C, `play2D` / `play3D` on the connection.
    Sound { profile: String, pos: Option<[f32; 3]> },
    /// S→C: replication at server tick `tick` (stages 3–4, [`Rep`]).
    Rep { tick: u32, rep: Rep },
    /// S→C, every 2 s: (client connection id, ping ms) for the player list.
    PingList { entries: Vec<(u32, u32)> },
    /// S→C: (client id, kills, deaths, assists) for the player list
    /// (`gameplay::scoreboard`); on a change and every 2 s, the whole list.
    KdaList { entries: Vec<(u32, u32, u32, u32)> },
    /// S→C, before the def manifest: the server's mods that change
    /// the game (docs/modding.md); the client checks its own.
    ModList { mods: Vec<tt_net::ModRef> },
}

pub use super::rep::{NetHit, Rep};

impl ConnectArgs {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_varint(self.args.len() as u64);
        for a in &self.args {
            put_str(&mut w, a);
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<ConnectArgs, DecodeError> {
        let mut r = BitReader::new(b);
        let n = get_count(&mut r)?.min(16);
        let args = (0..n).map(|_| get_str(&mut r)).collect::<Result<_, _>>()?;
        Ok(ConnectArgs { args })
    }
}

impl Msg {
    /// Encodes a message that has no tagged texts (anything but
    /// [`Msg::Game`], or a test): a [`Msg::Game`] defines its texts afresh.
    pub fn encode(&self) -> Vec<u8> {
        self.encode_on(&mut TextTx::default())
    }

    /// Encodes for a connection whose text table is `tx`.
    pub fn encode_on(&self, tx: &mut TextTx) -> Vec<u8> {
        let mut w = BitWriter::with_capacity(64);
        match self {
            Msg::Cmd(c) => {
                w.write_bits(0, 8);
                put_client_cmd(&mut w, c);
            }
            Msg::Game(m) => {
                w.write_bits(1, 8);
                put_server_msg(&mut w, tx, m);
            }
            Msg::DefManifest { seq, files, hash, count } => {
                w.write_bits(2, 8);
                put_str(&mut w, seq);
                w.write_varint(files.len() as u64);
                for f in files {
                    put_str(&mut w, f);
                }
                w.write_bits(*hash, 64);
                w.write_varint(*count as u64);
            }
            Msg::DefsDone { seq } => {
                w.write_bits(3, 8);
                put_str(&mut w, seq);
            }
            Msg::MissionInfo { path, hash } => {
                w.write_bits(4, 8);
                put_str(&mut w, path);
                w.write_bits(*hash, 64);
            }
            Msg::ReplicasDone => w.write_bits(5, 8),
            Msg::ReplicaReset => w.write_bits(6, 8),
            Msg::Sound { profile, pos } => {
                w.write_bits(8, 8);
                put_str(&mut w, profile);
                w.write_bool(pos.is_some());
                if let Some(p) = pos {
                    for v in p {
                        w.write_bits(v.to_bits() as u64, 32);
                    }
                }
            }
            Msg::Rep { tick, rep } => {
                w.write_bits(9, 8);
                w.write_varint(*tick as u64);
                rep.encode(&mut w);
            }
            Msg::PingList { entries } => {
                w.write_bits(10, 8);
                w.write_varint(entries.len() as u64);
                for (id, p) in entries {
                    w.write_varint(*id as u64);
                    w.write_varint(*p as u64);
                }
            }
            Msg::ModList { mods } => {
                w.write_bits(12, 8);
                w.write_varint(mods.len() as u64);
                for m in mods {
                    put_str(&mut w, &m.name);
                    put_str(&mut w, &m.version);
                    w.write_bits(m.hash, 64);
                }
            }
            Msg::KdaList { entries } => {
                w.write_bits(11, 8);
                w.write_varint(entries.len() as u64);
                for (id, k, d, a) in entries {
                    for v in [id, k, d, a] {
                        w.write_varint(*v as u64);
                    }
                }
            }
        }
        w.finish()
    }

    /// Decodes with a fresh text table (see [`Msg::encode`]).
    pub fn decode(b: &[u8]) -> Result<Msg, DecodeError> {
        Msg::decode_on(b, &mut TextRx::default())
    }

    /// Decodes on a connection whose text table is `rx`.
    pub fn decode_on(b: &[u8], rx: &mut TextRx) -> Result<Msg, DecodeError> {
        let mut r = BitReader::new(b);
        Ok(match r.read_bits(8)? {
            0 => Msg::Cmd(get_client_cmd(&mut r)?),
            1 => Msg::Game(get_server_msg(&mut r, rx)?),
            2 => {
                let seq = get_str(&mut r)?;
                let n = get_count(&mut r)?;
                let files = (0..n).map(|_| get_str(&mut r)).collect::<Result<_, _>>()?;
                Msg::DefManifest { seq, files, hash: r.read_bits(64)?, count: r.read_varint()? as u32 }
            }
            3 => Msg::DefsDone { seq: get_str(&mut r)? },
            4 => Msg::MissionInfo { path: get_str(&mut r)?, hash: r.read_bits(64)? },
            5 => Msg::ReplicasDone,
            6 => Msg::ReplicaReset,
            // 7: once `ControlSet` (the control object now comes with the snapshot header).
            8 => {
                let profile = get_str(&mut r)?;
                let pos = if r.read_bool()? {
                    let mut p = [0f32; 3];
                    for v in &mut p {
                        *v = f32::from_bits(r.read_bits(32)? as u32);
                    }
                    Some(p)
                } else {
                    None
                };
                Msg::Sound { profile, pos }
            }
            9 => Msg::Rep { tick: r.read_varint()? as u32, rep: Rep::decode(&mut r)? },
            10 => {
                let n = get_count(&mut r)?;
                let mut entries = Vec::with_capacity(n);
                for _ in 0..n {
                    entries.push((r.read_varint()? as u32, r.read_varint()?.min(u32::MAX as u64) as u32));
                }
                Msg::PingList { entries }
            }
            11 => {
                let n = get_count(&mut r)?;
                let mut entries = Vec::with_capacity(n);
                let mut v = || -> Result<u32, DecodeError> { Ok(r.read_varint()?.min(u32::MAX as u64) as u32) };
                for _ in 0..n {
                    entries.push((v()?, v()?, v()?, v()?));
                }
                Msg::KdaList { entries }
            }
            12 => {
                let n = get_count(&mut r)?;
                let mut mods = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    mods.push(tt_net::ModRef { name: get_str(&mut r)?, version: get_str(&mut r)?, hash: r.read_bits(64)? });
                }
                Msg::ModList { mods }
            }
            k => return Err(DecodeError::BadKind(k as u8)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let all = vec![
            Msg::Game(ServerMsg::Message { kind: tt_gameplay::MsgKind::ClientDrop { name: tt_gameplay::Value::tag("Bait"), id: "1210".into() }, text: tt_gameplay::Value::tag("%1 left") }),
            Msg::Game(ServerMsg::MissionPhase1 { seq: "1".into(), file: "missions/tt1_1.ron".into(), music: "Lush".into() }),
            Msg::Cmd(ClientCmd::PhaseAck { phase: 2, seq: "1".into() }),
            Msg::Cmd(ClientCmd::MessageSent { text: "hello".into() }),
            Msg::DefManifest { seq: "1".into(), files: vec!["tanks".into()], hash: u64::MAX - 3, count: 140 },
            Msg::DefsDone { seq: "1".into() },
            Msg::MissionInfo { path: "missions/tt1_1.ron".into(), hash: 42 },
            Msg::ReplicasDone,
            Msg::ReplicaReset,
            Msg::Sound { profile: "fireSound".into(), pos: Some([1.0, -2.5, 3.25]) },
            Msg::Sound { profile: "x".into(), pos: None },
            Msg::Rep { tick: 9, rep: Rep::TankInfo { net: 4, def: "GoldHeavyTank".into(), name: "Hum".into(), team: -1 } },
            Msg::Rep { tick: 9, rep: Rep::TriggerSet { net: 5, class: "PowerUp".into(), def: "Heal".into(), xf: [1.0; 12], on: true, state: 3 } },
            Msg::Rep { tick: 9, rep: Rep::Spawn { net: 6, def: format!("#{:08x}", super::super::rep::db_hash("D")), pos: [1.0, 2.5, -3.25], vel: [85.0, 0.5, 0.0], source: 4, scale: 1.0, brain: String::new(), fire: 3 } },
            Msg::Rep { tick: 9, rep: Rep::Spawn { net: 7, def: "#00000001".into(), pos: [1.0; 3], vel: [0.0; 3], source: 4, scale: 1.0, brain: "jar.dts".into(), fire: 0 } },
            Msg::Rep { tick: 9, rep: Rep::Explode { net: 6, def: "#00000002".into(), pos: [1.0; 3], normal: [0.0, 0.0, 1.0], hit: NetHit::Tank(4), source: 0 } },
            Msg::Rep { tick: 9, rep: Rep::Collided { tank: 4, other: NetHit::Static(77), speed: 3.5 } },
            Msg::Rep { tick: 9, rep: Rep::PadUsed { trigger: 5, tank: 4 } },
            Msg::Rep { tick: 9, rep: Rep::UserThread { trigger: 5, slot: 1, scale: 1.0 } },
            Msg::Rep { tick: 9, rep: Rep::Fired { tank: 4, def: String::new() } },
            Msg::Rep { tick: 9, rep: Rep::Fired { tank: 4, def: format!("#{:08x}", super::super::rep::db_hash("ScatterProjectile")) } },
            Msg::PingList { entries: vec![(1210, 64), (1211, 130)] },
            Msg::KdaList { entries: vec![(1210, 3, 1, 2), (1211, 0, 4, 0)] },
            Msg::ModList { mods: vec![] },
            Msg::ModList { mods: vec![tt_net::ModRef { name: "Big Maps".into(), version: "1.0".into(), hash: u64::MAX - 1 }] },
        ];
        for m in all {
            assert_eq!(Msg::decode(&m.encode()).unwrap(), m);
        }
        let c = ConnectArgs { args: vec!["Bait".into(), "1".into(), "2".into()] };
        assert_eq!(ConnectArgs::decode(&c.encode()).unwrap(), c);
        assert!(Msg::decode(&[1, 200]).is_err());
        assert!(Msg::decode(&[]).is_err());
    }
}
