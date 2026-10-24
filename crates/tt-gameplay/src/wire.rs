//! Bit codec of [`ServerMsg`] and [`ClientCmd`] (one kind byte, then the
//! fields with `tt_net::msg`'s texts: integers as varints, tagged texts by
//! number once the connection has seen them).

use crate::msg::{ClientCmd, DebugCmd, HudMsg, MsgKind, ServerMsg, Value};
use tt_net::bits::{BitReader, BitWriter};
use tt_net::msg::{get_count, put_text, TextRx, TextTx};
use tt_net::DecodeError;

fn put_value(w: &mut BitWriter, tx: &mut TextTx, v: &Value) {
    match v {
        Value::Str(s) => put_text(w, s),
        Value::Tag(s) => tx.put(w, s),
    }
}

fn get_value(r: &mut BitReader, rx: &mut TextRx) -> Result<Value, DecodeError> {
    Ok(match rx.get(r)? {
        (true, s) => Value::Tag(s),
        (false, s) => Value::Str(s),
    })
}

fn put_values(w: &mut BitWriter, tx: &mut TextTx, v: &[Value]) {
    w.write_varint(v.len() as u64);
    for x in v {
        put_value(w, tx, x);
    }
}

fn get_values(r: &mut BitReader, rx: &mut TextRx) -> Result<Vec<Value>, DecodeError> {
    let n = get_count(r)?;
    (0..n).map(|_| get_value(r, rx)).collect()
}

const HUD_KINDS: &[&str] = &["clockShowTime", "clockSetTime", "clockVisible", "brainsMax", "brainsCount", "brainsVisible", "score", "lives", "sound", "targetPracticeDone", "soloGameOver"];

const DEBUG_CMDS: &[DebugCmd] = &[DebugCmd::ToggleCamera, DebugCmd::DropPlayerAtCamera, DebugCmd::DropCameraAtPlayer, DebugCmd::SelectObject, DebugCmd::CycleTank, DebugCmd::TogglePlayerAI];

