//! The Host screen (the Online screen's Host tab): a riveted plate of the
//! server and the rules on the left; on the right the map rotation or the
//! bots (as plates, three to a row) under two tabs, the player's tank and
//! team under them.

use egui::{Align2, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};

use super::online::{default_host, field, tick_box, type_into};
use super::selectors::{health_packs_selector, rotation_table, team_selector, BotPool};
use super::steel::{shade, Plate};
use super::{pal, Gfx};
use crate::backend::*;
use crate::{Field, MenuAction, MenuState};

/// One of the game's options, drawn as a selector in `r`.
#[derive(Clone, Copy)]
enum Opt {
    Mode,
    Round,
    Players,
    Health,
    Tank,
    Team,
    Port,
}

fn opt(g: &mut Gfx, b: &mut dyn MenuBackend, hg: &mut HostGame, o: Opt, r: Rect) {
    let modes = b.game_modes();
    let tanks = b.tanks();
    let mode_i = modes.iter().position(|m| m.index == hg.game.mode).unwrap_or(0);
    match o {
        Opt::Mode => {
            let d = g.selector("hmode", r, modes.get(mode_i).map(|m| m.label.as_str()).unwrap_or("-"), false);
            if d != 0 && !modes.is_empty() {
                hg.game.mode = modes[(mode_i as i32 + d).rem_euclid(modes.len() as i32) as usize].index;
            }
        }
        Opt::Round => {
            let d = g.selector("hminutes", r, &format!("{} min", hg.game.minutes), false);
            hg.game.minutes = (hg.game.minutes as i32 + d * if hg.game.minutes >= 10 { 5 } else { 1 }).clamp(1, 60) as u32;
        }
        Opt::Players => {
            let d = g.selector("hmax", r, &format!("{} max", hg.max_players), false);
            hg.max_players = (hg.max_players as i32 + d).clamp(2, 64) as u32;
        }
        Opt::Health => health_packs_selector(g, b, "hhealth", r),
        Opt::Tank => {
            let name = tanks.get(hg.game.player_tank).map(|t| t.name.clone()).unwrap_or_default();
            let d = g.selector("htank", r, &name, false);
            if d != 0 && !tanks.is_empty() {
                let mut i = hg.game.player_tank as i32;
                for _ in 0..tanks.len() {
                    i = (i + d).rem_euclid(tanks.len() as i32);
                    if !tanks[i as usize].locked {
                        break;
                    }
                }
                hg.game.player_tank = i as usize;
            }
        }
        // Only in a team game.
        Opt::Team => {
            if teams(b, hg) {
                team_selector(g, "hteam", r, &mut hg.game.player_team);
            }
        }
        Opt::Port => {
            let d = g.selector("hport", r, &format!("{}", hg.port), false);
            hg.port = (hg.port as i32 + d).clamp(1024, 65535) as u16;
        }
    }
}

fn teams(b: &dyn MenuBackend, hg: &HostGame) -> bool {
    b.game_modes().iter().find(|m| m.index == hg.game.mode).is_some_and(|m| m.teams)
}

