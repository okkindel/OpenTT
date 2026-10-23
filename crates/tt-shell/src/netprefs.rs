//! Typed accessors for the network prefs the Online menu edits.

use crate::prefs::Prefs;
use crate::vars::atof;

/// Recent join addresses kept.
pub const RECENT_SERVERS: usize = 8;

impl Prefs {
    fn num_pref(&self, name: &str, default: f64) -> f64 {
        let v = self.get(name);
        if v.trim().is_empty() {
            default
        } else {
            atof(&v)
        }
    }

    /// The LAN list shows it.
    pub fn server_name(&self) -> String {
        self.get("server.name")
    }

    pub fn set_server_name(&mut self, v: &str) {
        self.set("server.name", v);
    }

    pub fn server_port(&self) -> u16 {
        self.num_pref("server.port", 28000.0).clamp(1.0, 65535.0) as u16
    }

    pub fn set_server_port(&mut self, v: u16) {
        self.set("server.port", &v.to_string());
    }

    /// The most players a hosted server takes (OpenTT's default is 16).
    pub fn server_max_players(&self) -> u32 {
        self.num_pref("server.max_players", 16.0).clamp(1.0, 64.0) as u32
    }

    pub fn set_server_max_players(&mut self, v: u32) {
        self.set("server.max_players", &v.to_string());
    }

    /// empty: none.
    pub fn server_password(&self) -> String {
        self.get("server.password")
    }

    pub fn set_server_password(&mut self, v: &str) {
        self.set("server.password", v);
    }

    /// The map last hosted. Settings imported from the game hold its path
    /// (`game/data/missions/TT2_1.mis`): read as the asset path.
    pub fn host_mission(&self) -> String {
        let v = self.get("host.mission");
        tt_assets::layout::migrate_mission(&v).unwrap_or(v)
    }

    pub fn set_host_mission(&mut self, v: &str) {
        self.set("host.mission", v);
    }

    /// The mode last hosted.
    pub fn host_mode(&self) -> u32 {
        self.num_pref("host.mode", 0.0).clamp(0.0, 3.0) as u32
    }

    pub fn set_host_mode(&mut self, v: u32) {
        self.set("host.mode", &v.to_string());
    }

    /// The round length last hosted, minutes.
    pub fn host_minutes(&self) -> u32 {
        self.num_pref("host.minutes", 15.0).clamp(1.0, 60.0) as u32
    }

    pub fn set_host_minutes(&mut self, v: u32) {
        self.set("host.minutes", &v.to_string());
    }

    /// The maps a server plays in turn (mission ids; the console, the Host
    /// screen and a hand-written list may separate them by tabs, spaces or
    /// commas).
    pub fn map_rotation(&self) -> Vec<String> {
        self.get("server.map_rotation").split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(String::from).collect()
    }

    pub fn set_map_rotation(&mut self, ids: &[String]) {
        self.set("server.map_rotation", &ids.join("\t"));
    }

    /// The rotation's maps come in a random order (default off: in turn).
    pub fn map_rotation_random(&self) -> bool {
        self.num_pref("server.map_rotation_random", 0.0) != 0.0
    }

    pub fn set_map_rotation_random(&mut self, on: bool) {
        self.set("server.map_rotation_random", if on { "1" } else { "0" });
    }

    /// Some host settings were saved.
    pub fn has_host_settings(&self) -> bool {
        !self.host_mission().trim().is_empty()
    }

    /// The master server (`host[:port]`) for the internet list and for
    /// listing hosted games; empty = none.
    pub fn master_server(&self) -> String {
        self.get("net.master_server").trim().to_string()
    }

    pub fn set_master_server(&mut self, v: &str) {
        self.set("net.master_server", v.trim());
    }

    /// Adds each of [`tt_net::master::DEFAULT_MASTER_SERVERS`] not offered
    /// before to the master list (run after the settings are loaded): a new
    /// install gets them all, an existing one the defaults added since; a
    /// default the user removed stays removed.
    pub fn add_default_masters(&mut self) {
        self.offer_masters(tt_net::master::DEFAULT_MASTER_SERVERS);
    }

