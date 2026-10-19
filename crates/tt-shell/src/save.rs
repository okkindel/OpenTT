//! The names of every file of the user data directory are here (docs/layout.md § User data);
//! tt-engine's `userdata` module moves an older layout into this one at start-up.
//!
//! | RON file | was | content |
//! |---|---|---|
//!
//! [`load`] reads a RON file when it exists. Solo scores and levels are plain numbers (players' best,
//! high scores, progress); files of earlier versions wrote them as strings ("083080"), which still read
//! ([`de_u64`]) and are written back as numbers on the next save.

use serde::{Deserialize, Serialize};

use crate::hiscores::{HiScore, HiScores};
use crate::players::Players;
use crate::progress::Progress;
use crate::settings::Settings;

pub const PLAYERS: &str = "players.ron";
pub const HISCORES: &str = "hiscores.ron";
pub const PROGRESS: &str = "progress.ron";
pub const SETTINGS: &str = "settings.ron";
/// The key bindings (`tt_gameplay::client::binds::Binds`, read by tt-engine).
pub const BINDS: &str = "binds.ron";
/// The server's ban list ([`crate::banlist::BanList`]).
pub const BANLIST: &str = "banlist.ron";
/// The console log of the last session.
pub const CONSOLE_LOG: &str = "logs/console.log";
/// Where `doScreenShot` saves.
pub const SCREENSHOTS: &str = "screenshots";
/// Files of an older layout kept as backups (never read by the game).
pub const LEGACY_DIR: &str = "legacy";

pub use crate::legacy::{LEGACY_BANLISTS, LEGACY_BINDS, LEGACY_CLIENT_PREFS, LEGACY_CONFIG, LEGACY_CONSOLE_LOG, LEGACY_HISCORES, LEGACY_PLAYERS, LEGACY_PROGRESS, LEGACY_SERVER_PREFS};

/// Pretty RON as the save files are written (`implicit_some`: optional
/// fields are written as plain values), with a comment line after the
/// extension header.
pub fn pretty<T: Serialize>(v: &T, comment: &str) -> String {
    let cfg = ron::ser::PrettyConfig::new().struct_names(true).extensions(ron::extensions::Extensions::IMPLICIT_SOME);
    let text = ron::ser::to_string_pretty(v, cfg).unwrap_or_default();
    match text.split_once('\n') {
        Some((header, rest)) if header.starts_with("#![") => format!("{header}\n// {comment}\n{rest}\n"),
        _ => format!("// {comment}\n{text}\n"),
    }
}

/// Where the saves are read and written (paths relative to the data dir).
pub trait SaveStore {
    fn read(&self, rel: &str) -> Option<Vec<u8>>;
    fn write(&mut self, rel: &str, bytes: &[u8]) -> std::io::Result<()>;
}

/// A plain directory.
pub struct DirStore(pub std::path::PathBuf);

impl SaveStore for DirStore {
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(rel)).ok()
    }
    fn write(&mut self, rel: &str, bytes: &[u8]) -> std::io::Result<()> {
        let p = self.0.join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, bytes)
    }
}

// ---- players ------------------------------------------------------------------------

/// One profile of `players.ron`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerRecord {
    pub name: String,
    /// Multiplayer kills and deaths (absent: never played).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kills: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deaths: Option<u32>,
    /// Multiplayer games counted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub games: Option<u32>,
    /// Best solo score.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "de_opt_u64")]
    pub solo_best: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayersFile {
    /// Index into `players` of the current one.
    pub last_selection: usize,
    pub players: Vec<PlayerRecord>,
}

/// A whole number written as a number, or as text by earlier versions
/// ("083080", "52000"; anything else reads as 0).
#[derive(Deserialize)]
#[serde(untagged)]
enum NumOrText {
    Num(f64),
    Text(String),
}

impl NumOrText {
    fn value(self) -> Option<f64> {
        let v = match self {
            NumOrText::Num(v) => v,
            NumOrText::Text(t) => t.trim().parse::<f64>().ok()?,
        };
        (v.is_finite() && v >= 0.0).then(|| v.round())
    }
}

/// [`NumOrText`] as a `u64` (serde `deserialize_with`).
pub fn de_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(NumOrText::deserialize(d)?.value().unwrap_or(0.0) as u64)
}

/// [`NumOrText`] as a `u32`.
pub fn de_u32<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    Ok(NumOrText::deserialize(d)?.value().unwrap_or(0.0).min(u32::MAX as f64) as u32)
}

