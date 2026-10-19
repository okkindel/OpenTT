//! Fuzz-style robustness tests with a seeded generator (no cargo-fuzz /
//! nightly needed): random and mutated datagrams must never panic a decoder,
//! the server or the client, and must not disturb a connection they cannot
//! authenticate (different source address or wrong session).

mod common;

use std::time::Duration;

use common::*;
use tt_net::bits::{BitReader, BitWriter};
use tt_net::input::{InputReceiver, MoveCodec};
use tt_net::packet::{DataBody, Packet};
use tt_net::rng::Rng;
use tt_net::*;

fn random_bytes(rng: &mut Rng, max: usize) -> Vec<u8> {
    let n = rng.below(max as u64 + 1) as usize;
    (0..n).map(|_| rng.next_u32() as u8).collect()
}

fn mutate(rng: &mut Rng, src: &[u8]) -> Vec<u8> {
    let mut d = src.to_vec();
    for _ in 0..1 + rng.below(4) {
        match rng.below(7) {
            0 if !d.is_empty() => {
                let i = rng.below(d.len() as u64) as usize;
                d[i] ^= 1 << rng.below(8);
            }
            1 if !d.is_empty() => {
                let i = rng.below(d.len() as u64) as usize;
                d[i] = rng.next_u32() as u8;
            }
            2 => d.truncate(rng.below(d.len() as u64 + 1) as usize),
            3 => {
                let extra = random_bytes(rng, 64);
                d.extend(extra);
            }
            4 if d.len() > 1 => {
                // Keep the kind/header, randomise the rest.
                let keep = (1 + rng.below(16)) as usize;
                for b in d.iter_mut().skip(keep) {
                    *b = rng.next_u32() as u8;
                }
            }
            5 if !d.is_empty() => {
                let i = rng.below(d.len() as u64) as usize;
                d[i] = [0x00, 0xff, 0x7f, 0x80][rng.below(4) as usize];
            }
            _ => {
                let i = rng.below(d.len() as u64 + 1) as usize;
                d.insert(i, rng.next_u32() as u8);
            }
        }
    }
    d.truncate(1500);
    d
}

#[derive(Debug, Clone, PartialEq)]
struct Mv(u32);

impl MoveCodec for Mv {
    fn encode(&self, w: &mut BitWriter) {
        w.write_bits(self.0 as u64, 20);
    }
    fn decode(r: &mut BitReader) -> Result<Self, DecodeError> {
        Ok(Mv(r.read_bits(20)? as u32))
    }
}

#[test]
fn decoders_never_panic_on_random_input() {
    let mut rng = Rng::new(0xf022);
    let mut ok = 0;
    for i in 0..200_000 {
        let mut d = random_bytes(&mut rng, 1300);
        if i % 2 == 0 && !d.is_empty() {
            d[0] = (rng.below(11)) as u8; // mostly valid kinds
        }
        if Packet::decode(&d).is_ok() {
            ok += 1;
        }
        let _ = DataBody::decode(&d);
        let mut r = BitReader::new(&d);
        let _ = InputReceiver::default().receive::<Mv>(&mut r);
        let mut r = BitReader::new(&d);
        let _ = r.read_varint();
        let _ = r.read_bytes(rng.below(2000) as usize);
        let _ = r.read_bits(rng.below(70) as u32);
    }
    assert!(ok > 1000, "some random inputs decode ({ok})");
}

#[test]
fn mutated_valid_bodies_never_panic() {
    let mut rng = Rng::new(77);
    let body = DataBody {
        ordered: vec![
            tt_net::packet::WireChunk { id: 7, last: false, data: vec![1; 384] },
            tt_net::packet::WireChunk { id: 8, last: true, data: vec![2; 10] },
        ],
        unordered: vec![tt_net::packet::WireEvent { id: 3, data: vec![3; 30] }],
        unreliable: vec![vec![4; 50], vec![]],
    };
    let enc = body.encode();
    assert_eq!(DataBody::decode(&enc).unwrap(), body);
    for _ in 0..100_000 {
        let m = mutate(&mut rng, &enc);
        let _ = DataBody::decode(&m);
    }
}

/// Builds a live server + client, records real traffic, then sprays random
/// and mutated datagrams at both endpoints from (a) a third address and
/// (b) spoofed peer addresses. Nothing panics; with (a) only, the connection
/// keeps delivering reliable data exactly once and in order.
#[test]
fn endpoints_survive_fuzzed_traffic() {
    let mut s = Sim::new(31, LinkConditions::lan());
    s.net.set_capture(true);
    s.add_client(addr(2, 5000), ClientConfig::default());
    s.connect_all();
    let conn = s.server_conns()[0];
    for i in 0..200u32 {
        let now = s.now();
        s.server.send(conn, Lane::Reliable, &payload(i, 900), now).unwrap();
        s.server.send(conn, Lane::Event, &payload(i, 50), now).unwrap();
        s.server.send(conn, Lane::Unreliable, &payload(i, 200), now).unwrap();
        s.clients[0].send(Lane::Reliable, &payload(i, 300), now).unwrap();
        s.clients[0].send(Lane::Unreliable, &payload(i, 20), now).unwrap();
        s.step(TICK);
    }
    let mut corpus: Vec<Vec<u8>> = s.net.take_captured().into_iter().map(|(_, _, d)| d).collect();
    corpus.push(Packet::InfoQuery { protocol: PROTOCOL_VERSION, nonce: 1 }.encode());
    s.net.set_capture(false);
    s.run(Duration::from_secs(1));
    let got_before = s.client_messages(0, Lane::Reliable);
    assert_eq!(got_before.len(), 200);

    let attacker = addr(99, 9999);
    let _attacker_ep = s.net.endpoint(attacker);
    let mut rng = Rng::new(0xbad);
    let mut k = 200u32;
    let mut expected_down = Vec::new();
    for round in 0..3000 {
        for _ in 0..10 {
            let d = if rng.chance(0.3) {
                random_bytes(&mut rng, 1300)
            } else {
                let src = &corpus[rng.below(corpus.len() as u64) as usize];
                mutate(&mut rng, src)
            };
            let to = if rng.chance(0.5) { server_addr() } else { addr(2, 5000) };
            s.net.inject(attacker, to, &d);
        }
        if round % 10 == 0 {
            let now = s.now();
            let p = payload(k, 500);
            s.server.send(conn, Lane::Reliable, &p, now).unwrap();
            expected_down.push(p);
            k += 1;
        }
        s.step(TICK);
    }
    s.run(Duration::from_secs(1));
    assert!(s.clients[0].is_connected());
    assert_eq!(s.server.connection_count(), 1);
    assert_eq!(s.client_messages(0, Lane::Reliable), expected_down);
    assert!(s.server.endpoint_stats().malformed > 1000);

    // Spoofed source addresses (on-path attacker without the session, or
    // with mutated real packets): no panic. The connection may legitimately
    // break here (there is no packet authentication in v1, §6).
    for _ in 0..20_000 {
        let src = &corpus[rng.below(corpus.len() as u64) as usize];
        let d = mutate(&mut rng, src);
        if rng.chance(0.5) {
            s.net.inject(addr(2, 5000), server_addr(), &d);
        } else {
            s.net.inject(server_addr(), addr(2, 5000), &d);
        }
        if rng.chance(0.1) {
            s.step(TICK);
        }
    }
    s.run(Duration::from_secs(1));
    let _ = s.server.connection_count();
}
