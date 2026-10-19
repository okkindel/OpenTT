//! Shell tables: the tank list of tankSelectionGui.gui, the map list
//! (discovered from the mission files and `missions/catalog.ron`, which
//! replaced `createGameGui::getMissionDisplayName`'s table) and the Quick
//! Play map pool (`launchQuickServer`).

use crate::players::Players;
use crate::prefs::Prefs;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TankChoice {
    pub index: u8,
    pub name: &'static str,
    /// Under `models/tanks/` of the asset directory.
    pub model: &'static str,
    pub brain: &'static str,
    pub def: &'static str,
}

pub const TANKS: [TankChoice; 4] = [
    TankChoice { index: 0, name: "Light", model: "tank01.dts", brain: "brainjar01.dts", def: "LightTank" },
    TankChoice { index: 1, name: "Medium", model: "tank02.dts", brain: "brainjar02.dts", def: "MediumTank" },
    TankChoice { index: 2, name: "Heavy", model: "tank03.dts", brain: "brainjar03.dts", def: "HeavyTank" },
    TankChoice { index: 3, name: "Baby Boss", model: "tank05.dts", brain: "brainjar04.dts", def: "BabyBoss" },
];

pub fn baby_boss_available(players: &Players, prefs: &Prefs) -> bool {
    prefs.unlock_baby_boss() || players.baby_boss_unlocked()
}

/// A playable map of the menus (discovered: [`build`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionInfo {
    /// The mission id: its files are `missions/<id>.ron`, `.ter` and `.png`
    /// of the asset directory (or of a mod).
    pub id: String,
    /// `MissionInfo.name` (runs of spaces collapsed), else the id.
    pub name: String,
    /// 1 green, 2 blue, 3 red; 0 for the special maps (test arenas, maps
    /// of no world: never in Quick Play).
    pub world: u8,
    /// `MissionGroup.music_track`.
    pub music: String,
}

/// The menu's map list file (in the game data, and optionally in mods: all
/// of them are merged, [`build`]).
pub const CATALOG: &str = "missions/catalog.ron";

/// `missions/catalog.ron`: the order and world of the maps, and the mission
/// files that are no maps of the list (the solo campaign's, target practice).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CatalogFile {
    pub missions: Vec<CatalogEntry>,
    /// Mission ids left out of the list.
    pub hidden: Vec<String>,
}

/// A map of [`CatalogFile`]; unset fields come from the mission file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub id: String,
    #[serde(default)]
    pub world: Option<u8>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub music: Option<String>,
}

impl CatalogFile {
    pub fn parse(text: &str) -> Result<CatalogFile, String> {
        let opts = ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
        opts.from_str(text).map_err(|e| e.to_string())
    }
}

/// What a mission file (`missions/<id>.ron`) says about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MissionMeta {
    pub id: String,
    /// `MissionInfo.name`.
    pub name: String,
    /// `MissionGroup.music_track`.
    pub music: String,
    /// `MissionInfo.world` (a mod map's own field), if set.
    pub world: Option<u8>,
}

/// The world of a `TT<world>_<n>` id.
pub fn world_of_id(id: &str) -> Option<u8> {
    let rest = id.get(..2).filter(|p| p.eq_ignore_ascii_case("TT")).map(|_| &id[2..])?;
    let (w, n) = rest.split_once('_')?;
    let w: u8 = w.parse().ok()?;
    ((1..=3).contains(&w) && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).then_some(w)
}

/// The world's name in the menus.
pub fn world_label(world: u8) -> &'static str {
    match world {
        1 => "Green",
        2 => "Blue",
        3 => "Red",
        _ => "Special",
    }
}

/// The map list: the mission files found (`found`, every `missions/*.ron`
/// but the catalog) in the order of the catalogs (`catalogs`: the game
/// data's, then each enabled mod's; a later entry of the same id changes
/// the fields it sets, `hidden` lists add up), then the maps no catalog
/// lists by world (special ones last) and id. A map's world: its catalog
/// entry, else its own `MissionInfo.world`, else a `TT<world>_<n>` id, else
/// 0. Name and music: the catalog entry, else the mission file.
pub fn build(catalogs: &[CatalogFile], found: &[MissionMeta]) -> Vec<MissionInfo> {
    let mut entries: Vec<CatalogEntry> = Vec::new();
    let mut hidden: Vec<String> = Vec::new();
    for c in catalogs {
        for e in &c.missions {
            match entries.iter_mut().find(|x| x.id.eq_ignore_ascii_case(&e.id)) {
                Some(x) => {
                    x.world = e.world.or(x.world);
                    x.name = e.name.clone().or(x.name.take());
                    x.music = e.music.clone().or(x.music.take());
                }
                None => entries.push(e.clone()),
            }
        }
        hidden.extend(c.hidden.iter().cloned());
    }
    let is_hidden = |id: &str| hidden.iter().any(|h| h.eq_ignore_ascii_case(id));
    let info = |m: &MissionMeta, e: Option<&CatalogEntry>| {
        let collapse = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = e.and_then(|e| e.name.clone()).unwrap_or_else(|| collapse(&m.name));
        MissionInfo {
            id: m.id.clone(),
            name: if name.is_empty() { m.id.clone() } else { name },
            world: e.and_then(|e| e.world).or(m.world).or_else(|| world_of_id(&m.id)).unwrap_or(0),
            music: e.and_then(|e| e.music.clone()).unwrap_or_else(|| if m.music.is_empty() { "Lush".into() } else { m.music.clone() }),
        }
    };
    let mut out = Vec::new();
    for e in &entries {
        if let Some(m) = found.iter().find(|m| m.id.eq_ignore_ascii_case(&e.id)).filter(|m| !is_hidden(&m.id)) {
            out.push(info(m, Some(e)));
        }
    }
    let mut rest: Vec<MissionInfo> = found.iter().filter(|m| !is_hidden(&m.id) && !entries.iter().any(|e| e.id.eq_ignore_ascii_case(&m.id))).map(|m| info(m, None)).collect();
    rest.sort_by_key(|m| (m.world == 0, m.world, m.id.to_ascii_lowercase()));
    out.extend(rest);
    out
}