/// Writes `m` (its tagged texts through the connection's `tx`).
pub fn put_server_msg(w: &mut BitWriter, tx: &mut TextTx, m: &ServerMsg) {
    use ServerMsg::*;
    let t = put_text;
    match m {
        Message { kind, text } => {
            w.write_bits(0, 8);
            let k = match kind {
                MsgKind::ClientJoin { .. } => 0,
                MsgKind::ClientDrop { .. } => 1,
                MsgKind::ClientKilled { .. } => 2,
                MsgKind::ScoreChanged { .. } => 3,
                MsgKind::TeamScoreChanged { .. } => 4,
                MsgKind::AddTeam { .. } => 5,
                MsgKind::LoadInfo { .. } => 6,
                MsgKind::LoadInfoDone => 7,
                MsgKind::ConnectionError { .. } => 8,
                MsgKind::AdminForce { .. } => 9,
                MsgKind::Other { .. } => 10,
            };
            w.write_bits(k, 8);
            put_value(w, tx, text);
            match kind {
                MsgKind::ClientJoin { name, id, team, guid, score, cum, ai, admin, super_admin } => {
                    put_value(w, tx, name);
                    for s in [id, team, guid, score, cum, ai, admin, super_admin] {
                        t(w, s);
                    }
                }
                MsgKind::ClientDrop { name, id } => {
                    put_value(w, tx, name);
                    t(w, id);
                }
                MsgKind::ClientKilled { victim, killer } => {
                    put_value(w, tx, victim);
                    put_value(w, tx, killer);
                }
                MsgKind::ScoreChanged { score, cum, id: x } | MsgKind::TeamScoreChanged { score, cum, team: x } => {
                    for s in [score, cum, x] {
                        t(w, s);
                    }
                }
                MsgKind::AddTeam { team, name, score, cum } => {
                    t(w, team);
                    put_value(w, tx, name);
                    t(w, score);
                    t(w, cum);
                }
                MsgKind::LoadInfo { name: v } | MsgKind::ConnectionError { text: v } | MsgKind::AdminForce { who: v } => put_value(w, tx, v),
                MsgKind::LoadInfoDone => {}
                MsgKind::Other { kind, args } => {
                    put_value(w, tx, kind);
                    put_values(w, tx, args);
                }
            }
        }
        Chat { from, voice, pitch, text, args } => {
            w.write_bits(1, 8);
            t(w, from);
            put_value(w, tx, voice);
            t(w, pitch);
            put_value(w, tx, text);
            put_values(w, tx, args);
        }
        CenterPrint { text, secs, lines, lc } | BottomPrint { text, secs, lines, lc } => {
            let center = matches!(m, CenterPrint { .. });
            w.write_bits(match (center, lc) {
                (true, false) => 2,
                (false, false) => 3,
                (true, true) => 16,
                (false, true) => 17,
            }, 8);
            put_value(w, tx, text);
            t(w, secs);
            t(w, lines);
        }
        ClearCenterPrint => w.write_bits(4, 8),
        ClearBottomPrint => w.write_bits(5, 8),
        SyncClock { secs, dir, lc } => {
            w.write_bits(if *lc { 18 } else { 6 }, 8);
            t(w, secs);
            t(w, dir);
        }
        MissionStart { seq } => {
            w.write_bits(7, 8);
            t(w, seq);
        }
        MissionEnd { seq } => {
            w.write_bits(8, 8);
            t(w, seq);
        }
        MissionPhase1 { seq, file, music } => {
            w.write_bits(9, 8);
            for s in [seq, file, music] {
                t(w, s);
            }
        }
        MissionPhase2 { seq, file } | MissionPhase3 { seq, file } => {
            w.write_bits(if matches!(m, MissionPhase2 { .. }) { 10 } else { 11 }, 8);
            t(w, seq);
            t(w, file);
        }
        GameStart => w.write_bits(12, 8),
        GameEnd { stats } => {
            w.write_bits(13, 8);
            t(w, stats);
        }
        Hud(h) => {
            w.write_bits(14, 8);
            let (kind, args) = h.encode();
            w.write_bits(HUD_KINDS.iter().position(|k| *k == kind).unwrap_or(0) as u64, 8);
            if let Some(a) = args.first() {
                t(w, a);
            }
        }
        VoteStatus { map, yes, no, players, secs, mine } => {
            w.write_bits(19, 8);
            t(w, map);
            for n in [yes, no, players, secs] {
                w.write_varint(*n as u64);
            }
            w.write_bits(mine.map_or(0, |y| 1 + y as u64), 8);
        }
        MapList { maps } => {
            w.write_bits(20, 8);
            w.write_varint(maps.len() as u64);
            for (id, name) in maps {
                t(w, id);
                t(w, name);
            }
        }
    }
}

