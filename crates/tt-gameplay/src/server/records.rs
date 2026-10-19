//! The gameplay records of clients and teams: scores, the solo run's
//! counters, a bot's slot, name and AI. Unset fields are `None`; the
//! round summaries list the set ones ([`ClientRecord::trace_fields`]).

use tt_game::fmt::format_g;

/// A client's (or bot client's) record.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClientRecord {
    pub score: Option<f64>,
    /// The score over the server's rounds.
    pub cum_score: Option<f64>,
    pub kills: Option<i32>,
    pub deaths: Option<i32>,
    /// Solo: lives left.
    pub lives: Option<i32>,
    /// Solo: the run's score.
    pub spscore: Option<f64>,
    pub bronze_kills: Option<i32>,
    pub silver_kills: Option<i32>,
    pub gold_kills: Option<i32>,
    pub boss_tank_killed: Option<i32>,
    /// Solo: kills in quick succession.
    pub quick_kill: Option<i32>,
    /// Solo: the time of the last kill or brain (sim ms).
    pub last_kill_time: Option<i64>,
    pub brains: Option<i32>,
    pub max_brains: Option<i32>,
    /// The round clock the client was given when it entered (s).
    pub synch_time: Option<f64>,
    /// A bot's slot.
    pub index: Option<u32>,
    /// The plain player name.
    pub name_base: Option<String>,
    /// A bot's driver (its AI def).
    pub ai: Option<String>,
}

/// A solo score as text: the plain number ("1234567", never "1.23457e+06").
pub fn score_text(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        super::state::secs_text(v)
    }
}

fn num(v: Option<f64>) -> Option<String> {
    v.map(format_g)
}

fn int(v: Option<i32>) -> Option<String> {
    v.map(|v| v.to_string())
}

impl ClientRecord {
    /// (name, value) of the set fields, in a fixed order (the round
    /// summaries).
    pub fn trace_fields(&self) -> Vec<(&'static str, String)> {
        let all = [
            ("score", num(self.score)),
            ("cumscore", num(self.cum_score)),
            ("kills", int(self.kills)),
            ("deaths", int(self.deaths)),
            ("lives", int(self.lives)),
            ("spscore", self.spscore.map(score_text)),
            ("bronzekills", int(self.bronze_kills)),
            ("silverkills", int(self.silver_kills)),
            ("goldkills", int(self.gold_kills)),
            ("bosstankkilled", int(self.boss_tank_killed)),
            ("quickkill", int(self.quick_kill)),
            ("lastkilltime", self.last_kill_time.map(|t| t.to_string())),
            ("brains", int(self.brains)),
            ("maxbrains", int(self.max_brains)),
            ("synchtime", self.synch_time.map(super::state::secs_text)),
            ("index", self.index.map(|i| i.to_string())),
            ("namebase", self.name_base.clone()),
            ("ai", self.ai.clone()),
        ];
        all.into_iter().filter_map(|(n, v)| v.map(|v| (n, v))).collect()
    }

    /// The score as clients see it ("" unset).
    pub fn score_text(&self) -> String {
        num(self.score).unwrap_or_default()
    }

    pub fn cum_score_text(&self) -> String {
        num(self.cum_score).unwrap_or_default()
    }

    pub fn name(&self) -> &str {
        self.name_base.as_deref().unwrap_or("")
    }
}

/// A text field: empty is unset.
pub fn text_field(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// A team's record.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TeamRecord {
    pub long_name: String,
    pub short_name: String,
    pub score: f64,
    pub cum_score: f64,
}

impl TeamRecord {
    pub fn trace_fields(&self) -> Vec<(&'static str, String)> {
        vec![("longname", self.long_name.clone()), ("shortname", self.short_name.clone()), ("score", format_g(self.score)), ("cumscore", format_g(self.cum_score))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_fields_in_order() {
        let r = ClientRecord { cum_score: Some(12.0), score: Some(3.0), last_kill_time: Some(1_234_567), ..Default::default() };
        assert_eq!(r.trace_fields(), vec![("score", "3".to_string()), ("cumscore", "12".to_string()), ("lastkilltime", "1234567".to_string())]);
    }
}
