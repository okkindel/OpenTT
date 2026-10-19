//! The solo campaign: the level's numbers and the campaign tanks'
//! strengths, the bot spawner, the level's end, points, the quick-shot
//! bonus, extra lives and the game over; and target practice.

use super::combat::BRAIN_SEPARATED;
use super::state::*;
use super::*;
use crate::data::*;
use crate::entity::EntityId;
use crate::msg::*;
use tt_game::fmt::format_g;

impl Server {
    pub fn start_campaign_server(&mut self, h: &mut dyn ServerHost) {
        for (t, tank) in SOLO_TANKS.iter().enumerate() {
            let def = h.data().tanks.named(tank);
            self.campaign.base_scales[t] = def.map_or([0.0; 3], |d| [d.damage_scale, d.armor_scale, d.fire_scale]);
        }
        let first = h.solo_level(1).unwrap_or_default();
        self.apply_solo_scales(h, first.damage_scale, first.armor_scale, first.fire_scale);
    }

    /// The campaign tanks' strengths for a level.
    pub fn apply_solo_scales(&mut self, h: &mut dyn ServerHost, damage: f32, armor: f32, fire: f32) {
        h.set_tank_scales(Some(SoloScales::compute(&self.campaign.base_scales, damage, armor, fire)));
    }

    pub fn stop_campaign_server(&mut self, h: &mut dyn ServerHost) {
        self.setup.single_player = Some(false);
        self.apply_solo_scales(h, 1.0, 1.0, 1.0);
        self.campaign.levels_loaded = false;
    }

    /// The run's next level: its numbers, the tanks' strengths; its
    /// mission file ("" if there is none).
    pub(crate) fn next_solo_level(&mut self, h: &mut dyn ServerHost) -> String {
        let fresh = self.campaign.mission_seq_total.unwrap_or(0) == 0;
        let Some(step) = h.next_sp_mission(fresh) else { return String::new() };
        self.campaign.mission_seq = Some(step.seq);
        self.campaign.mission_seq_total = Some(step.total);
        let l = &step.level;
        self.apply_solo_scales(h, l.damage_scale, l.armor_scale, l.fire_scale);
        self.campaign.level = Some(step.level);
        step.mission_file
    }

    /// A campaign bot of tank `db`, or of the level's mix.
    pub fn spawn_campaign_bot(&mut self, h: &mut dyn ServerHost, db: Option<&str>) {
        let mut def = db.and_then(|db| h.data().tanks.find(db));
        if def.is_none() {
            let mix = self.campaign.mission_seq.and_then(|s| h.solo_level(s)).unwrap_or_default();
            let rand = self.rng.get_random_max(100) as f64;
            let mut db: String = if rand < mix.bronze {
                "Bronze".into()
            } else if rand < mix.silver + mix.bronze {
                "Silver".into()
            } else {
                "Gold".into()
            };
            let rand = self.rng.get_random_max(100) as f64;
            let size = if rand < mix.light {
                "LightTank"
            } else if rand < mix.medium + mix.light {
                "MediumTank"
            } else {
                "HeavyTank"
            };
            db += size;
            def = h.data().tanks.find(&db);
        }
        let Some(def) = def else { return };
        let Some(bot) = self.new_tank(h, def, None) else { return };
        let pos = self.spawn_point(h, "Green");
        h.set_transform(bot, &pos);
        let ai = h.data().tanks[def].ai;
        h.set_ai(bot, ai);
        h.set_team(bot, "Red");
        self.campaign.live_bots += 1.0;
        self.campaign.spawned_bots += 1.0;
    }

    /// The spawner's pause until the next bot: the level's minimum plus a
    /// random part up to its maximum.
    pub(crate) fn next_bot_tick(&mut self) {
        let l = self.campaign.level();
        let span = tt_game::fmt::atoi(&format_g(l.max_bot_ticks as f64 - l.min_bot_ticks as f64));
        let r = self.rng.get_random_max(span) as f64;
        self.campaign.next_bot_tick = (l.min_bot_ticks as f64 + r) as f32;
    }

    pub fn start_solo_spawner(&mut self, h: &mut dyn ServerHost) -> TimerId {
        self.campaign.live_bots = 0.0;
        self.campaign.spawned_bots = 0.0;
        self.campaign.dead_bots = 0.0;
        self.next_bot_tick();
        let l = self.local();
        self.with_record(l, |r| r.boss_tank_killed = Some(0));
        self.timers.schedule(h, delay_ms(5.0), Timer::SoloTick)
    }

