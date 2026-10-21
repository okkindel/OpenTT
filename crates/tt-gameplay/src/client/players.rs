//! The player list: the players' and teams' rows as the lists held them,
//! plus OpenTT's K/D/A and ping columns, and the copy the
//! end-of-round screen shows.
//!
//! A row keeps the list control's tab-separated fields (icon path, name,
//! score[, K, D, A][, ping], cumScore) so updates, padding and sorting
//! behave exactly as the text list did; [`PlayerList::rows`] gives them
//! typed.

use std::collections::HashMap;

use super::text::{atof, atoi, get_field, lf, set_field, strip_ml};

/// `joinGameServerList.iconPath`.
pub const ICON_PATH: &str = "ui/icons/";

/// Team of a row (its icon).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeamIcon {
    Red,
    Green,
    Blue,
}

impl TeamIcon {
    /// `joinGameServerList.<redTeam|GreenTeam|BlueTeam>`.
    pub fn bitmap(self) -> &'static str {
        match self {
            TeamIcon::Red => "RedDot",
            TeamIcon::Green => "GreenDot",
            TeamIcon::Blue => "BlueDot",
        }
    }

    fn of_path(icon: &str) -> TeamIcon {
        let i = icon.to_ascii_lowercase();
        if i.ends_with("greendot") {
            TeamIcon::Green
        } else if i.ends_with("bluedot") {
            TeamIcon::Blue
        } else {
            TeamIcon::Red
        }
    }
}

/// K/D/A of one client ().
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Kda {
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
}

/// A row of a list: its id (the client or team id) and fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub text: String,
}

/// A `GuiTextListCtrl`'s rows and the operations the scripts used.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextList {
    pub rows: Vec<Row>,
}

impl TextList {
    fn find(&self, id: &str) -> Option<usize> {
        self.rows.iter().position(|r| atoi(&r.id) == atoi(id))
    }

    pub fn text(&self, id: &str) -> Option<&str> {
        self.find(id).map(|i| self.rows[i].text.as_str())
    }

    /// `addRow` when the id is new, else `setRowById`; then sorted by score.
    fn put(&mut self, id: &str, text: String) {
        match self.find(id) {
            Some(i) => self.rows[i].text = text,
            None => self.rows.push(Row { id: id.to_string(), text }),
        }
        self.sort_numerical_desc(2);
    }

    fn set(&mut self, id: &str, text: String) {
        if let Some(i) = self.find(id) {
            self.rows[i].text = text;
        }
    }

    fn remove(&mut self, id: &str) {
        if let Some(i) = self.find(id) {
            self.rows.remove(i);
        }
    }

    /// `sortNumerical(col, 0)` (stable).
    fn sort_numerical_desc(&mut self, col: usize) {
        self.rows.sort_by(|x, y| {
            let (a, b) = (atof(&get_field(&x.text, col)), atof(&get_field(&y.text, col)));
            a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal).reverse()
        });
    }

    /// `sort(col)`: by the text, ascending, case-insensitive (stable).
    fn sort_text(&mut self, col: usize) {
        self.rows.sort_by_key(|x| get_field(&x.text, col).to_ascii_lowercase());
    }
}

/// One row as the HUD and the round results show it.
#[derive(Debug, Clone, PartialEq)]
pub struct RowView {
    pub team: TeamIcon,
    /// The name with the " [Admin]"/" [Super]"/" [Bot]" tags, markup removed, trimmed.
    pub name: String,
    pub score: String,
    pub kda: Option<[String; 3]>,
    pub ping: Option<String>,
}

/// The player list (`PlayerListGui`): players and teams.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerList {
    pub players: TextList,
    pub teams: TextList,
    /// `TeamListGuiList.visible` (team games).
    pub teams_visible: bool,
    /// The ping column is on (set by the first ping list of a network
    /// game, kept afterwards).
    pub ping_column: bool,
    /// The last ping of each client.
    pub pings: HashMap<i32, String>,
    /// What the K/D/A columns show, by client id.
    pub kda: HashMap<u32, Kda>,
}

/// The fields of a player row: (ping, cumScore) indices, after the K/D/A
/// columns, with or without the ping column (`ping`).
pub fn fields(ping: bool) -> (usize, usize) {
    (PING_AT, PING_AT + usize::from(ping))
}

/// The ping field of a player row (after icon, name, score, K, D, A).
const PING_AT: usize = 6;