    /// [`Self::add_default_masters`] of `defaults`; `net.default_masters_added`
    /// lists those offered already. Entries compare case-insensitively, the
    /// default port implied.
    fn offer_masters(&mut self, defaults: &[&str]) {
        let key = |s: &str| {
            let s = s.trim().to_ascii_lowercase();
            let s = s.strip_prefix("ip:").unwrap_or(&s);
            s.strip_suffix(&format!(":{}", tt_net::master::DEFAULT_MASTER_PORT)).unwrap_or(s).to_string()
        };
        let mut offered = self.get("net.default_masters_added");
        let mut list = self.master_server();
        let before = offered.clone();
        for d in defaults {
            if offered.split(',').any(|o| key(o) == key(d)) {
                continue;
            }
            if !list.split(',').any(|e| key(e) == key(d)) {
                list = if list.is_empty() { d.to_string() } else { format!("{list}, {d}") };
            }
            offered = if offered.is_empty() { d.to_string() } else { format!("{offered}, {d}") };
        }
        if offered != before {
            self.set_master_server(&list);
            self.set("net.default_masters_added", &offered);
        }
    }

    /// Ask the router to forward the server port (UPnP / NAT-PMP) while
    /// hosting. Default off.
    pub fn port_mapping(&self) -> bool {
        self.num_pref("net.port_mapping", 1.0) != 0.0
    }

    pub fn set_port_mapping(&mut self, on: bool) {
        self.set("net.port_mapping", if on { "1" } else { "0" });
    }

    /// "Always" by default, "Never".
    pub fn display_on_master(&self) -> bool {
        !self.get("server.display_on_master").trim().eq_ignore_ascii_case("Never")
    }

    pub fn set_display_on_master(&mut self, on: bool) {
        self.set("server.display_on_master", if on { "Always" } else { "Never" });
    }

    /// tab-separated, newest first.
    pub fn recent_servers(&self) -> Vec<String> {
        self.get("net.recent_servers").split('\t').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
    }

