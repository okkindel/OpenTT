//! The server's ban list: `banlist.ron` in the data directory (docs/layout.md
//! § User data; older ban lists are imported once, [`crate::legacy`]).

use serde::{Deserialize, Serialize};

/// One ban: the GUID, the IP (without port) and the end (Unix seconds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ban {
    pub guid: String,
    pub ip: String,
    pub until: i64,
}

/// `banlist.ron`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename = "BanList", default)]
pub struct BanList {
    pub bans: Vec<Ban>,
}

/// The IP part of `IP:a.b.c.d:port`, `a.b.c.d:port`, `[::1]:port` or a bare IP.
pub fn ip_of(addr: &str) -> String {
    let a = addr.trim().trim_matches('"');
    let a = a.strip_prefix("IP:").or_else(|| a.strip_prefix("ip:")).unwrap_or(a);
    if let Ok(s) = a.parse::<std::net::SocketAddr>() {
        return s.ip().to_string();
    }
    match a.rsplit_once(':') {
        Some((ip, port)) if port.parse::<u16>().is_ok() && !ip.contains(':') => ip.to_string(),
        _ => a.trim_start_matches('[').trim_end_matches(']').to_string(),
    }
}

impl BanList {
    /// Adds a ban, replacing one of the same GUID and IP.
    pub fn add(&mut self, ban: Ban) {
        self.bans.retain(|b| !(b.ip == ban.ip && b.guid == ban.guid));
        self.bans.push(ban);
    }

    pub fn to_ron(&self) -> String {
        crate::save::pretty(self, &format!("{} server ban list (until: Unix time in seconds).", tt_assets::product::NAME))
    }

    pub fn from_ron(src: &str) -> Result<BanList, String> {
        ron::from_str(src).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ips() {
        assert_eq!(ip_of("IP:10.0.0.2:5000"), "10.0.0.2");
        assert_eq!(ip_of("10.0.0.2:5000"), "10.0.0.2");
        assert_eq!(ip_of("10.0.0.2"), "10.0.0.2");
        assert_eq!(ip_of("[::1]:7"), "::1");
        assert_eq!(ip_of("\"IP:1.2.3.4:0\""), "1.2.3.4");
    }

    #[test]
    fn add_and_ron_round_trip() {
        let mut b = BanList::default();
        b.add(Ban { guid: "0".into(), ip: "local".into(), until: 1800001825 });
        b.add(Ban { guid: "1234".into(), ip: "10.0.0.7".into(), until: 1800000100 });
        b.add(Ban { guid: "1234".into(), ip: "10.0.0.7".into(), until: 1800009999 });
        assert_eq!(b.bans, vec![Ban { guid: "0".into(), ip: "local".into(), until: 1800001825 }, Ban { guid: "1234".into(), ip: "10.0.0.7".into(), until: 1800009999 }]);
        assert_eq!(BanList::from_ron(&b.to_ron()).unwrap(), b);
        assert_eq!(BanList::from_ron(&BanList::default().to_ron()).unwrap(), BanList::default());
    }
}