/// A player's join (`handleClientJoin` → `PlayerListGui::update`).
#[derive(Debug, Clone, Default)]
pub struct Join<'a> {
    pub id: &'a str,
    /// The name, detagged.
    pub name: &'a str,
    pub is_super: &'a str,
    pub is_admin: &'a str,
    pub score: &'a str,
    pub cum_score: &'a str,
    /// The team's name (detagged), e.g. `BlueTeam`.
    pub team: &'a str,
}

impl PlayerList {
    fn kda_of(&self, id: &str) -> Kda {
        self.kda.get(&(atoi(id) as u32)).copied().unwrap_or_default()
    }

    /// `PlayerListGui::update`: the row of a joining (or renamed) client.
    pub fn update(&mut self, j: &Join) {
        let tag = if lf(j.is_super) != 0.0 {
            "[Super]"
        } else if lf(j.is_admin) != 0.0 {
            "[Admin]"
        } else {
            ""
        };
        let icon = if super::text::eq(j.team, "BlueTeam") {
            TeamIcon::Blue
        } else if super::text::eq(j.team, "GreenTeam") {
            TeamIcon::Green
        } else {
            TeamIcon::Red
        };
        let name = strip_ml(&strip_ml(j.name));
        let k = self.kda_of(j.id);
        let mut text = format!("{ICON_PATH}{}	{name} {tag}	{}	{}	{}	{}", icon.bitmap(), j.score, k.kills, k.deaths, k.assists);
        if self.ping_column {
            let ping = self.pings.get(&atoi(j.id)).cloned().unwrap_or_default();
            text = format!("{text}\t{ping}");
        }
        let text = format!("{text}\t{}", j.cum_score);
        self.players.put(j.id, text);
    }

    /// `PlayerListGui::remove` (`handleClientDrop`).
    pub fn remove(&mut self, id: &str) {
        self.players.remove(id);
    }

    pub fn update_score(&mut self, id: &str, score: &str, cum: &str) {
        let cum_at = fields(self.ping_column).1;
        update_score(&mut self.players, id, score, cum, cum_at);
    }

    /// Team games only.
    pub fn update_team_score(&mut self, id: &str, score: &str, cum: &str) {
        update_score(&mut self.teams, id, score, cum, 3);
    }

    pub fn add_team(&mut self, id: &str, long_name: &str, score: &str, cum: &str) {
        self.teams_visible = true;
        let icon = if super::text::eq(long_name, "Blue Team") {
            TeamIcon::Blue
        } else if super::text::eq(long_name, "Green Team") {
            TeamIcon::Green
        } else {
            TeamIcon::Red
        };
        let text = format!("{ICON_PATH}{}\t{}\t{score}\t{cum}", icon.bitmap(), strip_ml(long_name));
        self.teams.put(id, text);
    }

    /// `PlayerListGui::clear` (disconnect).
    pub fn clear(&mut self) {
        self.teams = TextList::default();
        self.teams_visible = false;
        self.players = TextList::default();
    }

    /// Scores to 0, rows sorted by their first field.
    pub fn zero_scores(&mut self) {
        for list in [&mut self.players, &mut self.teams] {
            for r in &mut list.rows {
                r.text = set_field(&r.text, 2, "0");
            }
            list.sort_text(0);
        }
    }

    /// K/D/A of `entries` into the view and the rows; `full` = the whole
    /// list (everyone else is zero).
    pub fn set_kda(&mut self, entries: &[(u32, Kda)], full: bool) {
        if full {
            self.kda.clear();
        }
        for (id, k) in entries {
            self.kda.insert(*id, *k);
        }
        for i in 0..self.players.rows.len() {
            let k = self.kda_of(&self.players.rows[i].id.clone());
            let text = &self.players.rows[i].text;
            let new = with_kda(text, k);
            if new != *text {
                let id = self.players.rows[i].id.clone();
                self.players.set(&id, new);
            }
        }
    }

    /// A ping list: the first one switches the rows to the ping layout (an
    /// empty ping field before cumScore); every listed row gets its ping.
    pub fn set_pings(&mut self, entries: &[(u32, u32)]) {
        let ping_at = PING_AT;
        if !self.ping_column {
            self.ping_column = true;
            for r in &mut self.players.rows {
                let mut f: Vec<&str> = r.text.split('\t').collect();
                if f.len() == ping_at + 1 {
                    f.insert(ping_at, "");
                    r.text = f.join("\t");
                }
            }
        }
        for (id, ping) in entries {
            let (id, ping) = (id.to_string(), ping.to_string());
            self.pings.insert(atoi(&id), ping.clone());
            if let Some(t) = self.players.text(&id) {
                let t = set_field(t, ping_at, &ping);
                self.players.set(&id, t);
            }
        }
    }

