//! Fuzz-style tests of the game-level decoders (random and mutated input,
//! seeded): messages, connect args, snapshots against a baseline, input.
//! Nothing may panic, and decoded snapshots must stay bounded.

use super::proto::{ConnectArgs, Msg, NetHit, Rep};
use super::snapshot::{self, Header, NetMove, QFlag, QTank, State};
use tt_net::bits::BitReader;
use tt_net::input::InputReceiver;
use tt_net::rng::Rng;

fn bytes(rng: &mut Rng, max: usize) -> Vec<u8> {
    let n = rng.below(max as u64 + 1) as usize;
    (0..n).map(|_| rng.next_u32() as u8).collect()
}

fn mutate(rng: &mut Rng, src: &[u8]) -> Vec<u8> {
    let mut d = src.to_vec();
    for _ in 0..1 + rng.below(4) {
        match rng.below(5) {
            0 if !d.is_empty() => {
                let i = rng.below(d.len() as u64) as usize;
                d[i] ^= 1 << rng.below(8);
            }
            1 if !d.is_empty() => {
                let i = rng.below(d.len() as u64) as usize;
                d[i] = rng.next_u32() as u8;
            }
            2 => d.truncate(rng.below(d.len() as u64 + 1) as usize),
            3 => d.extend(bytes(rng, 16)),
            _ => {
                let i = rng.below(d.len() as u64 + 1) as usize;
                d.insert(i, [0, 0xff, 0x7f, 0x80][rng.below(4) as usize]);
            }
        }
    }
    d
}

fn sample_state() -> State {
    let mut st = State::default();
    for i in 1..=12u32 {
        st.tanks.insert(i, QTank { pos: [i as i32 * 640, -300, 77], rot: snapshot::pack_quat(glam::Quat::from_rotation_z(i as f32)), vel: [i as i32, 3, -2], aim: [1000, 1100], damage: 30, bits: 6, corpse_t: 0, death_seq: 0 });
    }
    st.flags.insert(40, QFlag { carrier: 3, pos: [0; 3], vel: [0; 3] });
    st.owner = Some((3, (0..snapshot::OWNER_WORDS as u32).map(|k| k.wrapping_mul(0x9e37_79b9)).collect()));
    st
}

fn corpus() -> Vec<Vec<u8>> {
    let mut c: Vec<Vec<u8>> = vec![
        Msg::Game(tt_gameplay::ServerMsg::Chat { from: "1210".into(), voice: tt_gameplay::Value::empty(), pitch: String::new(), text: tt_gameplay::Value::tag("\u{5}%1: %2"), args: vec![tt_gameplay::Value::tag("Bait"), tt_gameplay::Value::str("hi")] }).encode(),
        Msg::Game(tt_gameplay::ServerMsg::Message { kind: tt_gameplay::MsgKind::ScoreChanged { score: "2".into(), cum: "12".into(), id: "1210".into() }, text: tt_gameplay::Value::empty() }).encode(),
        Msg::Cmd(tt_gameplay::ClientCmd::MessageSent { text: "hello".into() }).encode(),
        Msg::DefManifest { seq: "1".into(), files: vec!["tanks".into()], hash: 9, count: 3 }.encode(),
        Msg::MissionInfo { path: "missions/tt1_1.ron".into(), hash: 1 }.encode(),
        Msg::PingList { entries: vec![(1, 2), (3, 4)] }.encode(),
        Msg::KdaList { entries: vec![(1, 2, 3, 4)] }.encode(),
        Msg::Rep { tick: 5, rep: Rep::TriggerSet { net: 1, class: "Boost".into(), def: "Boost1".into(), xf: [1.0; 12], on: true, state: 2 } }.encode(),
        Msg::Rep { tick: 5, rep: Rep::Spawn { net: 9, def: "#00000001".into(), pos: [1.0; 3], vel: [2.0; 3], source: 3, scale: 1.0, brain: "jar".into(), fire: 4 } }.encode(),
        Msg::Rep { tick: 5, rep: Rep::Explode { net: 9, def: "#00000001".into(), pos: [1.0; 3], normal: [0.0, 0.0, 1.0], hit: NetHit::Static(4), source: 3 } }.encode(),
        ConnectArgs { args: vec!["Bait".into(), "1".into()] }.encode(),
    ];
    let base = sample_state();
    let mut cur = base.clone();
    cur.tanks.get_mut(&2).unwrap().pos[0] += 9;
    cur.tanks.remove(&4);
    if let Some((_, w)) = cur.owner.as_mut() {
        w[0] ^= 0x55;
    }
    let order: Vec<u32> = (1..=12).collect();
    let h = Header { id: 2, baseline: 1, tick: 9, ack_move: 3, depth: 1, control: 3, ammo: 4 };
    c.push(snapshot::encode(&h, &cur, &base, &order, 760).0);
    c.push(snapshot::encode(&h, &cur, &State::default(), &order, 760).0);
    c
}

#[test]
fn decoders_survive_random_and_mutated_input() {
    let mut rng = Rng::new(0xfeed);
    let base = sample_state();
    let corpus = corpus();
    for i in 0..60_000 {
        let k = rng.below(corpus.len() as u64) as usize;
        let d = if i % 3 == 0 { bytes(&mut rng, 600) } else { mutate(&mut rng, &corpus[k]) };
        let _ = Msg::decode(&d);
        let _ = ConnectArgs::decode(&d);
        let _ = snapshot::decode_header(&d);
        for b in [&base, &State::default()] {
            if let Ok((_, st)) = snapshot::decode(&d, b) {
                // Bounded: every entry costs input bits.
                assert!(st.tanks.len() <= b.tanks.len() + d.len() * 8 / 7);
                assert!(st.owner.as_ref().is_none_or(|(_, w)| w.len() <= 256));
            }
        }
        let mut r = BitReader::new(&d);
        let _ = InputReceiver::default().receive::<NetMove>(&mut r);
    }
}

#[test]
fn corpus_decodes() {
    let base = sample_state();
    for (i, c) in corpus().iter().enumerate() {
        let ok = Msg::decode(c).is_ok() || ConnectArgs::decode(c).is_ok() || snapshot::decode(c, &base).is_ok() || snapshot::decode(c, &State::default()).is_ok();
        assert!(ok, "corpus entry {i} decodes");
    }
}
