//! Network administration and status (docs/archive/completeness.md WP-D): the ban
//! list (`BanList::add/addAbsolute/export`, checked on connect; saved as
//! `banlist.ron`, [`tt_shell::banlist`]), the ping list for the player
//! list's ping column, and the lag icon.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::host::EngineHost;
pub use tt_shell::banlist::{ip_of, Ban};
use tt_shell::banlist::BanList;

/// The ban list file of the data directory.
pub const BAN_FILE: &str = tt_shell::save::BANLIST;

/// Unix time (s) for ban expiry: the wall clock, or in a golden run a
/// fixed epoch plus the virtual clock (deterministic ban files).
pub fn unix_now(h: &EngineHost) -> i64 {
    if h.golden.is_some() {
        return 1_800_000_000 + (h.now_ms_f64() / 1000.0) as i64;
    }
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `BanList::add(guid, address, seconds)`.
pub fn add(h: &mut EngineHost, guid: &str, addr: &str, seconds: i64) {
    add_absolute(h, guid, addr, unix_now(h) + seconds.max(0));
}

pub fn add_absolute(h: &mut EngineHost, guid: &str, addr: &str, until: i64) {
    let ban = Ban { guid: guid.to_string(), ip: ip_of(addr), until };
    let bans = &mut h.netgame.bans;
    bans.retain(|b| !(b.ip == ban.ip && b.guid == ban.guid));
    bans.push(ban);
}

/// Is this address banned now?
pub fn is_banned(h: &EngineHost, ip: &str) -> bool {
    let now = unix_now(h);
    h.netgame.bans.iter().any(|b| b.until > now && b.ip == ip)
}

/// The live bans as `banlist.ron` text.
fn live_bans(h: &EngineHost) -> String {
    let now = unix_now(h);
    BanList { bans: h.netgame.bans.iter().filter(|b| b.until > now).cloned().collect() }.to_ron()
}

/// `BanList::export(file)`: the current bans ([`BanList`] RON).
pub fn export(h: &mut EngineHost, file: &str) {
    let text = live_bans(h);
    if let Err(e) = h.fs.write(file, text.as_bytes(), false) {
        h.log.warn(&format!("BanList::export {file}: {e}"));
    }
}

/// The game defines neither pref.
pub const KICK_BAN_TIME: &str = "300";
pub const BAN_TIME: &str = "1800";

/// Sets the ban-time prefs where unset (a server opens).
pub fn default_prefs(h: &mut EngineHost) {
    for (name, v) in [("server.kick_ban_time", KICK_BAN_TIME), ("server.ban_time", BAN_TIME)] {
        if h.shell.prefs.get(name).trim().is_empty() {
            h.shell.prefs.declare(name, tt_shell::VarValue::Str(v.to_string()));
        }
    }
}

/// Writes the live bans to [`BAN_FILE`] at once (`BanList::add` from the
/// scripts), so a ban survives a crash, not only the clean exit where the
/// scripts call `BanList::export`. Not in deterministic runs.
pub fn persist(h: &mut EngineHost) {
    if h.offline() {
        return;
    }
    // Never write over saved bans not read yet.
    load(h);
    let text = live_bans(h);
    if let Err(e) = h.fs.write(BAN_FILE, text.as_bytes(), false) {
        h.log.warn(&format!("ban list {BAN_FILE}: {e}"));
    }
}

/// Reads the saved ban list (once, when a server opens).
pub fn load(h: &mut EngineHost) {
    if h.netgame.bans_loaded {
        return;
    }
    h.netgame.bans_loaded = true;
    let Some(bytes) = h.fs.read_data(BAN_FILE) else { return };
    match BanList::from_ron(&String::from_utf8_lossy(&bytes)) {
        Ok(list) => {
            for b in list.bans {
                add_absolute(h, &b.guid, &b.ip, b.until);
            }
        }
        Err(e) => h.log.warn(&format!("ban list {BAN_FILE}: {e}")),
    }
}

/// The ping column of the client's player list: the first list switches the rows to
/// the network layout (a ping field before cumScore; after the K/D/A fields), then
/// every listed row gets its ping (`tt_gameplay::client::PlayerList::set_pings`).
pub fn apply_pings(h: &mut EngineHost, entries: &[(u32, u32)]) {
    h.client.players.set_pings(entries);
}
