//! The solo top 10.

use tt_game::obfuscation::{strfrap_decode, strfrip, strswiz};

use crate::vars::{atof, VarFile};
use crate::IDLE_TEXT;

pub const DEFAULTS: [(&str, u64, u32); 10] = [
    ("Jonny", 498720, 31),
    ("Krevitz", 410010, 34),
    ("Shark", 310220, 27),
    ("Locke", 121700, 15),
    ("Patton", 83080, 12),
    ("Ribbit", 45070, 10),
    ("Mack", 25030, 8),
    ("Rascal", 18640, 8),
    ("Artiz", 7320, 5),
    ("Mike", 3170, 3),
];

/// One entry: plain numbers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HiScore {
    pub name: String,
    pub score: u64,
    /// At the end of the run.
    pub level: u32,
}

fn name_key(i: usize) -> String {
    strswiz(&format!("toad {i}{IDLE_TEXT}"), "12")
}

fn score_key(i: usize, name: &str) -> String {
    strswiz(&format!("{i}{name}{IDLE_TEXT}"), "12")
}

fn level_key(i: usize, score: &str) -> String {
    strswiz(&format!("{i}{score}{IDLE_TEXT}"), "12")
}

pub(crate) fn whole(text: &str) -> f64 {
    let v = atof(text).round();
    if v.is_finite() && v > 0.0 {
        v
    } else {
        0.0
    }
}

/// The solo top 10, best first (empty until set: the defaults apply).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HiScores {
    list: Vec<HiScore>,
}

const P: &str = "hiscores.";

impl HiScores {
    /// An old high-score file ([`crate::legacy`]): its obfuscated entries
    /// decoded (an entry failing the `strfrap` check leaves the
    /// table unset, as the game then reset it to the defaults).
    pub fn from_legacy(src: &str) -> HiScores {
        let vars = crate::legacy::parse(src);
        let entry = |i: usize| -> Option<HiScore> {
            let field = |f: &str, k: &str| {
                let code = vars.get(&format!("{P}{f}{i}"));
                strfrap_decode(k, &code)
            };
            let name = field("name", &name_key(i))?;
            let score = field("score", &score_key(i, &name))?;
            let level = field("level", &level_key(i, &score))?;
            Some(HiScore { name, score: whole(&score) as u64, level: whole(&level).min(u32::MAX as f64) as u32 })
        };
        HiScores { list: (0..10).map(entry).collect::<Option<Vec<_>>>().unwrap_or_default() }
    }

    pub fn to_legacy(&self) -> String {
        let mut vars = VarFile::new();
        for (i, e) in self.list.iter().enumerate() {
            let score = e.score.to_string();
            vars.set(&format!("{P}name{i}"), &strfrip(&name_key(i), &e.name));
            vars.set(&format!("{P}score{i}"), &strfrip(&score_key(i, &e.name), &score));
            vars.set(&format!("{P}level{i}"), &strfrip(&level_key(i, &score), &e.level.to_string()));
        }
        crate::legacy::export(&vars, P)
    }

    /// Entry `i`, if the table is set.
    pub fn entry(&self, i: usize) -> Option<HiScore> {
        self.entries()?.get(i).cloned()
    }

    /// All ten entries, or None when the table is not set.
    pub fn entries(&self) -> Option<Vec<HiScore>> {
        (self.list.len() == 10).then(|| self.list.clone())
    }

    /// Sets the table to `list` (the first ten; fewer leave it unset).
    pub fn set_entries(&mut self, list: &[HiScore]) {
        self.list = list.iter().take(10).cloned().collect();
    }

    /// `soloHiScoreList::setDefaultHiScores` (save afterwards).
    pub fn set_defaults(&mut self) {
        self.list = DEFAULTS.iter().map(|(n, s, l)| HiScore { name: n.to_string(), score: *s, level: *l }).collect();
    }

    /// The table, resetting it to the defaults first when it is not set.
    pub fn entries_or_defaults(&mut self) -> Vec<HiScore> {
        if self.entries().is_none() {
            self.set_defaults();
        }
        self.list.clone()
    }

    /// `soloPlayGui::endGame` table update: inserts the finished run before
    /// the first lower score. Returns the rank (0-based) or None. Save
    /// afterwards.
    pub fn insert(&mut self, name: &str, score: f64, level: u32) -> Option<usize> {
        let mut list = self.entries_or_defaults();
        let mine = (0..10).find(|&i| score > list[i].score as f64);
        if let Some(i) = mine {
            list.insert(i, HiScore { name: name.to_string(), score: whole(&score.to_string()) as u64, level });
            list.truncate(10);
        }
        self.list = list;
        mine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_insert() {
        let mut h = HiScores::default();
        assert!(h.entries().is_none());
        let e = h.entries_or_defaults();
        assert_eq!(e[4], HiScore { name: "Patton".into(), score: 83080, level: 12 });
        let rank = h.insert("Me", 200000.0, 9);
        assert_eq!(rank, Some(3));
        let e = h.entries().unwrap();
        assert_eq!(e[3].name, "Me");
        assert_eq!(e[4].name, "Locke");
        assert_eq!(e[9].name, "Artiz");
        assert_eq!(h.insert("Low", 10.0, 1), None);
    }

    #[test]
    fn legacy_file_round_trip() {
        let mut h = HiScores::default();
        h.set_defaults();
        h.insert("Me", 1_234_567.0, 40);
        let text = h.to_legacy();
        assert!(!text.contains("1234567") && !text.contains("Jonny"), "{text}");
        assert_eq!(HiScores::from_legacy(&text), h);
        assert_eq!(h.entry(0).unwrap().score, 1_234_567);
        // A tampered entry leaves the table unset (the defaults then apply).
        let i = text.find("score3").unwrap();
        let q = i + text[i..].find('"').unwrap() + 1;
        let bad = format!("{}0{}", &text[..q], &text[q..]);
        assert_eq!(HiScores::from_legacy(&bad).entries(), None);
    }
}
