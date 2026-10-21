//! Server side of LAN discovery: what a server that accepts connections answers to
//! `InfoQuery` packets.
//!
//! The reply comes from the server's session (the game mode label, the
//! info line of platform, world and mission name, the player list of team
//! letters and names, the scores), the settings (server name, maximum
//! players, the password flag), the player and bot counts, whether the
//! server is dedicated, and the CPU speed. [`refresh`] rebuilds it every
//! tick (cheap) and hands it to tt-net, which answers statelessly.

use std::sync::OnceLock;

use crate::host::EngineHost;
use tt_net::ServerInfo;

/// The score of every client, then every bot: the order the server's
/// player list names them in.
fn scores(h: &EngineHost) -> Vec<i16> {
    let v = h.srv.get().map(|s| s.scores()).unwrap_or_default();
    v.into_iter().map(|s| s.unwrap_or(0.0).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16).collect()
}

/// Random per process: tells replies of the same server apart from others.
fn process_id() -> u64 {
    static ID: OnceLock<u64> = OnceLock::new();
    *ID.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u32(std::process::id());
        h.finish()
    })
}

/// The CPU clock in MHz: the fastest core of `/proc/cpuinfo`, else a
/// nominal 2000.
pub fn cpu_mhz() -> u16 {
    static MHZ: OnceLock<u16> = OnceLock::new();
    *MHZ.get_or_init(|| {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .filter(|l| l.starts_with("cpu MHz"))
                    .filter_map(|l| l.split(':').nth(1)?.trim().parse::<f64>().ok())
                    .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))))
            })
            .map(|v| v.clamp(1.0, 65535.0) as u16)
            .unwrap_or(2000)
    })
}

/// This server instance's random id (`ServerInfo::server_id`, the master registration).
pub fn server_id(h: &EngineHost) -> u64 {
    let id = h.netgame.server.as_ref().map(|s| s.local_addr().port() as u64).unwrap_or(0);
    process_id() ^ id.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1
}

/// The info this server reports now.
pub fn server_info(h: &EngineHost) -> ServerInfo {
    let (humans, bots) = h.srv.get().map(|s| s.counts()).unwrap_or_default();
    let session = h.srv.get().map(|s| s.session.clone()).unwrap_or_default();
    let dedicated = h.srv.get().is_some_and(|s| s.setup.dedicated);
    let prefs = &h.shell.prefs;
    ServerInfo {
        server_id: server_id(h),
        build: h.netgame.build_override.unwrap_or(tt_net::BUILD_ID),
        game_type: tt_net::GAME_TYPE.to_string(),
        name: prefs.get("server.name"),
        mission_type: session.mode_label,
        info: session.info,
        mission_file: session.mission_file,
        players: session.player_list.clone(),
        player_count: (humans + bots).min(255) as u8,
        max_players: (tt_game::fmt::atoi(&prefs.get("server.max_players")) as i64).clamp(0, 255) as u8,
        bot_count: bots.min(255) as u8,
        password: !prefs.get("server.password").is_empty(),
        dedicated,
        cpu_mhz: cpu_mhz(),
        modded: false,
        mods: crate::mods::gameplay_refs(h),
        version: tt_net::GAME_VERSION.to_string(),
        protocols: (tt_net::PROTOCOL_VERSION, tt_net::PROTOCOL_VERSION),
        scores: {
            // Only when the list matches the groups (it is rebuilt on joins and drops).
            let s = scores(h);
            let list = &session.player_list;
            let listed = if list.is_empty() { 0 } else { list.split('\t').count().div_ceil(2) };
            if s.len() == listed {
                s
            } else {
                Vec::new()
            }
        },
    }
}

/// Sets the `Info` reply of the open server (none while connections are
/// not allowed, so a closed server stays invisible).
pub fn refresh(h: &mut EngineHost) {
    if h.netgame.server.is_none() {
        return;
    }
    let payload = h.netgame.accepting.then(|| server_info(h).encode());
    if let Some(s) = h.netgame.server.as_mut() {
        s.set_info(payload);
    }
}