    pub fn spawn_solo_start_bots(&mut self, h: &mut dyn ServerHost) {
        let mut i = 0;
        while (i as f64) < self.campaign.level().start_bots as f64 {
            self.spawn_campaign_bot(h, None);
            i += 1;
        }
        if self.campaign.kind() == Some(LevelKind::Boss) {
            self.spawn_campaign_bot(h, Some("BossTank"));
        }
    }

    /// Every 5 s of a solo level: the level's end, the next bot.
    pub fn solo_tick(&mut self, h: &mut dyn ServerHost) {
        let local = self.local();
        if self.rec(local, |r| r.lives.unwrap_or(0)) == 0 {
            return;
        }
        let l = self.campaign.level();
        let mut finished = l.max_mission_ticks == 1.0;
        let now = h.sim_time() as i32 as f64;
        if self.rec(local, |r| r.last_kill_time.unwrap_or(0)) as f64 + 1000.0 < now {
            let brains = h.group("BrainGroup");
            let done = (self.campaign.dead_bots >= l.end_bots && l.end_bots.trunc() != 0.0)
                || self.rec(local, |r| r.boss_tank_killed.unwrap_or(0)) != 0
                || brains.is_some_and(|b| b.len() as i32 == self.rec(local, |r| r.brains.unwrap_or(0)));
            if done {
                finished = true;
            }
        }
        if finished {
            self.timers.schedule(h, 0, Timer::SoloLevelEnd);
            return;
        }
        if let Some(l) = self.campaign.level.as_mut() {
            l.max_mission_ticks -= 1.0;
        }
        if self.campaign.next_bot_tick == 0.0 {
            if self.campaign.spawned_bots < l.max_spawn_bots && self.campaign.live_bots < l.max_live_bots {
                self.spawn_campaign_bot(h, None);
            }
            self.next_bot_tick();
        } else {
            self.campaign.next_bot_tick -= 1.0;
        }
        let id = self.timers.schedule(h, delay_ms(5.0), Timer::SoloTick);
        self.round.solo_timer = Some(id);
    }

    pub fn spawn_target(&mut self, h: &mut dyn ServerHost, def: Option<Ref<TankDef>>) {
        let Some(def) = def else { return };
        let Some(bot) = self.new_tank(h, def, None) else { return };
        let pos = self.spawn_point(h, "Green");
        h.set_transform(bot, &pos);
        let ai = h.data().tanks[def].ai;
        h.set_ai(bot, ai);
        h.set_team(bot, "Green");
    }

    /// A player's solo score now (0 when unset).
    pub(crate) fn solo_score(&self, c: Option<ClientId>) -> f64 {
        self.rec(c, |r| r.spscore.unwrap_or(0.0))
    }

    /// Sets a solo score and shows it.
    pub(crate) fn set_solo_score(&mut self, h: &mut dyn ServerHost, c: Option<ClientId>, score: f64) {
        self.with_record(c, |r| r.spscore = Some(score));
        h.hud(c, &HudMsg::Score(super::records::score_text(score)));
    }

