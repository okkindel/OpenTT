//! The solo campaign: the `SPLevel1..20` table, the level sequence of
//! `nextSPLevel`/`currentSPLevel`, the run state the scripts keep on
//! `LocalClientConnection`, the post-level tally of soloPlayGui
//! (`presentScore` / `advanceScore` / `finishScore`) and the game-over
//! bookkeeping.

use tt_game::fields::Fields;

use crate::hiscores::HiScores;
use crate::players::Players;
use crate::vars::{atof, num};

/// One `SPLevelN` object. Numeric fields are kept as f64 (script numbers).
#[derive(Debug, Clone)]
pub struct SpLevel {
    pub index: u32,
    pub mission: String,
    pub mission_briefing: String,
    /// "Battle", "Boss" or "Brain".
    pub level_type: String,
    /// Non-zero: this slot only redirects to level `goto_level` and bumps
    /// every other level by the `add*` values.
    pub gotolevel: u32,
    pub damage_scale: f64,
    pub armor_scale: f64,
    pub fire_scale: f64,
    pub bronze_pts: f64,
    pub silver_pts: f64,
    pub gold_pts: f64,
    pub boss_pts: f64,
    pub brain_pts: f64,
    pub add_damage_scale: f64,
    pub add_armor_scale: f64,
    pub add_fire_scale: f64,
    pub add_bronze_pts: f64,
    pub add_silver_pts: f64,
    pub add_gold_pts: f64,
    pub add_boss_pts: f64,
    pub add_brain_pts: f64,
    /// Every field as the script stores it (bot counts, mixes, timings…).
    pub fields: Fields,
}

impl SpLevel {
    pub fn from_fields(index: u32, f: &Fields) -> SpLevel {
        let n = |k: &str| f.get(k).map_or(0.0, atof);
        SpLevel {
            index,
            mission: f.str_or("mission", "").to_string(),
            mission_briefing: f.str_or("mission_briefing", "").to_string(),
            level_type: f.str_or("level_type", "").to_string(),
            gotolevel: n("goto_level").max(0.0) as u32,
            damage_scale: n("damage_scale"),
            armor_scale: n("armor_scale"),
            fire_scale: n("fire_scale"),
            bronze_pts: n("bronze_pts"),
            silver_pts: n("silver_pts"),
            gold_pts: n("gold_pts"),
            boss_pts: n("boss_pts"),
            brain_pts: n("brain_pts"),
            add_damage_scale: n("add_damage_scale"),
            add_armor_scale: n("add_armor_scale"),
            add_fire_scale: n("add_fire_scale"),
            add_bronze_pts: n("add_bronze_pts"),
            add_silver_pts: n("add_silver_pts"),
            add_gold_pts: n("add_gold_pts"),
            add_boss_pts: n("add_boss_pts"),
            add_brain_pts: n("add_brain_pts"),
            fields: f.clone(),
        }
    }

    pub fn field(&self, name: &str) -> String {
        match name.to_ascii_lowercase().as_str() {
            "damage_scale" => num(self.damage_scale),
            "armor_scale" => num(self.armor_scale),
            "fire_scale" => num(self.fire_scale),
            "bronze_pts" => num(self.bronze_pts),
            "silver_pts" => num(self.silver_pts),
            "gold_pts" => num(self.gold_pts),
            "boss_pts" => num(self.boss_pts),
            "brain_pts" => num(self.brain_pts),
            _ => self.fields.get(name).unwrap_or("").to_string(),
        }
    }
}

/// The SPLevels group: `levels[i]` is `SPLevel(i+1)`.
#[derive(Debug, Clone)]
pub struct LevelTable {
    pub levels: Vec<SpLevel>,
}

impl LevelTable {
    /// `SPLevel<n>` (1-based).
    pub fn get(&self, n: u32) -> Option<&SpLevel> {
        n.checked_sub(1).and_then(|i| self.levels.get(i as usize))
    }
}

/// The engine writes the counters during play (the gameplay scripts still
/// do) and hands them back here between levels.
#[derive(Debug, Clone)]
pub struct SoloRun {
    /// The run's own copy of SPLevels (gotolevel bumps mutate it).
    pub table: LevelTable,
    /// The level being / last played.
    pub mission_seq: u32,
    /// Levels started in this run.
    pub mission_seq_total: u32,
    pub lives: i32,
    /// `.spscore`: the run's score.
    pub score: f64,
    pub bronze_kills: i32,
    pub silver_kills: i32,
    pub gold_kills: i32,
    pub brains: i32,
    pub max_brains: i32,
    pub deaths: i32,
    pub boss_tank_killed: i32,
    /// Campaign position the run started at.
    pub start_level: u32,
}

