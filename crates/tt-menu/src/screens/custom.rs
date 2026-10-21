//! The Custom Game screen: map, mode, round and bots of a local game.

use egui::{Align2, Key, Pos2, Rect, Vec2};

use super::gfx::{native_picture, picture};
use super::selectors::{bot_table, health_packs_selector, map_table, team_selector, world_color, BotPool};
use super::{pal, pressed, Gfx};
use crate::backend::*;
use crate::{MenuAction, MenuState};

pub(super) fn custom_game(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let ctx = g.ctx();
    let missions = b.missions();
    let modes = b.game_modes();
    let tanks = b.tanks();
    let bot_tanks: Vec<TankInfo> = tanks.iter().take(3).cloned().collect();
    let names = b.bot_names();
    let max_bots = b.max_bots();
    let mut cg = st.custom.take().unwrap_or_else(|| CustomGame {
        mission: missions.first().map(|m| m.file.clone()).unwrap_or_default(),
        mode: modes.first().map(|m| m.index).unwrap_or(0),
        bots: names.iter().take(3).enumerate().map(|(i, n)| BotSetup { name: n.clone(), skill: BotSkill::ALL[i % 3], tank: i % bot_tanks.len().max(1), team: Team::Auto }).collect(),
        player_tank: b.selected_tank(),
        player_team: Team::Auto,
        minutes: 15,
        replace_bots: b.replace_bots(),
    });
    // Maps.
    g.caption(g.r(40.0, 104.0, 0.0, 0.0).min, "Map");
    if let Some(i) = map_table(g, "maps", g.r(40.0, 134.0, 380.0, 470.0), &missions, &cg.mission) {
        cg.mission = missions[i].file.clone();
    }
    // Picture + options.
    let mission = missions.iter().find(|m| m.file == cg.mission).cloned();
    g.caption(g.r(440.0, 104.0, 0.0, 0.0).min, mission.as_ref().map(|m| m.name.as_str()).unwrap_or(""));
    let pv = g.r(440.0, 134.0, 400.0, 200.0);
    let ppp = g.ppp();
    match b.mission_preview(&cg.mission, [(pv.width() * ppp) as u32, (pv.height() * ppp) as u32]) {
        Some(t) => picture(g, Some(t), pv, pv.width() / pv.height()),
        None => {
            let inner = pv.shrink(Gfx::BEZEL);
            g.bezel(inner);
            match mission.as_ref().and_then(|m| b.art(&ctx, &Art::Mission(m.id.clone()))) {
                Some(t) => {
                    let size = (t.size_vec2() / ppp).min(inner.size());
                    native_picture(g, &t, inner.center() - size / 2.0, inner.size());
                }
                // A map without a picture (OpenTT's own maps): its name in
                // the empty well.
                None => {
                    if let Some(m) = &mission {
                        g.text(inner.center() - Vec2::new(0.0, 14.0), &m.name, 30.0, pal::DISABLED, Align2::CENTER_CENTER);
                        g.text(inner.center() + Vec2::new(0.0, 20.0), "no picture", 20.0, pal::DISABLED, Align2::CENTER_CENTER);
                    }
                }
            }
            g.bezel_lip(inner);
            if let Some(m) = &mission {
                g.text(Pos2::new(inner.left() + 12.0, inner.bottom() - 8.0), &format!("{} world", m.world), 20.0, world_color(&m.world), Align2::LEFT_BOTTOM);
            }
        }
    }
    let opts = g.panel(g.r(440.0, 344.0, 400.0, 260.0));
    let mode_i = modes.iter().position(|m| m.index == cg.mode).unwrap_or(0);
    let teams = modes.get(mode_i).is_some_and(|m| m.teams);
    let rowh = opts.height() / 5.0;
    let row = |i: usize| Rect::from_min_size(Pos2::new(opts.left() + 100.0, opts.top() + i as f32 * rowh + (rowh - 40.0) / 2.0), Vec2::new(opts.width() - 100.0, 40.0));
    let label = |g: &Gfx, i: usize, t: &str| {
        g.text(Pos2::new(opts.left(), opts.top() + i as f32 * rowh + rowh / 2.0), t, 24.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
    };
    label(g, 0, "Mode");
    let d = g.selector("mode", row(0), modes.get(mode_i).map(|m| m.label.as_str()).unwrap_or("-"), false);
    if d != 0 && !modes.is_empty() {
        cg.mode = modes[(mode_i as i32 + d).rem_euclid(modes.len() as i32) as usize].index;
    }
    label(g, 1, "Round");
    let d = g.selector("minutes", row(1), &format!("{} min", cg.minutes), false);
    cg.minutes = (cg.minutes as i32 + d * if cg.minutes >= 10 { 5 } else { 1 }).clamp(1, 60) as u32;
    label(g, 2, "Health");
    health_packs_selector(g, b, "health", row(2));
    label(g, 3, "Tank");
    let tank_name = tanks.get(cg.player_tank).map(|t| t.name.clone()).unwrap_or_default();
    let d = g.selector("ptank", row(3), &tank_name, false);
    if d != 0 && !tanks.is_empty() {
        let mut i = cg.player_tank as i32;
        for _ in 0..tanks.len() {
            i = (i + d).rem_euclid(tanks.len() as i32);
            if !tanks[i as usize].locked {
                break;
            }
        }
        cg.player_tank = i as usize;
    }
    if teams {
        label(g, 4, "Team");
        team_selector(g, "pteam", row(4), &mut cg.player_team);
    }
    bot_table(g, &mut cg, BotPool { tanks: &bot_tanks, names: &names, max: max_bots }, teams, false, g.r(860.0, 134.0, 380.0, 470.0));
    let back = g.button("back", g.action_left(0.0, 200.0), "Back", false, false, true).clicked();
    let start = g.button("launch", g.action(0.0, 220.0), "Launch", true, false, true).clicked() || (pressed(&ctx, Key::Enter) && ctx.input(|i| i.modifiers.ctrl));
    let out = (start && !g.blocked).then(|| MenuAction::StartCustom(cg.clone()));
    st.custom = Some(cg);
    if back {
        st.back();
    }
    out
}