/// An optional [`NumOrText`] (text that is no number: None).
pub fn de_opt_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(Option::<NumOrText>::deserialize(d)?.and_then(NumOrText::value).map(|v| v as u64))
}

/// A decoded stat as a count when it is one exactly as the game writes it.
fn count(text: &str) -> Option<u32> {
    let n: u32 = text.trim().parse::<f64>().ok().filter(|v| *v >= 0.0 && v.fract() == 0.0 && *v <= u32::MAX as f64)? as u32;
    (crate::vars::num(n as f64) == text).then_some(n)
}

impl PlayersFile {
    pub fn of(p: &Players) -> PlayersFile {
        let players = (0..p.count())
            .map(|i| {
                let combat = p.combat_data(i);
                let words: Vec<&str> = combat.split(' ').collect();
                let (kills, deaths) = match words[..] {
                    [k, d] => match (count(k), count(d)) {
                        (Some(k), Some(d)) => (Some(k), Some(d)),
                        _ => (None, None),
                    },
                    _ => (None, None),
                };
                PlayerRecord { name: p.name(i), kills, deaths, games: count(&p.combat_exp(i)), solo_best: p.solo_best(i) }
            })
            .collect();
        PlayersFile { last_selection: p.last_selection(), players }
    }

    pub fn players(&self) -> Players {
        let mut p = Players::from_records(self.last_selection, &self.players);
        p.prepare_save();
        p
    }
}

// ---- high scores --------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HiScoresFile {
    /// The ten entries, best first (empty: the defaults).
    pub entries: Vec<HiScore>,
}

impl HiScoresFile {
    pub fn of(h: &HiScores) -> HiScoresFile {
        HiScoresFile { entries: h.entries().unwrap_or_default() }
    }

    pub fn hiscores(&self) -> HiScores {
        let mut h = HiScores::default();
        if self.entries.len() == 10 {
            h.set_entries(&self.entries);
        }
        h
    }
}

// ---- loading and importing ----------------------------------------------------------

/// Everything the saves hold.
#[derive(Debug, Clone, Default)]
pub struct Saves {
    pub players: Players,
    pub hiscores: HiScores,
    pub progress: Progress,
    /// The saved prefs (None: no settings saved; the defaults apply).
    pub settings: Option<Settings>,
}

/// What [`load`] did besides reading RON files.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadReport {
    pub imported: Vec<(Vec<String>, String)>,
    /// Unreadable files (the defaults were used; nothing was overwritten).
    pub errors: Vec<String>,
}

impl LoadReport {
    /// One line per import / error, for the console log.
    pub fn lines(&self) -> Vec<String> {
        let mut v: Vec<String> = self.imported.iter().map(|(from, to)| format!("saves: imported {} into {to} (the old file stays as a backup)", from.join(" + "))).collect();
        v.extend(self.errors.iter().map(|e| format!("saves: {e}")));
        v
    }
}

/// One save: the RON file, else an import of the legacy files.
fn load_one<T>(
    store: &mut dyn SaveStore,
    report: &mut LoadReport,
    ron_file: &str,
    legacy: &[&str],
    parse: impl Fn(&str) -> Result<T, String>,
    import: impl Fn(&[Option<String>]) -> T,
    to_ron: impl Fn(&T) -> String,
) -> Option<T> {
    if let Some(b) = store.read(ron_file) {
        return match parse(&String::from_utf8_lossy(&b)) {
            Ok(v) => Some(v),
            Err(e) => {
                report.errors.push(format!("{ron_file}: {e}"));
                None
            }
        };
    }
    let texts: Vec<Option<String>> = legacy.iter().map(|f| store.read(f).map(|b| tt_game::fmt::latin1(&b))).collect();
    if texts.iter().all(Option::is_none) {
        return None;
    }
    let v = import(&texts);
    let text = to_ron(&v);
    match store.write(ron_file, text.as_bytes()) {
        Ok(()) => report.imported.push((legacy.iter().zip(&texts).filter(|(_, t)| t.is_some()).map(|(f, _)| f.to_string()).collect(), ron_file.to_string())),
        Err(e) => report.errors.push(format!("{ron_file}: {e}")),
    }
    // What the next start reads.
    Some(parse(&text).unwrap_or(v))
}

fn text(t: &[Option<String>], i: usize) -> &str {
    t.get(i).and_then(|t| t.as_deref()).unwrap_or("")
}

