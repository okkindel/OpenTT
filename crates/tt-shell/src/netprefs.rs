
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

    /// OpenTT's default is 16, netcode §1.
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

    /// The map last hosted. Saves of older builds hold the game's path
    /// (`game/data/missions/TT2_1.mis`) or the mixed-case file name of the
    /// time before lowercase asset names (`missions/TT2_1.ron`): read as the
    /// new one.
    pub fn host_mission(&self) -> String {
        let v = self.get("host.mission");
        if let Some(m) = tt_assets::layout::migrate_mission(&v) {
            return m;
        }
        let in_missions = v.get(..9).is_some_and(|d| d.eq_ignore_ascii_case("missions/")) && !v[9..].contains('/');
        if in_missions && v.to_ascii_lowercase().ends_with(".ron") {
            tt_assets::layout::mission(&tt_assets::layout::mission_id(&v))
        } else {
            v
        }
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

    /// Ask the router to forward the server port (UPnP / NAT-PMP) while
    /// hosting. Default off.
    pub fn port_mapping(&self) -> bool {
        self.num_pref("net.port_mapping", 0.0) != 0.0
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

    #[test]
    fn host_mission_of_an_older_save() {
        let mut p = Prefs::defaults();
        p.set_host_mission("game/data/missions/TT2_1.mis");
        assert_eq!(p.host_mission(), "missions/tt2_1.ron");
        p.set_host_mission("missions/tt3_2.ron");
        assert_eq!(p.host_mission(), "missions/tt3_2.ron");
        // The mixed-case name of an older build.
        p.set_host_mission(&format!("missions/{}.ron", "TT3_2"));
        assert_eq!(p.host_mission(), "missions/tt3_2.ron");
    }
}