/// One entry of the level-select list: the level played at campaign
/// position `position` of a run that never loses, with the slot
/// (`SPLevel<slot>`) the sequence reaches there and how many times its
/// mission came before (+1).
#[derive(Debug, Clone, PartialEq)]
pub struct CampaignLevel {
    pub position: u32,
    pub slot: u32,
    pub mission: String,
    pub level_type: String,
    pub briefing: String,
    /// 1 on the first pass through the worlds, 2 once the sequence wraps
    /// (`SPLevel18` → `SPLevel19`, then `SPLevel20` → `SPLevel2`), …
    pub cycle: u32,
}

/// The first `count` positions of the campaign.
pub fn campaign_levels(table: &LevelTable, count: u32) -> Vec<CampaignLevel> {
    let mut run = SoloRun::new(table.clone());
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for position in 1..=count {
        let Some(l) = run.advance_level().cloned() else { break };
        let cycle = 1 + seen.iter().filter(|m| m.eq_ignore_ascii_case(&l.mission)).count() as u32;
        seen.push(l.mission.clone());
        out.push(CampaignLevel { position, slot: l.index, mission: l.mission, level_type: l.level_type, briefing: l.mission_briefing, cycle });
    }
    out
}

/// One row of the post-level tally.
#[derive(Debug, Clone, PartialEq)]
pub enum TallyRow {
    /// Bronze/Silver/Gold/Brain row: `count` × `pts_per` = `pts`
    /// (`out_of` = maxBrains for the brain row).
    Kills { kind: KillKind, count: u32, pts_per: f64, pts: f64, out_of: Option<i32> },
    /// No deaths in a Battle level: the kill points again.
    Survival { pts: f64 },
    Boss { pts: f64 },
    /// All brains collected: 5000 + brain_pts·50.
    Completion { pts: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillKind {
    Bronze,
    Silver,
    Gold,
    Brain,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tally {
    pub rows: Vec<TallyRow>,
    pub start_score: f64,
    pub score: f64,
    pub lives_before: i32,
    pub lives: i32,
    /// Extra lives awarded (one per 10 000 points crossed).
    pub extra_lives: i32,
    /// The animation steps of `advanceScore` with their delays (ms), for a
    /// faithful presentation.
    pub steps: Vec<TallyStep>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TallyStep {
    pub delay_ms: i32,
    pub score: f64,
    pub lives: i32,
    pub event: TallyEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TallyEvent {
    ExtraLifeFlash,
    ExtraLife,
    BonusText,
    ShowRow(KillKind),
    Count(KillKind),
    Survival,
    Boss,
    Completion,
    Done,
}

impl SoloRun {
    /// A new run: 3 lives, no score, sequence 0.
    pub fn new(table: LevelTable) -> SoloRun {
        SoloRun {
            table,
            mission_seq: 0,
            mission_seq_total: 0,
            lives: 3,
            score: 0.0,
            bronze_kills: 0,
            silver_kills: 0,
            gold_kills: 0,
            brains: 0,
            max_brains: 0,
            deaths: 0,
            boss_tank_killed: 0,
            start_level: 1,
        }
    }

    /// `position` 1 is [`SoloRun::new`]. None when the sequence ends
    /// before `position`.
    pub fn starting_at(table: LevelTable, position: u32) -> Option<SoloRun> {
        let mut run = SoloRun::new(table);
        for _ in 1..position.max(1) {
            run.advance_level()?;
        }
        run.peek_next_level()?;
        run.start_level = position.max(1);
        Some(run)
    }

    /// The run has started a level.
    pub fn started(&self) -> bool {
        self.mission_seq_total + 1 > self.start_level
    }

    pub fn score_value(&self) -> f64 {
        self.score
    }

    pub fn set_score(&mut self, score: f64) {
        self.score = score;
    }

    /// `currentSPLevel()`: `SPLevel<MissionSeq>`.
    pub fn current_level(&self) -> Option<&SpLevel> {
        self.table.get(self.mission_seq)
    }

    /// `nextSPLevel(0)`: the level the solo screen shows next (following
    /// a `goto_level` slot without applying it).
    pub fn peek_next_level(&self) -> Option<&SpLevel> {
        let l = self.table.get(self.mission_seq + 1)?;
        if l.gotolevel != 0 {
            self.table.get(l.gotolevel)
        } else {
            Some(l)
        }
    }

    /// `nextSPLevel(1)` (via `nextSPMission`): advances the sequence; a
    /// `goto_level` slot bumps every other level (armor/fire/damage scales
    /// capped at 1, points added) and jumps. Returns the level to play.
    pub fn advance_level(&mut self) -> Option<&SpLevel> {
        let n = self.mission_seq + 1;
        self.mission_seq += 1;
        self.mission_seq_total += 1;
        let l = self.table.get(n)?.clone();
        if l.gotolevel != 0 {
            for other in self.table.levels.iter_mut().filter(|o| o.index != l.index) {
                other.armor_scale = (other.armor_scale + l.add_armor_scale).min(1.0);
                other.fire_scale = (other.fire_scale + l.add_fire_scale).min(1.0);
                other.damage_scale = (other.damage_scale + l.add_damage_scale).min(1.0);
                other.bronze_pts += l.add_bronze_pts;
                other.silver_pts += l.add_silver_pts;
                other.gold_pts += l.add_gold_pts;
                other.boss_pts += l.add_boss_pts;
                other.brain_pts += l.add_brain_pts;
            }
            self.mission_seq = l.gotolevel;
            return self.table.get(l.gotolevel);
        }
        self.table.get(n)
    }

    /// Before launching the next level (`soloPlayLaunchButton`): the tally
    /// must be finished; `lastKillTime` and `deaths` reset.
    pub fn prepare_launch(&mut self) {
        self.deaths = 0;
    }

    /// The post-level tally of `presentScore` + `finishScore`: runs
    /// `advanceScore` to completion, applying everything it mutates (score,
    /// extra lives, counters zeroed, `deaths = 1` marker after a survival
    /// bonus, boss flag cleared). Returns None before the first level.
    pub fn tally(&mut self) -> Option<Tally> {
        if self.mission_seq_total == 0 {
            return None;
        }
        let level = self.current_level()?.clone();
        let level_type = level.level_type.to_ascii_lowercase();
        let pts_per = [level.bronze_pts * 1.5, level.silver_pts * 1.5, level.gold_pts * 1.5, level.brain_pts * 2.0];
        let completion_pts = 5000.0 + level.brain_pts * 50.0;
        let boss_pts = level.boss_pts * 2.0;
        let kinds = [KillKind::Bronze, KillKind::Silver, KillKind::Gold, KillKind::Brain];

        let start_score = self.score_value();
        let lives_before = self.lives;
        let mut shown = [false; 4];
        let mut mult = [0u32; 4];
        let mut pts = [0.0f64; 4];
        let (mut bonus_text, mut survival_shown, mut boss_shown, mut completion_shown) = (false, false, false, false);
        let (mut add_life, mut life_flash, mut extra_visible) = (0i32, 0i32, false);
        let mut survival_pts = 0.0;
        let mut steps = Vec::new();
        let mut extra_lives = 0;

        loop {
            // One advanceScore().
            let step_start = self.score_value();
            let mut score = step_start;
            let mut done = false;
            let mut delay = -1;
            let mut event = TallyEvent::Done;
            if life_flash > 0 {
                if extra_visible {
                    extra_visible = false;
                    life_flash -= 1;
                    if life_flash == 0 {
                        self.lives += 1;
                        extra_lives += 1;
                        event = TallyEvent::ExtraLife;
                    } else {
                        event = TallyEvent::ExtraLifeFlash;
                    }
                } else {
                    extra_visible = true;
                    event = TallyEvent::ExtraLifeFlash;
                }
                delay = 250;
                done = true;
            } else if add_life > 0 {
                extra_visible = true;
                add_life -= 1;
                life_flash = 5;
                delay = 150;
                done = true;
                event = TallyEvent::ExtraLifeFlash;
            } else {
                extra_visible = false;
            }
            let mut kills = [self.bronze_kills, self.silver_kills, self.gold_kills, self.brains];
            if !done && !bonus_text && kills.iter().any(|&k| k > 0) {
                bonus_text = true;
                delay = 1500;
                done = true;
                event = TallyEvent::BonusText;
            }
            for idx in 0..4 {
                if done {
                    break;
                }
                if !shown[idx] && kills[idx] > 0 {
                    shown[idx] = true;
                    delay = 750;
                    done = true;
                    event = TallyEvent::ShowRow(kinds[idx]);
                } else if kills[idx] > 0 {
                    mult[idx] += 1;
                    pts[idx] += pts_per[idx];
                    score += pts_per[idx];
                    delay = if idx == 3 { 100 } else { 250 };
                    done = true;
                    kills[idx] -= 1;
                    event = TallyEvent::Count(kinds[idx]);
                }
            }
            (self.bronze_kills, self.silver_kills, self.gold_kills, self.brains) = (kills[0], kills[1], kills[2], kills[3]);
            if !survival_shown && !done && self.deaths == 0 && level_type == "battle" {
                survival_pts = pts[0] + pts[1] + pts[2];
                survival_shown = true;
                score += survival_pts;
                self.deaths = 1;
                delay = 500;
                done = true;
                event = TallyEvent::Survival;
            }
            if !boss_shown && !done && level_type == "boss" && self.boss_tank_killed != 0 {
                boss_shown = true;
                self.boss_tank_killed = 0;
                score += boss_pts;
                delay = 500;
                done = true;
                event = TallyEvent::Boss;
            }
            if !completion_shown && !done && mult[3] as i32 == self.max_brains && level_type == "brain" {
                completion_shown = true;
                score += completion_pts;
                delay = 500;
                event = TallyEvent::Completion;
            }
            let free = (score / 10000.0).floor() as i64 - (step_start / 10000.0).floor() as i64;
            if free > 0 {
                add_life += free as i32;
            }
            self.set_score(score);
            steps.push(TallyStep { delay_ms: delay, score, lives: self.lives, event });
            if delay == -1 {
                break;
            }
        }

        let mut rows = Vec::new();
        for idx in 0..4 {
            if shown[idx] {
                rows.push(TallyRow::Kills { kind: kinds[idx], count: mult[idx], pts_per: pts_per[idx], pts: pts[idx], out_of: (idx == 3).then_some(self.max_brains) });
            }
        }
        if survival_shown {
            rows.push(TallyRow::Survival { pts: survival_pts });
        }
        if boss_shown {
            rows.push(TallyRow::Boss { pts: boss_pts });
        }
        if completion_shown {
            rows.push(TallyRow::Completion { pts: completion_pts });
        }
        Some(Tally { rows, start_score, score: self.score_value(), lives_before, lives: self.lives, extra_lives, steps })
    }

    /// Returns the rank (0-based) or None. Save both files afterwards.
    pub fn end_run(&mut self, players: &mut Players, hiscores: &mut HiScores, player_name: &str) -> Option<usize> {
        let score = self.score;
        players.record_solo_score(score);
        self.lives = 0;
        self.bronze_kills = 0;
        self.silver_kills = 0;
        self.gold_kills = 0;
        self.brains = 0;
        hiscores.insert(player_name, score, self.mission_seq_total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(i: u32, pairs: &[(&str, &str)]) -> SpLevel {
        SpLevel::from_fields(i, &Fields::from_pairs(&format!("SPLevel{i}"), "Level", pairs))
    }

    fn battle() -> SpLevel {
        level(1, &[("bronze_pts", "50"), ("silver_pts", "100"), ("gold_pts", "150"), ("boss_pts", "3000"), ("brain_pts", "100"), ("level_type", "Battle"), ("mission", "tt1_1"), ("armor_scale", "0.2"), ("fire_scale", "0.5"), ("damage_scale", "0.4")])
    }

    fn run_with(levels: Vec<SpLevel>) -> SoloRun {
        let mut r = SoloRun::new(LevelTable { levels });
        r.advance_level();
        r
    }

    #[test]
    fn tally_like_test_score() {
        // soloPlayGui::testScore: 5 bronze, 7 silver, 3 gold, start 9000,
        // no deaths, Battle level.
        let mut r = run_with(vec![battle()]);
        r.set_score(9000.0);
        (r.bronze_kills, r.silver_kills, r.gold_kills) = (5, 7, 3);
        let t = r.tally().unwrap();
        // 5·75 + 7·150 + 3·225 = 2100, survival bonus 2100 again.
        assert_eq!(t.rows[0], TallyRow::Kills { kind: KillKind::Bronze, count: 5, pts_per: 75.0, pts: 375.0, out_of: None });
        assert_eq!(t.rows[3], TallyRow::Survival { pts: 2100.0 });
        assert_eq!(t.score, 9000.0 + 4200.0);
        // 10 000 crossed once: one extra life.
        assert_eq!((t.lives_before, t.lives, t.extra_lives), (3, 4, 1));
        assert_eq!(r.deaths, 1);
        assert_eq!((r.bronze_kills, r.silver_kills, r.gold_kills), (0, 0, 0));
        assert_eq!(t.steps.last().unwrap().delay_ms, -1);
        // Running it again does nothing more (counters consumed).
        let again = r.tally().unwrap();
        assert_eq!(again.score, t.score);
        assert!(again.rows.is_empty());
    }

    #[test]
    fn tally_brain_and_boss() {
        let mut brain = battle();
        brain.level_type = "Brain".into();
        let mut r = run_with(vec![brain]);
        r.set_score(1000.0);
        (r.brains, r.max_brains, r.deaths) = (20, 20, 2);
        let t = r.tally().unwrap();
        // 20 × 200, completion 5000 + 100·50.
        assert_eq!(t.rows, vec![TallyRow::Kills { kind: KillKind::Brain, count: 20, pts_per: 200.0, pts: 4000.0, out_of: Some(20) }, TallyRow::Completion { pts: 10000.0 }]);
        assert_eq!(t.score, 15000.0);
        assert_eq!(t.extra_lives, 1);

        let mut boss = battle();
        boss.level_type = "Boss".into();
        let mut r = run_with(vec![boss]);
        (r.gold_kills, r.boss_tank_killed, r.deaths) = (1, 1, 1);
        let t = r.tally().unwrap();
        assert_eq!(t.rows, vec![TallyRow::Kills { kind: KillKind::Gold, count: 1, pts_per: 225.0, pts: 225.0, out_of: None }, TallyRow::Boss { pts: 6000.0 }]);
        assert_eq!(t.score, 6225.0);
    }

    #[test]
    fn gotolevel_redirect_and_bumps() {
        let mut l3 = level(3, &[("goto_level", "1"), ("add_armor_scale", "0.9"), ("add_bronze_pts", "50"), ("add_boss_pts", "3000")]);
        l3.index = 3;
        let mut r = SoloRun::new(LevelTable { levels: vec![battle(), level(2, &[("mission", "tt1_2"), ("armor_scale", "0.5")]), l3] });
        assert_eq!(r.peek_next_level().unwrap().mission, "tt1_1");
        r.advance_level();
        r.advance_level();
        assert_eq!(r.peek_next_level().unwrap().mission, "tt1_1");
        let l = r.advance_level().unwrap().clone();
        assert_eq!((l.index, l.armor_scale, l.bronze_pts, l.boss_pts), (1, 1.0, 100.0, 6000.0));
        assert_eq!((r.mission_seq, r.mission_seq_total), (1, 3));
        assert_eq!(r.table.get(2).unwrap().armor_scale, 1.0);
        assert_eq!(r.current_level().unwrap().mission, "tt1_1");
    }

    #[test]
    fn starting_at_matches_a_played_run() {
        let mut l3 = level(3, &[("goto_level", "1"), ("add_armor_scale", "0.3"), ("add_fire_scale", "0.2"), ("add_damage_scale", "0.1"), ("add_gold_pts", "150")]);
        l3.index = 3;
        let table = LevelTable { levels: vec![battle(), level(2, &[("mission", "tt1_2"), ("armor_scale", "0.5")]), l3] };
        for pos in 1..=6 {
            let mut played = SoloRun::new(table.clone());
            for _ in 1..pos {
                played.advance_level();
                played.set_score(1234.0);
                played.lives = 1;
            }
            let mut chosen = SoloRun::starting_at(table.clone(), pos).unwrap();
            assert_eq!((chosen.lives, chosen.score_value(), chosen.start_level), (3, 0.0, pos));
            assert!(!chosen.started());
            assert_eq!((chosen.mission_seq, chosen.mission_seq_total), (played.mission_seq, played.mission_seq_total));
            let (a, b) = (played.advance_level().unwrap().clone(), chosen.advance_level().unwrap().clone());
            assert_eq!((a.index, a.mission.as_str(), a.armor_scale, a.fire_scale, a.damage_scale, a.gold_pts), (b.index, b.mission.as_str(), b.armor_scale, b.fire_scale, b.damage_scale, b.gold_pts));
            assert_eq!(chosen.mission_seq_total, pos);
            assert!(chosen.started());
        }
        let list = campaign_levels(&table, 4);
        assert_eq!(list.iter().map(|l| (l.position, l.slot, l.cycle)).collect::<Vec<_>>(), vec![(1, 1, 1), (2, 2, 1), (3, 1, 2), (4, 2, 2)]);
    }
}