/// Reads a message written by [`put_server_msg`].
pub fn get_server_msg(r: &mut BitReader, rx: &mut TextRx) -> Result<ServerMsg, DecodeError> {
    use ServerMsg::*;
    let kind = r.read_bits(8)?;
    Ok(match kind {
        0 => {
            let k = r.read_bits(8)?;
            let text = get_value(r, rx)?;
            let kind = match k {
                0 => {
                    let name = get_value(r, rx)?;
                    let mut v = Vec::with_capacity(8);
                    for _ in 0..8 {
                        v.push(rx.get_text(r)?);
                    }
                    let mut v = v.into_iter();
                    let mut n = || v.next().unwrap_or_default();
                    MsgKind::ClientJoin { name, id: n(), team: n(), guid: n(), score: n(), cum: n(), ai: n(), admin: n(), super_admin: n() }
                }
                1 => MsgKind::ClientDrop { name: get_value(r, rx)?, id: rx.get_text(r)? },
                2 => MsgKind::ClientKilled { victim: get_value(r, rx)?, killer: get_value(r, rx)? },
                3 => MsgKind::ScoreChanged { score: rx.get_text(r)?, cum: rx.get_text(r)?, id: rx.get_text(r)? },
                4 => MsgKind::TeamScoreChanged { score: rx.get_text(r)?, cum: rx.get_text(r)?, team: rx.get_text(r)? },
                5 => MsgKind::AddTeam { team: rx.get_text(r)?, name: get_value(r, rx)?, score: rx.get_text(r)?, cum: rx.get_text(r)? },
                6 => MsgKind::LoadInfo { name: get_value(r, rx)? },
                7 => MsgKind::LoadInfoDone,
                8 => MsgKind::ConnectionError { text: get_value(r, rx)? },
                9 => MsgKind::AdminForce { who: get_value(r, rx)? },
                10 => MsgKind::Other { kind: get_value(r, rx)?, args: get_values(r, rx)? },
                _ => return Err(DecodeError::Invalid("message kind")),
            };
            Message { kind, text }
        }
        1 => Chat { from: rx.get_text(r)?, voice: get_value(r, rx)?, pitch: rx.get_text(r)?, text: get_value(r, rx)?, args: get_values(r, rx)? },
        2 | 3 | 16 | 17 => {
            let (text, secs, lines) = (get_value(r, rx)?, rx.get_text(r)?, rx.get_text(r)?);
            let lc = kind >= 16;
            if kind == 2 || kind == 16 {
                CenterPrint { text, secs, lines, lc }
            } else {
                BottomPrint { text, secs, lines, lc }
            }
        }
        4 => ClearCenterPrint,
        5 => ClearBottomPrint,
        6 | 18 => SyncClock { secs: rx.get_text(r)?, dir: rx.get_text(r)?, lc: kind == 18 },
        7 => MissionStart { seq: rx.get_text(r)? },
        8 => MissionEnd { seq: rx.get_text(r)? },
        9 => MissionPhase1 { seq: rx.get_text(r)?, file: rx.get_text(r)?, music: rx.get_text(r)? },
        10 => MissionPhase2 { seq: rx.get_text(r)?, file: rx.get_text(r)? },
        11 => MissionPhase3 { seq: rx.get_text(r)?, file: rx.get_text(r)? },
        12 => GameStart,
        13 => GameEnd { stats: rx.get_text(r)? },
        14 => {
            let k = *HUD_KINDS.get(r.read_bits(8)? as usize).ok_or(DecodeError::Invalid("hud kind"))?;
            let args = if matches!(k, "targetPracticeDone" | "soloGameOver") { vec![] } else { vec![rx.get_text(r)?] };
            Hud(HudMsg::decode(k, &args).ok_or(DecodeError::Invalid("hud kind"))?)
        }
        19 => {
            let map = rx.get_text(r)?;
            let mut n = [0u32; 4];
            for x in &mut n {
                *x = r.read_varint()?.min(u32::MAX as u64) as u32;
            }
            let mine = match r.read_bits(8)? {
                0 => None,
                v => Some(v == 2),
            };
            VoteStatus { map, yes: n[0], no: n[1], players: n[2], secs: n[3], mine }
        }
        20 => {
            let n = get_count(r)?;
            MapList { maps: (0..n).map(|_| Ok((rx.get_text(r)?, rx.get_text(r)?))).collect::<Result<_, DecodeError>>()? }
        }
        _ => return Err(DecodeError::Invalid("server message kind")),
    })
}

/// Writes `c` (plain texts only: client strings are never tags).
pub fn put_client_cmd(w: &mut BitWriter, c: &ClientCmd) {
    use ClientCmd::*;
    match c {
        PhaseAck { phase, seq } => {
            w.write_bits((*phase).clamp(1, 3) as u64 - 1, 8);
            put_text(w, seq);
        }
        SpawnPlayer => w.write_bits(3, 8),
        MessageSent { text } | TeamMessageSent { text } => {
            w.write_bits(if matches!(c, MessageSent { .. }) { 4 } else { 5 }, 8);
            put_text(w, text);
        }
        Debug { cmd, args } => {
            w.write_bits(8 + DEBUG_CMDS.iter().position(|d| d == cmd).unwrap_or(0) as u64, 8);
            w.write_varint(args.len() as u64);
            for a in args {
                put_text(w, a);
            }
        }
        CallVote { map } => {
            w.write_bits(20, 8);
            put_text(w, map);
        }
        CastVote { yes } => w.write_bits(21 + *yes as u64, 8),
        MapList => w.write_bits(23, 8),
    }
}

