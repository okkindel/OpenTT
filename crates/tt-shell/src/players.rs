//! Player profiles and their stats.
//!
//! The best solo score (`hiscore<i>`) is a plain number.

use tt_game::obfuscation::{strfrap_decode, strfrip, strswiz};

use crate::prefs::Prefs;
use crate::vars::{atof, get_word, num, word_count, VarFile, VarValue};
use crate::IDLE_TEXT;

pub const MAX_PLAYERS: usize = 6;
/// `getSolohiscore(sel) > 500000` unlocks the Baby Boss tank.
pub const BABY_BOSS_SCORE: f64 = 500000.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerError {
    /// Six players already exist.
    Full,
    /// The name is empty after sanitising.
    EmptyName,
    /// Deleting would leave no player.
    LastPlayer,
    NoSuchPlayer,
}

/// What the player screen shows for one profile ("---" when unknown).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerStats {
    pub name: String,
    /// `getCombatRatio`: kills/deaths as "D.dd" (or "K/0", "---").
    pub combat_ratio: String,
    /// `getCombatExp`: multiplayer games played.
    pub combat_exp: String,
    /// `getSolohiscore`: best solo score.
    pub solo_hiscore: String,
}

pub use tt_game::fmt::str_to_player_name;

/// Engine `stripTrailingSpaces`.
pub fn strip_trailing_spaces(s: &str) -> String {
    s.trim_end_matches([' ', '_']).to_string()
}

/// Is never assigned, so the key depends on the slot index only.
pub fn slot_key(idx: usize) -> String {
    strswiz(&format!("{idx}{IDLE_TEXT}"), "12")
}

/// `getPlayerData(%swiz, %code)`.
pub fn player_data(key: &str, code: &str) -> String {
    strfrap_decode(key, code).unwrap_or_else(|| "---".into())
}

/// Script `mFloor(x)` of a float expression: the argument is passed as a
/// `%g` string, the result is an S32.
fn m_floor(v: f64) -> i64 {
    atof(&tt_game::fmt::format_g(v)).floor() as i64
}

