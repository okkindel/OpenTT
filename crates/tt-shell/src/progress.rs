
use crate::hiscores::HiScore;
use crate::vars::{VarFile, VarValue};

const P: &str = "progress.";

/// A high-score entry's start position.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HiScoreStart {
    pub name: String,
    pub score: u64,
    pub level: u32,
    pub start: u32,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Progress {
    /// (player name, highest position reached), in file order.
    pub reached: Vec<(String, u32)>,
    pub starts: Vec<HiScoreStart>,
}

impl Progress {
    /// An old progress file ([`crate::legacy`]).
    pub fn from_legacy(src: &str) -> Progress {
        let v = crate::legacy::parse(src);
        let n = |k: &str| v.get_f64(&format!("{P}{k}")).max(0.0) as usize;
        let mut p = Progress::default();
        for i in 0..n("count").min(4096) {
            let name = v.get(&format!("{P}name{i}"));
            let level = v.get_f64(&format!("{P}level{i}")).max(0.0) as u32;
            if !name.is_empty() && level > 0 {
                p.reached.push((name, level));
            }
        }
        for i in 0..n("hsCount").min(64) {
            let start = v.get_f64(&format!("{P}hsStart{i}")).max(0.0) as u32;
            let num = |k: &str| crate::hiscores::whole(&v.get(&format!("{P}{k}{i}")));
            let e = HiScoreStart { name: v.get(&format!("{P}hsName{i}")), score: num("hsScore") as u64, level: num("hsLevel").min(u32::MAX as f64) as u32, start };
            if start > 1 {
                p.starts.push(e);
            }
        }
        p
    }

    /// The progress as an old progress file ([`crate::legacy`]).
    pub fn to_legacy(&self) -> String {
        let mut v = VarFile::new();
        v.set_value(&format!("{P}count"), VarValue::Int(self.reached.len() as i32));
        for (i, (name, level)) in self.reached.iter().enumerate() {
            v.set(&format!("{P}name{i}"), name);
            v.set_value(&format!("{P}level{i}"), VarValue::Int(*level as i32));
        }
        v.set_value(&format!("{P}hsCount"), VarValue::Int(self.starts.len() as i32));
        for (i, e) in self.starts.iter().enumerate() {
            v.set(&format!("{P}hsName{i}"), &e.name);
            v.set(&format!("{P}hsScore{i}"), &e.score.to_string());
            v.set(&format!("{P}hsLevel{i}"), &e.level.to_string());
            v.set_value(&format!("{P}hsStart{i}"), VarValue::Int(e.start as i32));
        }
        crate::legacy::export(&v, P)
    }

    /// Highest position `player` reached (1 when never played: the first
    /// level is always open).
    pub fn highest(&self, player: &str) -> u32 {
        self.reached.iter().find(|(n, _)| n.eq_ignore_ascii_case(player)).map_or(1, |(_, l)| (*l).max(1))
    }

    /// `player` started position `position`; true when that is a new best
    /// (save afterwards).
    pub fn record_reached(&mut self, player: &str, position: u32) -> bool {
        if player.is_empty() {
            return false;
        }
        match self.reached.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(player)) {
            Some((_, l)) if *l >= position => false,
            Some((_, l)) => {
                *l = position;
                true
            }
            None => {
                self.reached.push((player.to_string(), position));
                true
            }
        }
    }

    /// The player's profile was deleted.
    pub fn forget(&mut self, player: &str) {
        self.reached.retain(|(n, _)| !n.eq_ignore_ascii_case(player));
    }

    /// A finished run entered the top 10: remember its start when it was
    /// not the first level, and drop the starts of entries that left the
    /// table (`table` = the entries after the insertion).
    pub fn record_hiscore(&mut self, entry: Option<&HiScore>, start: u32, table: &[HiScore]) {
        if let Some(e) = entry.filter(|_| start > 1) {
            self.starts.push(HiScoreStart { name: e.name.clone(), score: e.score, level: e.level, start });
        }
        // One start per table entry (equal entries pair up in order).
        let mut kept = Vec::new();
        let mut pool = std::mem::take(&mut self.starts);
        for t in table {
            if let Some(i) = pool.iter().rposition(|s| s.name == t.name && s.score == t.score && s.level == t.level) {
                kept.push(pool.remove(i));
            }
        }
        self.starts = kept;
    }

    /// Start positions of the table's entries (None: the first level).
    pub fn hiscore_starts(&self, table: &[HiScore]) -> Vec<Option<u32>> {
        let mut pool: Vec<&HiScoreStart> = self.starts.iter().collect();
        table
            .iter()
            .map(|t| {
                let i = pool.iter().position(|s| s.name == t.name && s.score == t.score && s.level == t.level)?;
                Some(pool.remove(i).start)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hs(name: &str, score: u64, level: u32) -> HiScore {
        HiScore { name: name.into(), score, level }
    }

    #[test]
    fn round_trip_and_best() {
        let mut p = Progress::default();
        assert_eq!(p.highest("Bait"), 1);
        assert!(p.record_reached("Bait", 3));
        assert!(!p.record_reached("bait", 2));
        assert!(p.record_reached("Rook \"the\" Tank", 9));
        let table = vec![hs("Bait", 52000, 9), hs("Jonny", 498720, 31)];
        p.record_hiscore(Some(&table[0]), 7, &table);
        let q = Progress::from_legacy(&p.to_legacy());
        assert_eq!(q, p);
        assert_eq!((q.highest("BAIT"), q.highest("Rook \"the\" Tank"), q.highest("x")), (3, 9, 1));
        assert_eq!(q.hiscore_starts(&table), vec![Some(7), None]);
        // The entry falls out of the table: its start goes too.
        let mut q = q;
        q.record_hiscore(None, 1, &table[1..]);
        assert!(q.starts.is_empty());
        q.forget("bait");
        assert_eq!(q.highest("Bait"), 1);
    }
}