/// Reads a command written by [`put_client_cmd`].
pub fn get_client_cmd(r: &mut BitReader) -> Result<ClientCmd, DecodeError> {
    use ClientCmd::*;
    let mut rx = TextRx::default();
    let k = r.read_bits(8)?;
    Ok(match k {
        0..=2 => PhaseAck { phase: k as u8 + 1, seq: rx.get_text(r)? },
        3 => SpawnPlayer,
        4 => MessageSent { text: rx.get_text(r)? },
        5 => TeamMessageSent { text: rx.get_text(r)? },
        20 => CallVote { map: rx.get_text(r)? },
        21 | 22 => CastVote { yes: k == 22 },
        23 => MapList,
        _ => {
            let cmd = *(k as usize).checked_sub(8).and_then(|i| DEBUG_CMDS.get(i)).ok_or(DecodeError::Invalid("client command kind"))?;
            let n = get_count(r)?;
            let args = (0..n).map(|_| rx.get_text(r)).collect::<Result<_, _>>()?;
            Debug { cmd, args }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::samples;

    #[test]
    fn server_messages_round_trip_on_one_connection() {
        let mut tx = TextTx::default();
        let mut rx = TextRx::default();
        // Twice: the second time every tag goes by number.
        let mut sizes = Vec::new();
        for _ in 0..2 {
            let mut total = 0;
            for m in samples() {
                let mut w = BitWriter::new();
                put_server_msg(&mut w, &mut tx, &m);
                let b = w.finish();
                total += b.len();
                let mut r = BitReader::new(&b);
                assert_eq!(get_server_msg(&mut r, &mut rx).unwrap(), m);
                assert_eq!(r.remaining_bits(), 0, "{m:?}");
            }
            sizes.push(total);
        }
        assert!(sizes[1] < sizes[0]);
    }

    #[test]
    fn client_commands_round_trip() {
        let all = [
            ClientCmd::PhaseAck { phase: 3, seq: "12".into() },
            ClientCmd::SpawnPlayer,
            ClientCmd::MessageSent { text: "hello".into() },
            ClientCmd::TeamMessageSent { text: "t".into() },
            ClientCmd::Debug { cmd: DebugCmd::TogglePlayerAI, args: vec!["1".into()] },
            ClientCmd::CallVote { map: "Snowblind".into() },
            ClientCmd::CastVote { yes: false },
            ClientCmd::CastVote { yes: true },
            ClientCmd::MapList,
        ];
        for c in all {
            let mut w = BitWriter::new();
            put_client_cmd(&mut w, &c);
            let b = w.finish();
            assert_eq!(get_client_cmd(&mut BitReader::new(&b)).unwrap(), c);
        }
        assert!(get_client_cmd(&mut BitReader::new(&[200])).is_err());
        // The admin login's old kinds: refused (in a debug build too).
        assert!(get_client_cmd(&mut BitReader::new(&[6])).is_err());
        assert!(get_client_cmd(&mut BitReader::new(&[7])).is_err());
        assert!(get_client_cmd(&mut BitReader::new(&[])).is_err());
    }

    #[test]
    fn garbage_never_panics() {
        let mut seed = 0x1234_5678_u64;
        for len in 0..400 {
            let b: Vec<u8> = (0..len % 40)
                .map(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    (seed >> 56) as u8
                })
                .collect();
            let mut rx = TextRx::default();
            let _ = get_server_msg(&mut BitReader::new(&b), &mut rx);
            let _ = get_client_cmd(&mut BitReader::new(&b));
        }
    }
}