    /// Puts `addr` first (once), keeping [`RECENT_SERVERS`].
    pub fn push_recent_server(&mut self, addr: &str) {
        let addr = addr.trim();
        if addr.is_empty() {
            return;
        }
        let mut v = self.recent_servers();
        v.retain(|a| !a.eq_ignore_ascii_case(addr));
        v.insert(0, addr.to_string());
        v.truncate(RECENT_SERVERS);
        self.set("net.recent_servers", &v.join("\t"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_prefs_and_recent_servers() {
        let mut p = Prefs::defaults();
        assert_eq!((p.server_port(), p.server_max_players(), p.host_minutes()), (28000, 16, 15));
        assert!(!p.has_host_settings());
        p.set_server_name("Bait's game");
        p.set_server_port(28005);
        p.set_server_max_players(8);
        p.set_server_password("secret");
        p.set_host_mission("missions/tt2_1.ron");
        p.set_host_mode(3);
        p.set_host_minutes(10);
        for i in 0..10 {
            p.push_recent_server(&format!("10.0.0.{i}"));
        }
        p.push_recent_server("10.0.0.5");
        let q = Prefs::with_settings(&crate::Settings::from_ron(&p.settings().to_ron()).unwrap());
        assert_eq!(q.server_name(), "Bait's game");
        assert_eq!((q.server_port(), q.server_max_players(), q.host_mode(), q.host_minutes()), (28005, 8, 3, 10));
        assert_eq!(q.server_password(), "secret");
        assert!(q.has_host_settings());
        let r = q.recent_servers();
        assert_eq!(r.len(), RECENT_SERVERS);
        assert_eq!(r[0], "10.0.0.5");
        assert_eq!(r[1], "10.0.0.9");
    }

    /// `server.map_rotation` in settings.ron: a list of ids, kept as it
    /// was; a hand-typed text with spaces or commas reads the same.
    #[test]
    fn map_rotation_round_trip() {
        let mut p = Prefs::defaults();
        assert!(p.map_rotation().is_empty());
        let ids: Vec<String> = ["tt1_1", "tt2_3", "moon_1"].map(String::from).to_vec();
        p.set_map_rotation(&ids);
        let ron = p.settings().to_ron();
        assert!(ron.contains("map_rotation") && ron.contains("\"moon_1\""), "{ron}");
        let s = crate::Settings::from_ron(&ron).unwrap();
        assert_eq!(s.server.map_rotation.as_deref(), Some(&ids[..]));
        assert_eq!(Prefs::with_settings(&s).map_rotation(), ids);
        p.set("server.map_rotation", "tt1_1, tt2_3  moon_1");
        assert_eq!(p.map_rotation(), ids);
    }

    /// `server.map_rotation_random`: off by default, a bool in settings.ron.
    #[test]
    fn map_rotation_random_round_trip() {
        let mut p = Prefs::defaults();
        assert!(!p.map_rotation_random());
        assert!(!p.settings().to_ron().contains("map_rotation_random"), "not written while unset");
        p.set_map_rotation_random(true);
        let ron = p.settings().to_ron();
        assert!(ron.contains("map_rotation_random: true"), "{ron}");
        let s = crate::Settings::from_ron(&ron).unwrap();
        assert_eq!(s.server.map_rotation_random, Some(true));
        assert!(Prefs::with_settings(&s).map_rotation_random());
        p.set_map_rotation_random(false);
        assert!(!Prefs::with_settings(&crate::Settings::from_ron(&p.settings().to_ron()).unwrap()).map_rotation_random());
    }

    /// Each default master is offered once per install: added to the list
    /// (new, empty or custom), never twice, not again after a removal; a
    /// later default reaches existing installs alone.
    #[test]
    fn default_masters_are_offered_once() {
        let saved = |p: &Prefs| Prefs::with_settings(&crate::Settings::from_ron(&p.settings().to_ron()).unwrap());
        let mut p = Prefs::defaults();
        p.add_default_masters();
        assert_eq!(p.master_server(), tt_net::master::DEFAULT_MASTER_SERVERS.join(", "), "a new install");
        assert_eq!(p.get("net.default_masters_added"), p.master_server());

        let mut p = Prefs::defaults();
        p.set_master_server("");
        p.offer_masters(&["a"]);
        assert_eq!(p.master_server(), "a", "an empty list");

        let mut p = Prefs::defaults();
        p.set_master_server("my.master:9");
        p.offer_masters(&["a"]);
        assert_eq!(p.master_server(), "my.master:9, a", "a custom list");
        // Removed after the migration: stays removed, also across a save.
        p.set_master_server("my.master:9");
        let mut q = saved(&p);
        q.offer_masters(&["a"]);
        assert_eq!(q.master_server(), "my.master:9");
        assert_eq!(q.get("net.default_masters_added"), "a");

        let mut p = Prefs::defaults();
        p.set_master_server("IP:A:28100, b");
        p.offer_masters(&["a"]);
        assert_eq!(p.master_server(), "IP:A:28100, b", "already listed");

        // Migrated with ["a"], "a" removed, a later version adds "b": only "b".
        let mut p = Prefs::defaults();
        p.offer_masters(&["a"]);
        p.set_master_server("");
        let mut q = saved(&p);
        q.offer_masters(&["a", "b"]);
        assert_eq!(q.master_server(), "b");
        assert_eq!(q.get("net.default_masters_added"), "a, b");
    }

    #[test]
    fn host_mission_imported_from_the_original() {
        let mut p = Prefs::defaults();
        p.set_host_mission("game/data/missions/TT2_1.mis");
        assert_eq!(p.host_mission(), "missions/tt2_1.ron");
        p.set_host_mission("missions/tt3_2.ron");
        assert_eq!(p.host_mission(), "missions/tt3_2.ron");
    }
}