    pub fn solo_tank_destroyed(&mut self, h: &mut dyn ServerHost, this: EntityId, killer: Option<EntityId>) {
        let tier = h.tank_def(this).and_then(|t| h.data().tanks[t].tier);
        // A tank without a client: a campaign bot.
        if self.tanks.get(&this).is_none_or(|t| t.client.is_none()) {
            self.campaign.live_bots -= 1.0;
            self.campaign.dead_bots += 1.0;
            let kc = self.client_of_tank(h, killer);
            let credited = kc.is_some() && self.rec(kc, |r| r.lives.unwrap_or(0)) > 0;
            if credited {
                let level = self.campaign.level();
                let start = self.solo_score(kc);
                let mut pts = 0.0f64;
                if let Some(tier) = tier {
                    pts = match tier {
                        Tier::Bronze => level.bronze_points,
                        Tier::Silver => level.silver_points,
                        Tier::Gold => level.gold_points,
                        Tier::Boss => level.boss_points,
                    } as f64;
                    self.with_record(kc, |r| {
                        let n = match tier {
                            Tier::Bronze => &mut r.bronze_kills,
                            Tier::Silver => &mut r.silver_kills,
                            Tier::Gold => &mut r.gold_kills,
                            Tier::Boss => &mut r.boss_tank_killed,
                        };
                        *n = Some(n.unwrap_or(0) + 1);
                    });
                }
                let now = h.sim_time() as i32 as i64;
                // (The kill time at single precision.)
                let last = self.rec(kc, |r| r.last_kill_time.unwrap_or(0)) as f64;
                if now as f32 as f64 - last < level.quick_shot_time as f64 {
                    self.with_record(kc, |r| r.quick_kill = Some(r.quick_kill.unwrap_or(0) + 1));
                    let qk = self.rec(kc, |r| r.quick_kill.unwrap_or(0)) as f64;
                    pts = (pts * 5.0 * qk) as f32 as f64;
                    let text = Value::Str(format!("x{} Quick-shot bonus!", format_g(qk * 5.0)));
                    self.send_to(h, kc, &ServerMsg::BottomPrint { text, secs: "2".into(), lines: "2".into(), lc: false });
                    h.hud(kc, &HudMsg::Sound("SPQuick".into()));
                } else {
                    self.with_record(kc, |r| r.quick_kill = Some(0));
                }
                self.with_record(kc, |r| r.last_kill_time = Some(now));
                let new_score = start + pts;
                self.set_solo_score(h, kc, new_score);
                self.free_lives(h, kc, start, new_score);
            } else {
                self.campaign.dead_bots -= 1.0;
                self.campaign.spawned_bots -= 1.0;
                self.spawn_campaign_bot(h, Some("GoldHeavyTank"));
            }
            return;
        }
        let client = self.tank_client(this);
        self.with_record(client, |r| {
            r.lives = Some(r.lives.unwrap_or(0) - 1);
            r.deaths = Some(r.deaths.unwrap_or(0) + 1);
        });
        let lives = self.rec(client, |r| r.lives.map(|l| l.to_string()).unwrap_or_default());
        h.hud(client, &HudMsg::Lives(lives));
        let demo = self.setup.demo();
        if self.rec(client, |r| r.lives.unwrap_or(0)) > 0 {
            if demo {
                self.respawn_now(h, client);
            } else {
                self.center_print_to(h, client, BRAIN_SEPARATED, "2");
            }
        } else if demo {
            self.respawn_now(h, client);
        } else {
            self.center_print_to(h, client, "G A M E  O V E R", "2");
            let score = self.solo_score(client);
            h.offer_high_score(score);
        }
    }

    /// An extra life for every 10000 points.
    pub fn free_lives(&mut self, h: &mut dyn ServerHost, client: Option<ClientId>, start: f64, score: f64) {
        let tens = |v: f64| (v / 10000.0).floor();
        let free = tens(score) - tens(start);
        if free > 0.0 {
            h.hud(client, &HudMsg::Sound("SPExtra".into()));
            self.with_record(client, |r| r.lives = Some((free + r.lives.unwrap_or(0) as f64) as i32));
            let lives = self.rec(client, |r| r.lives.map(|l| l.to_string()).unwrap_or_default());
            h.hud(client, &HudMsg::Lives(lives));
        }
    }

    pub fn target_practice_done(&mut self, h: &mut dyn ServerHost) {
        // The pause, the tutorial setting and the dialog are the client's
        // (target practice is a local game).
        let group = self.group.clone();
        if group.is_empty() {
            h.hud(None, &HudMsg::TargetPracticeDone);
        }
        for c in group {
            h.hud(Some(c), &HudMsg::TargetPracticeDone);
        }
    }

    pub fn target_destroyed(&mut self, h: &mut dyn ServerHost, this: EntityId, killer: Option<EntityId>) {
        let player_killed = self.client_of_tank(h, Some(this)).is_some();
        let kc = self.client_of_tank(h, killer);
        if kc.is_some() {
            self.score_client(h, kc, 1.0, 1.0);
            self.with_record(kc, |r| r.kills = Some(r.kills.unwrap_or(0) + 1));
            if self.rec(kc, |r| r.kills.unwrap_or(0)) % 10 == 0 {
                let saucer = h.data().tanks.find("TargetSaucer");
                self.spawn_target(h, saucer);
            }
        }
        if player_killed {
            // This notice went to no one.
            self.center_print_to(h, None, BRAIN_SEPARATED, "1");
        } else {
            let def = h.tank_def(this);
            if def.is_none_or(|d| !h.data().tanks[d].name.eq_ignore_ascii_case("TargetSaucer")) {
                self.spawn_target(h, def);
            }
        }
        if self.rec(self.local(), |r| r.kills.unwrap_or(0)) == 5 {
            self.timers.schedule(h, 1500, Timer::TargetPracticeDone);
        }
    }
}