    /// The rows typed: players and, when the team list is visible, the teams.
    pub fn rows(&self) -> (Vec<RowView>, Vec<RowView>) {
        let players = self.players.rows.iter().map(|r| row_view(&r.text, true, self.ping_column)).collect();
        let teams = if self.teams_visible { self.teams.rows.iter().map(|r| RowView { kda: None, ping: None, ..row_view(&r.text, false, false) }).collect() } else { Vec::new() };
        (players, teams)
    }
}

/// Fields 3..5 of a K/D/A row.
pub fn with_kda(row: &str, k: Kda) -> String {
    let mut f: Vec<String> = row.split('\t').map(str::to_string).collect();
    if f.len() < 6 {
        return row.to_string();
    }
    f[3] = k.kills.to_string();
    f[4] = k.deaths.to_string();
    f[5] = k.assists.to_string();
    f.join("\t")
}

fn update_score(list: &mut TextList, id: &str, score: &str, cum: &str, cum_at: usize) {
    let text = list.text(id).unwrap_or("").to_string();
    let text = set_field(&text, 2, score);
    let text = set_field(&text, cum_at, cum);
    list.set(id, text);
    list.sort_numerical_desc(2);
}

/// A row's fields typed (`icon, name, score[, K, D, A][, ping], cumScore`).
pub fn row_view(row: &str, kda: bool, ping: bool) -> RowView {
    let f: Vec<&str> = row.split('\t').collect();
    let get = |i: usize| f.get(i).copied().unwrap_or("").to_string();
    let name = super::text::clean(&get(1)).trim().to_string();
    let kda_f = (kda && f.len() >= 7).then(|| [get(3), get(4), get(5)]);
    let ping_at = if kda { 6 } else { 3 };
    RowView { team: TeamIcon::of_path(&get(0)), name, score: get(2), kda: kda_f, ping: ping.then(|| get(ping_at)) }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EndGame {
    pub players: TextList,
    pub teams: TextList,
    /// `endGameLoadingProgress` (0..1, script text).
    pub loading: String,
}

impl EndGame {
    /// Copies of the player list.
    pub fn copy_from(&mut self, list: &PlayerList) {
        for (to, from) in [(&mut self.players, &list.players), (&mut self.teams, &list.teams)] {
            to.rows = from.rows.clone();
            to.sort_numerical_desc(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn join<'a>(id: &'a str, name: &'a str, team: &'a str, score: &'a str) -> Join<'a> {
        Join { id, name, is_super: "0", is_admin: "0", score, cum_score: "0", team }
    }

    #[test]
    fn rows_sort_by_score() {
        let mut l = PlayerList::default();
        l.update(&join("10", "Bait", "BlueTeam", "3"));
        l.update(&join("11", "\u{10}\u{c}Hum [Bot]\u{11}", "GreenTeam", "5"));
        l.update(&join("12", "Tom", "", "5"));
        let names: Vec<String> = l.rows().0.into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["Hum [Bot]", "Tom", "Bait"]);
        l.update_score("10", "9", "9");
        let r = &l.rows().0[0];
        assert_eq!((r.name.as_str(), r.score.as_str(), r.team), ("Bait", "9", TeamIcon::Blue));
        l.set_kda(&[(10, Kda { kills: 2, deaths: 1, assists: 0 })], true);
        assert_eq!(l.rows().0[0].kda, Some(["2".into(), "1".into(), "0".into()]));
        l.set_pings(&[(10, 45)]);
        assert_eq!(l.rows().0[0].ping.as_deref(), Some("45"));
        assert_eq!(l.rows().0[1].ping.as_deref(), Some(""));
        l.zero_scores();
        assert_eq!(l.rows().0.iter().map(|r| r.team).collect::<Vec<_>>(), [TeamIcon::Blue, TeamIcon::Green, TeamIcon::Red]);
        l.remove("11");
        assert_eq!(l.players.rows.len(), 2);
    }

    #[test]
    fn teams() {
        let mut l = PlayerList::default();
        assert!(l.rows().1.is_empty());
        l.add_team("1", "Blue Team", "0", "0");
        l.add_team("2", "Green Team", "4", "0");
        l.update_team_score("1", "7", "7");
        let t = l.rows().1;
        assert_eq!(t.iter().map(|r| (r.name.as_str(), r.score.as_str())).collect::<Vec<_>>(), [("Blue Team", "7"), ("Green Team", "4")]);
        l.clear();
        assert!(!l.teams_visible && l.players.rows.is_empty());
    }
}