/// `getCombatRatio` formatting of a decoded "kills deaths" pair.
pub fn format_combat_ratio(data: &str) -> String {
    if word_count(data) != 2 {
        return "---".into();
    }
    let kills = get_word(data, 0);
    let deaths = get_word(data, 1);
    let (k, d) = (atof(&kills), atof(&deaths));
    if d < 1.0 {
        return format!("{kills}/0");
    }
    let digits = m_floor(k / d);
    let dec = m_floor(100.0 * (k / d - digits as f64));
    let dec = if dec == 0 { "00".to_string() } else { dec.to_string() };
    format!("{digits}.{dec}")
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Players {
    pub vars: VarFile,
}

const P: &str = "players.";

fn var(field: &str, i: usize) -> String {
    format!("{P}{field}{i}")
}

impl Players {
    /// An empty list as `loadPlayerGui::show` initialises it.
    pub fn new() -> Players {
        let mut p = Players::default();
        p.normalize();
        p
    }

    /// An old players file ([`crate::legacy`]): the obfuscated best solo
    /// scores decoded.
    pub fn from_legacy(src: &str) -> Players {
        let mut p = Players { vars: crate::legacy::parse(src) };
        p.normalize();
        for i in 0..MAX_PLAYERS {
            let best = player_data(&slot_key(i), &p.vars.get(&var("hiscore", i)));
            let plain = if best == "---" { String::new() } else { num(crate::hiscores::whole(&best)) };
            if !p.vars.get(&var("hiscore", i)).is_empty() {
                p.vars.set(&var("hiscore", i), &plain);
            }
        }
        p
    }

    /// The variables of `players.ron`'s profiles ([`crate::save::PlayersFile`]):
    /// the stats encoded with each slot's key, as the game keeps them.
    pub fn from_records(last_selection: usize, records: &[crate::save::PlayerRecord]) -> Players {
        let mut p = Players::default();
        let n = records.len().min(MAX_PLAYERS);
        p.vars.set_value(&format!("{P}count"), VarValue::Int(n as i32));
        p.vars.set(&format!("{P}lastSelection"), &last_selection.min(n.saturating_sub(1)).to_string());
        for (i, r) in records.iter().take(n).enumerate() {
            let key = slot_key(i);
            p.vars.set(&var("name", i), &r.name);
            if let (Some(k), Some(d)) = (r.kills, r.deaths) {
                p.vars.set(&var("cratio", i), &strfrip(&key, &format!("{} {}", num(k as f64), num(d as f64))));
            }
            if let Some(g) = r.games {
                p.vars.set(&var("cmissions", i), &strfrip(&key, &num(g as f64)));
            }
            if let Some(h) = r.solo_best {
                p.vars.set(&var("hiscore", i), &h.to_string());
            }
        }
        p
    }

    pub fn to_legacy(&self) -> String {
        let mut vars = self.vars.clone();
        for i in 0..MAX_PLAYERS {
            let best = vars.get(&var("hiscore", i));
            if !best.is_empty() {
                vars.set(&var("hiscore", i), &strfrip(&slot_key(i), &best));
            }
        }
        crate::legacy::export(&vars, P)
    }

    /// `loadPlayerGui::show` defaults: count 0, lastSelection 0.
    fn normalize(&mut self) {
        if self.vars.get(&format!("{P}count")).is_empty() {
            self.vars.set_value(&format!("{P}count"), VarValue::Int(0));
        }
        let ls = self.vars.get(&format!("{P}lastSelection"));
        if ls.is_empty() || atof(&ls) == -1.0 {
            self.vars.set_value(&format!("{P}lastSelection"), VarValue::Int(0));
        }
    }

    pub fn count(&self) -> usize {
        (self.vars.get_f64(&format!("{P}count")).max(0.0) as usize).min(MAX_PLAYERS)
    }

    pub fn last_selection(&self) -> usize {
        self.vars.get_f64(&format!("{P}lastSelection")).max(0.0) as usize
    }

    pub fn name(&self, i: usize) -> String {
        self.vars.get(&var("name", i))
    }

    pub fn names(&self) -> Vec<String> {
        (0..self.count()).map(|i| self.name(i)).collect()
    }

    /// `getCombatData(i)`: decoded "kills deaths" or "---".
    pub fn combat_data(&self, i: usize) -> String {
        player_data(&slot_key(i), &self.vars.get(&var("cratio", i)))
    }

    pub fn combat_ratio(&self, i: usize) -> String {
        format_combat_ratio(&self.combat_data(i))
    }

    /// `getCombatExp(i)` (also picks the Quick Play map pool).
    pub fn combat_exp(&self, i: usize) -> String {
        player_data(&slot_key(i), &self.vars.get(&var("cmissions", i)))
    }

    /// `getSolohiscore(i)`: the best solo score, "---" when none.
    pub fn solo_hiscore(&self, i: usize) -> String {
        self.solo_best(i).map_or_else(|| "---".into(), |v| v.to_string())
    }

    /// The best solo score, if any.
    pub fn solo_best(&self, i: usize) -> Option<u64> {
        let t = self.vars.get(&var("hiscore", i));
        let v = t.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)?;
        Some(v.round() as u64)
    }

    pub fn stats(&self, i: usize) -> PlayerStats {
        PlayerStats { name: self.name(i), combat_ratio: self.combat_ratio(i), combat_exp: self.combat_exp(i), solo_hiscore: self.solo_hiscore(i) }
    }

    /// Baby Boss is selectable for the current player.
    pub fn baby_boss_unlocked(&self) -> bool {
        self.solo_best(self.last_selection()).is_some_and(|v| v as f64 > BABY_BOSS_SCORE)
    }

    /// `loadPlayerList::update`: re-sanitise the listed names.
    fn update_list(&mut self) {
        for i in 0..self.count() {
            let n = strip_trailing_spaces(&str_to_player_name(&self.name(i)));
            self.vars.set(&var("name", i), &n);
        }
    }

    pub fn select(&mut self, i: usize, prefs: &mut Prefs) -> Result<(), PlayerError> {
        if i >= self.count() {
            return Err(PlayerError::NoSuchPlayer);
        }
        self.vars.set(&format!("{P}lastSelection"), &i.to_string());
        let name = self.name(i);
        if !name.is_empty() {
            prefs.set_player_name(&name);
        }
        Ok(())
    }

    /// `loadPlayerGui::createPlayer` (and the New button's six-player
    /// limit), then selects the new player. Returns its slot.
    pub fn create(&mut self, name: &str, prefs: &mut Prefs) -> Result<usize, PlayerError> {
        self.normalize();
        if self.count() >= MAX_PLAYERS {
            return Err(PlayerError::Full);
        }
        let name = strip_trailing_spaces(&str_to_player_name(name));
        if name.is_empty() {
            return Err(PlayerError::EmptyName);
        }
        let i = self.count();
        self.vars.set_value(&format!("{P}count"), VarValue::Int(i as i32 + 1));
        self.vars.set(&var("name", i), &name);
        self.update_list();
        self.select(i, prefs)?;
        Ok(i)
    }

    /// `loadPlayerDeleteButton` + `loadPlayerGui::deletePlayer`: refuses with
    /// fewer than two players; shifts the following slots down and re-encodes
    /// their stats with the new slot keys (unknown stats are re-encoded as
    /// "---", exactly as the script does). The old last slot keeps its
    /// variables until [`Players::prepare_save`]. Re-selects `lastSelection`
    /// (clamped). Save afterwards.
    pub fn delete(&mut self, id: usize, prefs: &mut Prefs) -> Result<(), PlayerError> {
        let count = self.count();
        if count < 2 {
            return Err(PlayerError::LastPlayer);
        }
        if id >= count {
            return Err(PlayerError::NoSuchPlayer);
        }
        for i in id..count - 1 {
            let next = self.name(i + 1);
            self.vars.set(&var("name", i), &next);
            let cdata = self.combat_data(i + 1);
            let kills = get_word(&cdata, 0);
            let deaths = get_word(&cdata, 1);
            let n = self.combat_exp(i + 1);
            let hi = self.vars.get(&var("hiscore", i + 1));
            let key = slot_key(i);
            self.vars.set(&var("cratio", i), &strfrip(&key, &format!("{kills} {deaths}")));
            self.vars.set(&var("cmissions", i), &strfrip(&key, &n));
            self.vars.set(&var("hiscore", i), &hi);
        }
        let count = count - 1;
        self.vars.set_value(&format!("{P}count"), VarValue::Int(count as i32));
        if self.last_selection() > count - 1 {
            self.vars.set_value(&format!("{P}lastSelection"), VarValue::Int(count as i32 - 1));
        }
        self.update_list();
        let sel = self.last_selection();
        self.select(sel, prefs)
    }

    /// `loadPlayerGui::hide` before its export: clears the unused slots'
    /// name/cmissions/hiscore.
    pub fn prepare_save(&mut self) {
        for i in 0..MAX_PLAYERS {
            if i as i64 > self.count() as i64 - 1 {
                self.vars.set(&var("name", i), "");
                self.vars.set(&var("cmissions", i), "");
                self.vars.set(&var("hiscore", i), "");
            }
        }
    }

    /// For a counted multiplayer game (Quick Play, deathmatch): adds
    /// the kills/deaths and one game to the current player. Save
    /// afterwards.
    pub fn record_mp_game(&mut self, kills: f64, deaths: f64) {
        let idx = self.last_selection();
        let data = self.combat_data(idx);
        let kills = kills + atof(&get_word(&data, 0));
        let deaths = deaths + atof(&get_word(&data, 1));
        let n = atof(&self.combat_exp(idx)) + 1.0;
        let key = slot_key(idx);
        self.vars.set(&var("cratio", idx), &strfrip(&key, &format!("{} {}", num(kills), num(deaths))));
        self.vars.set(&var("cmissions", idx), &strfrip(&key, &num(n)));
    }

    /// On game over: stores `score` as the current player's best when
    /// higher. Returns true when it changed (then save).
    pub fn record_solo_score(&mut self, score: f64) -> bool {
        let idx = self.last_selection();
        let score = if score.is_finite() && score > 0.0 { score.round() as u64 } else { 0 };
        if score > self.solo_best(idx).unwrap_or(0) {
            self.vars.set(&var("hiscore", idx), &score.to_string());
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sanitised() {
        assert_eq!(str_to_player_name("  Jo.hn__  Doe, the 3rd!!!!!!!"), "John_Doe the 3rd");
        assert_eq!(strip_trailing_spaces("ab _ "), "ab");
        assert_eq!(slot_key(0), strswiz("0refer", "12"));
    }

    #[test]
    fn combat_ratio_formatting() {
        assert_eq!(format_combat_ratio("---"), "---");
        assert_eq!(format_combat_ratio("--- "), "---");
        assert_eq!(format_combat_ratio("7 0"), "7/0");
        assert_eq!(format_combat_ratio("6 3"), "2.00");
        assert_eq!(format_combat_ratio("2 3"), "0.66");
        // The script does not zero-pad: 1.05 shows as "1.5".
        assert_eq!(format_combat_ratio("21 20"), "1.5");
    }
}