/// The players (with the level-select progress, which is per player).
pub fn load_players(store: &mut dyn SaveStore, report: &mut LoadReport) -> (Players, Progress) {
    let players = load_one(store, report, PLAYERS, &[LEGACY_PLAYERS], |s| ron::from_str::<PlayersFile>(s).map(|f| f.players()).map_err(|e| e.to_string()), |t| Players::from_legacy(text(t, 0)), save_players_text).unwrap_or_else(Players::new);
    let progress = load_one(store, report, PROGRESS, &[LEGACY_PROGRESS], |s| ron::from_str::<Progress>(s).map_err(|e| e.to_string()), |t| Progress::from_legacy(text(t, 0)), save_progress_text).unwrap_or_default();
    (players, progress)
}

pub fn load_hiscores(store: &mut dyn SaveStore, report: &mut LoadReport) -> HiScores {
    load_one(store, report, HISCORES, &[LEGACY_HISCORES], |s| ron::from_str::<HiScoresFile>(s).map(|f| f.hiscores()).map_err(|e| e.to_string()), |t| HiScores::from_legacy(text(t, 0)), save_hiscores_text).unwrap_or_default()
}

pub fn load_settings(store: &mut dyn SaveStore, report: &mut LoadReport) -> Option<Settings> {
    load_one(
        store,
        report,
        SETTINGS,
        &[LEGACY_CLIENT_PREFS, LEGACY_SERVER_PREFS],
        Settings::from_ron,
        |t| Settings::from_vars(&crate::legacy::import_prefs(text(t, 0), text(t, 1))),
        Settings::to_ron,
    )
}

/// Every save ([`Saves`]), importing what has no RON file yet.
pub fn load(store: &mut dyn SaveStore) -> (Saves, LoadReport) {
    let mut r = LoadReport::default();
    let (players, progress) = load_players(store, &mut r);
    let hiscores = load_hiscores(store, &mut r);
    let settings = load_settings(store, &mut r);
    (Saves { players, hiscores, progress, settings }, r)
}

/// [`load`] of a plain directory (tools, tests).
pub fn read_dir(dir: &std::path::Path) -> Saves {
    load(&mut DirStore(dir.to_path_buf())).0
}

pub fn save_players_text(p: &Players) -> String {
    pretty(&PlayersFile::of(p), &format!("{} players.", tt_assets::product::NAME))
}

pub fn save_hiscores_text(h: &HiScores) -> String {
    pretty(&HiScoresFile::of(h), &format!("{} solo high scores.", tt_assets::product::NAME))
}