/// Labelled option rows from `top` in `left..right`, `pitch` apart, the
/// controls `label_w` in; returns the bottom.
fn opt_rows(g: &mut Gfx, b: &mut dyn MenuBackend, hg: &mut HostGame, rows: &[(&str, Opt)], left: f32, right: f32, top: f32, pitch: f32, label_w: f32) -> f32 {
    let ctl = (pitch - 8.0).min(40.0);
    for (i, (l, o)) in rows.iter().enumerate() {
        let y = top + i as f32 * pitch;
        g.text(Pos2::new(left, y + pitch / 2.0), l, 22.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
        opt(g, b, hg, *o, Rect::from_min_size(Pos2::new(left + label_w, y + (pitch - ctl) / 2.0), Vec2::new(right - left - label_w, ctl)));
    }
    top + rows.len() as f32 * pitch
}

/// The name and password fields; returns their rects (a click elsewhere
/// leaves them).
fn name_field(g: &mut Gfx, st: &mut MenuState, hg: &HostGame, r: Rect) -> Rect {
    if field(g, "sname", r, &hg.name, "server name", st.field == Some(Field::ServerName), false) {
        st.field = Some(Field::ServerName);
    }
    r
}

fn password_field(g: &mut Gfx, st: &mut MenuState, hg: &HostGame, r: Rect) -> Rect {
    if field(g, "spw", r, &hg.password, "none", st.field == Some(Field::ServerPassword), false) {
        st.field = Some(Field::ServerPassword);
    }
    r
}

/// The rotation list (the first map starts).
fn maps(g: &mut Gfx, b: &mut dyn MenuBackend, hg: &mut HostGame, well: Rect) {
    if hg.rotation.is_empty() {
        hg.rotation.push(hg.game.mission.clone());
    }
    let missions = b.missions();
    rotation_table(g, "hmaps", well, &missions, &mut hg.rotation);
    hg.game.mission = hg.rotation[0].clone();
}

fn bot_pool(b: &dyn MenuBackend) -> (Vec<TankInfo>, Vec<String>, usize) {
    (b.tanks().into_iter().take(3).collect(), b.bot_names(), b.max_bots())
}

pub(super) fn host(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend, inner: Rect) -> Option<MenuAction> {
    let ctx = g.ctx();
    let mut hg = st.host.take().unwrap_or_else(|| b.host_settings().unwrap_or_else(|| default_host(b)));
    if !g.blocked {
        type_into(&ctx, st, Some(&mut hg));
    }
    let fields = create_game(g, st, b, &mut hg, inner);
    // One map: no order to choose (and none left on by an earlier choice).
    hg.rotation_random &= hg.rotation.len() > 1;
    if st.field.is_some() && ctx.input(|i| i.pointer.any_click()) && !ctx.pointer_interact_pos().is_some_and(|p| fields.iter().any(|r| r.contains(p))) {
        st.field = None;
    }
    let back = g.button("back", g.action_left(0.0, 200.0), "Back", false, false, true).clicked();
    let can = !hg.name.trim().is_empty();
    let start = g.button("host", g.action(0.0, 220.0), "Host", true, false, can).clicked() && can;
    let out = (start && !g.blocked).then(|| MenuAction::Host(hg.clone()));
    st.host = Some(hg);
    if back {
        st.back();
    }
    out
}

/// The screen's plates and lists in `inner`; returns the text fields' rects.
fn create_game(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend, hg: &mut HostGame, inner: Rect) -> Vec<Rect> {
    let plate = Rect::from_min_size(inner.min, Vec2::new(400.0, inner.height()));
    let face = g.plate(plate, Plate::Steel, false, false, true, 12.0);
    let (left, right) = (face.left() + 26.0, face.right() - 26.0);
    let mut y = face.top() + 20.0;
    let label = |g: &Gfx, x: f32, y: f32, t: &str| {
        g.text(Pos2::new(x, y), t, 20.0, pal::SAND_MUTED, Align2::LEFT_TOP);
    };
    label(g, left, y, "Server name");
    let name_r = name_field(g, st, hg, Rect::from_min_size(Pos2::new(left, y + 24.0), Vec2::new(right - left, 40.0)));
    y += 74.0;
    let half = (right - left - 16.0) / 2.0;
    label(g, left, y, "Password");
    label(g, left + half + 16.0, y, "Port");
    let pw_r = password_field(g, st, hg, Rect::from_min_size(Pos2::new(left, y + 24.0), Vec2::new(half, 40.0)));
    opt(g, b, hg, Opt::Port, Rect::from_min_size(Pos2::new(left + half + 16.0, y + 25.0), Vec2::new(half, 38.0)));
    y += 80.0;
    let rows = [("Mode", Opt::Mode), ("Round", Opt::Round), ("Players", Opt::Players), ("Health", Opt::Health)];
    y = opt_rows(g, b, hg, &rows, left, right, y, 42.0, 90.0) + 4.0;
    tick_box(g, "portmap", Rect::from_min_size(Pos2::new(left, y), Vec2::new(right - left, 28.0)), "Forward the port on the router", &mut hg.port_mapping, true);
    let has_master = !b.master_server().trim().is_empty();
    let public = if has_master { "List online" } else { "List online (set a master in Join)" };
    tick_box(g, "public", Rect::from_min_size(Pos2::new(left, y + 32.0), Vec2::new(right - left, 28.0)), public, &mut hg.public, has_master);
    // Maps or bots, under their tabs; the tick box of the one shown on
    // the tabs' row.
    let rx = plate.right() + 18.0;
    let (tanks, names, max) = bot_pool(b);
    let n = hg.rotation.len().max(1);
    let tabs = [format!("Maps  {n}"), format!("Bots  {}/{}", hg.game.bots.len(), max)];
    let tr = Rect::from_min_size(Pos2::new(rx, inner.top()), Vec2::new(300.0, 34.0));
    if let Some(i) = g.segmented("hlist", tr, &[tabs[0].as_str(), tabs[1].as_str()], st.host_bots as usize) {
        st.host_bots = i == 1;
    }
    let area = Rect::from_min_max(Pos2::new(rx, tr.bottom() + 10.0), Pos2::new(inner.right(), inner.bottom() - 50.0));
    let tick = |g: &Gfx, label: &str| {
        let w = g.text_width(label, 20.0) + 40.0;
        Rect::from_min_size(Pos2::new(inner.right() - w, tr.center().y - 15.0), Vec2::new(w, 30.0))
    };
    if st.host_bots {
        let r = tick(g, "Players who join replace bots");
        tick_box(g, "replace-bots", r, "Players who join replace bots", &mut hg.game.replace_bots, true);
        let teams = teams(b, hg);
        bot_cards(g, &mut hg.game, BotPool { tanks: &tanks, names: &names, max }, teams, area);
    } else {
        maps(g, b, hg, area);
        let r = tick(g, "Random order");
        tick_box(g, "hrandom", r, "Random order", &mut hg.rotation_random, hg.rotation.len() > 1);
    }
    // You.
    let yy = inner.bottom() - 42.0;
    g.text(Pos2::new(rx, yy + 21.0), "You", 22.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
    opt(g, b, hg, Opt::Tank, Rect::from_min_size(Pos2::new(rx + 60.0, yy + 2.0), Vec2::new(240.0, 38.0)));
    opt(g, b, hg, Opt::Team, Rect::from_min_size(Pos2::new(rx + 318.0, yy + 2.0), Vec2::new(inner.right() - rx - 318.0, 38.0)));
    vec![name_r, pw_r]
}

/// The bots as plates, three to a row: the name, then the skill (its
/// medal's colour), the tank and the team, each cycling on a click; a
/// small cross takes the bot out; the last plate adds one.
fn bot_cards(g: &mut Gfx, cg: &mut CustomGame, pool: BotPool, teams: bool, area: Rect) {
    // As high as the most bots and the add slot fill the area.
    let (cols, gap) = (3usize, 10.0);
    let rows = (pool.max + 1).div_ceil(cols).max(1);
    let h = ((area.height() - gap * (rows as f32 - 1.0)) / rows as f32).min(84.0);
    let w = (area.width() - gap * (cols as f32 - 1.0)) / cols as f32;
    let cell = |i: usize| Rect::from_min_size(Pos2::new(area.left() + (i % cols) as f32 * (w + gap), area.top() + (i / cols) as f32 * (h + gap)), Vec2::new(w, h));
    let rows_fit = ((area.height() + gap) / (h + gap)).floor() as usize;
    let mut remove = None;
    for (i, bot) in cg.bots.iter_mut().enumerate().take(rows_fit * cols) {
        let r = cell(i);
        let face = g.plate(r, Plate::Steel, false, false, false, 6.0);
        let name = bot.name.trim_end().strip_suffix("[Bot]").map(str::trim_end).unwrap_or(&bot.name).to_string();
        g.text_fit(Pos2::new(face.left() + 12.0, face.top() + h * 0.3), &name, 24.0, pal::SAND, face.width() - 48.0);
        let x = Rect::from_center_size(Pos2::new(face.right() - 18.0, face.top() + h * 0.3), Vec2::splat(24.0));
        let resp = g.interact_button(x, ("botx", i), true);
        g.remove_button(x, resp.hovered(), resp.is_pointer_button_down_on());
        if resp.clicked() {
            remove = Some(i);
        }
        let skill_c = match bot.skill {
            BotSkill::Gold => pal::BRASS,
            BotSkill::Silver => Color32::from_rgb(0xc8, 0xcc, 0xd0),
            BotSkill::Bronze => pal::RUST,
        };
        let mut cells = vec![(bot.skill.label().to_string(), skill_c), (pool.tanks.get(bot.tank).map(|t| t.name.clone()).unwrap_or_default(), pal::SAND_MUTED)];
        if teams {
            cells.push((bot.team.label().to_string(), pal::SAND_MUTED));
        }
        let mut cx = face.left() + 8.0;
        for (k, (t, c)) in cells.into_iter().enumerate() {
            let tw = g.text_width(&t, 21.0) + 10.0;
            let cr = Rect::from_min_size(Pos2::new(cx, face.bottom() - h * 0.42), Vec2::new(tw, 26.0));
            let resp = g.interact_button(cr, ("bot", i, k), true);
            if resp.hovered() {
                g.painter.rect_stroke(cr, 2.0, Stroke::new(1.0_f32, pal::BRASS), StrokeKind::Inside);
            }
            g.text(Pos2::new(cr.left() + 5.0, cr.center().y), &t, 21.0, if resp.hovered() { pal::SAND_HI } else { c }, Align2::LEFT_CENTER);
            if resp.clicked() {
                match k {
                    0 => bot.skill = BotSkill::ALL[(BotSkill::ALL.iter().position(|s| *s == bot.skill).unwrap_or(0) + 1) % 3],
                    1 => bot.tank = (bot.tank + 1) % pool.tanks.len().max(1),
                    _ => bot.team = Team::ALL[(Team::ALL.iter().position(|t| *t == bot.team).unwrap_or(0) + 1) % 3],
                }
            }
            cx += tw + 4.0;
        }
    }
    if let Some(i) = remove {
        cg.bots.remove(i);
    }
    let n = cg.bots.len();
    // The next free place: an empty slot pressed into the steel.
    if n < pool.max && n < rows_fit * cols {
        let r = cell(n);
        let resp = g.interact_button(r, "botadd", true);
        let hot = resp.hovered();
        g.inset(r, shade(pal::WELL, -0.2), pal::WELL, if hot { pal::BRASS } else { shade(pal::LINE, 0.1) }, 6.0);
        g.text(r.center(), "+ Add bot", 23.0, if hot { pal::SAND_HI } else { pal::SAND_MUTED }, Align2::CENTER_CENTER);
        if resp.clicked() {
            let used: Vec<String> = cg.bots.iter().map(|b| b.name.clone()).collect();
            let name = pool.names.iter().find(|n| !used.contains(n)).cloned().unwrap_or_else(|| format!("Bot {}", n + 1));
            cg.bots.push(BotSetup { name, skill: BotSkill::Bronze, tank: 0, team: Team::Auto });
        }
    }
}