/// Quick Play: a random map of worlds 1–3 out of `missions`. One
/// `getRandom(count - 1)` draw with the engine RNG; `None` (no draw)
/// without such a map.
pub fn pick_quick_play_mission<'a>(missions: &'a [MissionInfo], rng: &mut tt_game::rng::Rng) -> Option<&'a MissionInfo> {
    let pool: Vec<&MissionInfo> = missions.iter().filter(|m| (1..=3).contains(&m.world)).collect();
    if pool.is_empty() {
        return None;
    }
    Some(pool[rng.rand_range(0, pool.len() as i32 - 1) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baby_boss_unlock() {
        let mut players = Players::new();
        let mut prefs = Prefs::defaults();
        players.create("Newbie", &mut prefs).unwrap();
        assert!(!players.baby_boss_unlocked());
        players.record_solo_score(500001.0);
        assert!(players.baby_boss_unlocked());
    }

    fn meta(id: &str, name: &str, world: Option<u8>) -> MissionMeta {
        MissionMeta { id: id.into(), name: name.into(), music: "Spooky".into(), world }
    }

    #[test]
    fn quick_play_pool() {
        // Every multiplayer world, whatever the player's experience.
        let found = [meta("tt1_1", "a", None), meta("tt2_1", "b", None), meta("tt3_1", "c", None), meta("Arena", "Test", None)];
        let missions = build(&[], &found);
        let mut rng = tt_game::rng::Rng::new(5);
        let mut worlds = std::collections::BTreeSet::new();
        for _ in 0..200 {
            let m = pick_quick_play_mission(&missions, &mut rng).unwrap();
            assert!((1..=3).contains(&m.world), "{}", m.name);
            worlds.insert(m.world);
        }
        assert_eq!(worlds.len(), 3);
        assert!(pick_quick_play_mission(&missions[3..], &mut rng).is_none());
    }

    #[test]
    fn map_list_from_catalogs_and_files() {
        let game = CatalogFile::parse("(missions: [(id: \"tt1_2\", world: 1), (id: \"tt1_1\"), (id: \"Arena\", world: 0), (id: \"Gone\", world: 2)], hidden: [\"Boss\"])").unwrap();
        let modded = CatalogFile::parse("(missions: [(id: \"tt1_1\", name: \"Renamed\"), (id: \"ModB\", world: 3)], hidden: [\"tt1_2\"])").unwrap();
        let found = [
            meta("Arena", "Test  Arena", None),
            meta("Boss", "", None),
            meta("ModA", "Mod map A", Some(2)),
            meta("ModB", "Mod map B", None),
            meta("ModC", "", None),
            meta("tt1_1", "First", None),
            meta("tt1_2", "Second", None),
            meta("TT9_1", "Not a world", None),
        ];
        let only_game = build(std::slice::from_ref(&game), &found);
        let ids: Vec<(&str, u8)> = only_game.iter().map(|m| (m.id.as_str(), m.world)).collect();
        assert_eq!(ids, [("tt1_2", 1), ("tt1_1", 1), ("Arena", 0), ("ModA", 2), ("ModB", 0), ("ModC", 0), ("TT9_1", 0)]);
        assert_eq!(only_game[2].name, "Test Arena");
        assert_eq!(only_game[5].name, "ModC");
        let both = build(&[game, modded], &found);
        let ids: Vec<(&str, u8, &str)> = both.iter().map(|m| (m.id.as_str(), m.world, m.name.as_str())).collect();
        assert_eq!(&ids[..3], [("tt1_1", 1, "Renamed"), ("Arena", 0, "Test Arena"), ("ModB", 3, "Mod map B")]);
        assert_eq!(both[0].music, "Spooky");
        assert!(CatalogFile::parse("(maps: [])").is_err());
    }
}