pub fn save_progress_text(p: &Progress) -> String {
    pretty(p, &format!("{} level-select progress.", tt_assets::product::NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::Prefs;

    struct Mem(std::collections::BTreeMap<String, Vec<u8>>);

    impl SaveStore for Mem {
        fn read(&self, rel: &str) -> Option<Vec<u8>> {
            self.0.get(rel).cloned()
        }
        fn write(&mut self, rel: &str, bytes: &[u8]) -> std::io::Result<()> {
            self.0.insert(rel.to_string(), bytes.to_vec());
            Ok(())
        }
    }

    #[test]
    fn players_round_trip_through_ron() {
        let mut prefs = Prefs::defaults();
        let mut p = Players::new();
        p.create("Bait", &mut prefs).unwrap();
        p.create("Rook", &mut prefs).unwrap();
        p.record_mp_game(12.0, 5.0);
        p.record_mp_game(3.0, 0.0);
        p.record_solo_score(52000.0);
        p.select(0, &mut prefs).unwrap();
        p.prepare_save();
        let f = PlayersFile::of(&p);
        assert_eq!(f.players[1], PlayerRecord { name: "Rook".into(), kills: Some(15), deaths: Some(5), games: Some(2), solo_best: Some(52000) });
        assert_eq!(f.players[0], PlayerRecord { name: "Bait".into(), ..Default::default() });
        let q: PlayersFile = ron::from_str(&save_players_text(&p)).unwrap();
        let back = q.players();
        assert_eq!(back.vars.to_text(""), p.vars.to_text(""));
    }

    #[test]
    fn hiscores_round_trip_through_ron() {
        let mut h = HiScores::default();
        h.insert("Me", 200000.0, 9);
        let text = save_hiscores_text(&h);
        assert!(text.contains("score: 200000,") && text.contains("level: 9,"), "{text}");
        let q: HiScoresFile = ron::from_str(&text).unwrap();
        assert_eq!(q.hiscores(), h);
        assert_eq!(HiScoresFile::default().hiscores().entries(), None);
    }

    /// Files of earlier versions wrote the solo scores and levels as
    /// strings: they read as the same numbers and are written back plain.
    #[test]
    fn string_scores_of_older_saves_migrate() {
        let old = r#"#![enable(implicit_some)]
HiScoresFile(entries: [
    HiScore(name: "Jonny", score: "498720", level: "31"),
    HiScore(name: "Krevitz", score: "410010", level: "34"),
    HiScore(name: "Shark", score: "310220", level: "27"),
    HiScore(name: "Locke", score: "121700", level: "15"),
    HiScore(name: "Patton", score: "083080", level: "12"),
    HiScore(name: "Ribbit", score: "045070", level: "10"),
    HiScore(name: "Mack", score: "025030", level: "8"),
    HiScore(name: "Rascal", score: "018640", level: "8"),
    HiScore(name: "Artiz", score: "007320", level: "5"),
    HiScore(name: "Mike", score: "003170", level: "3"),
])"#;
        let h = ron::from_str::<HiScoresFile>(old).unwrap().hiscores();
        let mut d = HiScores::default();
        d.set_defaults();
        assert_eq!(h, d);
        assert!(save_hiscores_text(&h).contains("score: 83080,"));
        let old = r#"#![enable(implicit_some)]
PlayersFile(last_selection: 0, players: [PlayerRecord(name: "Bait", solo_best: "52000"), PlayerRecord(name: "Rook", kills: 1, deaths: 2, games: 1, solo_best: "---")])"#;
        let f: PlayersFile = ron::from_str(old).unwrap();
        assert_eq!(f.players[0].solo_best, Some(52000));
        assert_eq!(f.players[1].solo_best, None);
        let p = f.players();
        assert_eq!(p.solo_hiscore(0), "52000");
        assert!(save_players_text(&p).contains("solo_best: 52000"));
        let old = r#"#![enable(implicit_some)]
Progress(reached: [("Bait", 9)], starts: [HiScoreStart(name: "Bait", score: "52000", level: "9", start: 7)])"#;
        let q: Progress = ron::from_str(old).unwrap();
        assert_eq!((q.starts[0].score, q.starts[0].level), (52000, 9));
        assert!(save_progress_text(&q).contains("score: 52000,"));
    }

    #[test]
    fn import_once_and_keep_the_originals() {
        let mut prefs = Prefs::defaults();
        let mut p = Players::new();
        p.create("Bait", &mut prefs).unwrap();
        p.record_mp_game(2.0, 3.0);
        p.record_solo_score(75000.0);
        p.prepare_save();
        assert!(!p.to_legacy().contains("75000"));
        let mut h = HiScores::default();
        h.insert("Bait", 100000.0, 4);
        let cs = |t: String| t.into_bytes();
        let mut m = Mem(Default::default());
        m.0.insert(LEGACY_PLAYERS.into(), cs(p.to_legacy()));
        m.0.insert(LEGACY_HISCORES.into(), cs(h.to_legacy()));
        m.0.insert(LEGACY_CLIENT_PREFS.into(), cs(crate::legacy::prefs_file(&[("player.name", "Bait"), ("server.port", "28000")])));
        m.0.insert(LEGACY_SERVER_PREFS.into(), cs(crate::legacy::prefs_file(&[("server.port", "28005")])));
        let before = m.0.clone();
        let (s, r) = load(&mut m);
        assert_eq!(r.errors, Vec::<String>::new());
        assert_eq!(r.imported.len(), 3, "{:?}", r.imported);
        assert_eq!(s.players.vars.to_text(""), p.vars.to_text(""));
        assert_eq!(s.players.solo_best(0), Some(75000));
        assert_eq!(s.hiscores.entries(), h.entries());
        let st = s.settings.unwrap();
        assert_eq!((st.player.name.as_deref(), st.server.port), (Some("Bait"), Some(28005)));
        for (k, v) in &before {
            assert_eq!(m.0.get(k), Some(v), "{k} untouched");
        }
        // The second start reads the RON files only.
        m.0.insert(LEGACY_PLAYERS.into(), b"garbage".to_vec());
        let (s2, r2) = load(&mut m);
        assert!(r2.imported.is_empty() && r2.errors.is_empty());
        assert_eq!(s2.players.vars.to_text(""), p.vars.to_text(""));
    }
}
