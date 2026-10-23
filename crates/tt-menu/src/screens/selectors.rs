//! The game setup pieces shared by the Custom Game and Host screens: the
//! map list, the team and health choices and the bot table.

use egui::{Align2, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};

use super::widgets::CELL;
use super::{pal, Gfx};
use crate::backend::*;

/// Text colour of a map's world.
pub(super) fn world_color(w: &str) -> Color32 {
    match w {
        "Green" => Color32::from_rgb(0x5c, 0xc8, 0x46),
        "Blue" => Color32::from_rgb(0x4a, 0x8c, 0xe0),
        "Red" => Color32::from_rgb(0xd8, 0x5a, 0x40),
        "Moon" => Color32::from_rgb(0xb8, 0xc4, 0xd6),
        _ => pal::SAND_MUTED,
    }
}

/// The map list of the Custom Game and Host screens in a well at `rect`
/// (name, world; the World column as wide as its widest value); returns a
/// clicked row.
pub(crate) fn map_table(g: &mut Gfx, id: &str, rect: Rect, missions: &[MissionInfo], current: &str) -> Option<usize> {
    let well = g.well(rect);
    let xs = g.columns(well.width(), 16.0, 0, &[("Name", missions.iter().map(|m| m.name.clone()).collect()), ("World", missions.iter().map(|m| m.world.clone()).collect())], CELL);
    let body = g.header(well, &[(xs[0], "Name"), (xs[1], "World")]);
    let rows: Vec<Vec<(f32, String, Color32)>> = missions.iter().map(|m| vec![(xs[0], m.name.clone(), pal::SAND), (xs[1], m.world.clone(), world_color(&m.world))]).collect();
    let sel = missions.iter().position(|m| m.file == current);
    g.table(id, body, &rows, sel, 35.0).0
}

/// The Host screen's map list as the rotation: a click adds a map at the
/// end or takes it out (never the last one), a double click makes it the
/// only one; the # column is the order, the first map (highlighted) starts
/// the game.
pub(crate) fn rotation_table(g: &mut Gfx, id: &str, rect: Rect, missions: &[MissionInfo], rotation: &mut Vec<String>) {
    let well = g.well(rect);
    let order = |f: &str| rotation.iter().position(|r| r == f);
    let nums: Vec<String> = missions.iter().map(|m| order(&m.file).map(|i| (i + 1).to_string()).unwrap_or_default()).collect();
    let xs = g.columns(well.width(), 16.0, 1, &[("#", vec!["99".into()]), ("Name", missions.iter().map(|m| m.name.clone()).collect()), ("World", missions.iter().map(|m| m.world.clone()).collect())], CELL);
    let body = g.header(well, &[(xs[0], "#"), (xs[1], "Name"), (xs[2], "World")]);
    let rows: Vec<Vec<(f32, String, Color32)>> = missions
        .iter()
        .zip(nums)
        .map(|(m, n)| {
            let on = !n.is_empty();
            vec![(xs[0], n, pal::BRASS), (xs[1], m.name.clone(), if on { pal::SAND } else { pal::SAND_MUTED }), (xs[2], m.world.clone(), if on { world_color(&m.world) } else { pal::SAND_MUTED })]
        })
        .collect();
    let sel = rotation.first().and_then(|f| missions.iter().position(|m| &m.file == f));
    match g.table(id, body, &rows, sel, 35.0) {
        (Some(i), true) => *rotation = vec![missions[i].file.clone()],
        (Some(i), false) => match order(&missions[i].file) {
            Some(k) if rotation.len() > 1 => {
                rotation.remove(k);
            }
            Some(_) => {}
            None => rotation.push(missions[i].file.clone()),
        },
        _ => {}
    }
}

/// The Auto / Green / Blue team choice.
pub(crate) fn team_selector(g: &mut Gfx, id: &str, rect: Rect, team: &mut Team) {
    let i = Team::ALL.iter().position(|t| t == team).unwrap_or(0);
    if let Some(n) = g.segmented(id, rect, &Team::ALL.map(Team::label), i) {
        *team = Team::ALL[n];
    }
}

/// "Health: at least N" on the Custom Game and Host screens.
pub(crate) fn health_packs_selector(g: &mut Gfx, b: &mut dyn MenuBackend, id: &str, rect: Rect) {
    let n = b.min_health_packs().min(3);
    let text = if n == 0 { "classic".to_string() } else { format!("at least {n}") };
    let d = g.selector(id, rect, &text, false);
    if d != 0 {
        b.set_min_health_packs((n as i32 + d).clamp(0, 3) as u32);
    }
}

/// What the bot table offers: the tanks bots drive, the names for added
/// bots and the most bots a game takes.
#[derive(Clone, Copy)]
pub(crate) struct BotPool<'a> {
    pub(crate) tanks: &'a [TankInfo],
    pub(crate) names: &'a [String],
    pub(crate) max: usize,
}

/// The bot table of the Custom Game and Host screens (name, skill, tank,
/// team; click a cell to cycle; add / remove) with Replace bots, in the
/// well `well` (its caption above it).
pub(crate) fn bot_table(g: &mut Gfx, cg: &mut CustomGame, pool: BotPool, teams: bool, hosted: bool, well: Rect) {
    g.caption(well.min - Vec2::new(0.0, 30.0), &format!("Bots  {}/{}", cg.bots.len(), pool.max));
    if hosted {
        replace_bots_toggle(g, cg, well);
    }
    let bw = g.well(well);
    bot_rows(g, cg, pool, teams, bw)
}

/// "Replace bots" (hosted games only; a custom game is local): humans who
/// join take a bot's place, and the bot comes back when they leave.
fn replace_bots_toggle(g: &mut Gfx, cg: &mut CustomGame, well: Rect) {
    let rb = Rect::from_min_size(Pos2::new(well.right() - 200.0, well.top() - 34.0), Vec2::new(200.0, 30.0));
    let resp = g.interact_button(rb, "replace-bots", true);
    if resp.clicked() {
        cg.replace_bots = !cg.replace_bots;
    }
    let bx = Rect::from_min_size(Pos2::new(rb.right() - 24.0, rb.center().y - 12.0), Vec2::splat(24.0));
    g.check_box(bx, cg.replace_bots, resp.hovered());
    g.text(Pos2::new(bx.left() - 10.0, bx.center().y), "Replace bots", 22.0, if resp.hovered() { pal::SAND } else { pal::SAND_MUTED }, Align2::RIGHT_CENTER);
}

/// The rows of [`bot_table`] in its well `bw`.
fn bot_rows(g: &mut Gfx, cg: &mut CustomGame, pool: BotPool, teams: bool, bw: Rect) {
    let BotPool { tanks: bot_tanks, names, max: max_bots } = pool;
    // Columns as wide as their widest value (every skill, tank and team,
    // not just the ones shown, so they don't move when a cell cycles); the
    // name gets the rest, up to the remove buttons' column on the right.
    const LEAD: f32 = 14.0;
    const GAP: f32 = 12.0;
    const X_COL: f32 = 36.0;
    let width = |g: &Gfx, head: &str, vals: &mut dyn Iterator<Item = String>| vals.map(|v| g.text_width(&v, 22.0)).fold(g.label_width(head, 20.0), f32::max) + 8.0 + GAP;
    let mut fixed = vec![
        ("Skill", width(g, "Skill", &mut BotSkill::ALL.iter().map(|s| s.label().to_string()))),
        ("Tank", width(g, "Tank", &mut bot_tanks.iter().map(|t| t.name.clone()))),
    ];
    if teams {
        fixed.push(("Team", width(g, "Team", &mut Team::ALL.iter().map(|t| t.label().to_string()))));
    }
    let name_w = (bw.width() - LEAD - X_COL - fixed.iter().map(|c| c.1).sum::<f32>()).max(60.0);
    let mut cols = vec![(LEAD, "Name")];
    let mut x = LEAD + name_w;
    let mut cell_x = Vec::new();
    for (h, w) in &fixed {
        cols.push((x, *h));
        cell_x.push((x, *w - GAP + 4.0));
        x += w;
    }
    let body = g.header(bw, &cols);
    // Rows 44 high, lower when the most bots and the add button would not fit.
    let rh = (body.height() / (max_bots as f32 + 1.0)).min(44.0);
    let mut remove = None;
    for (i, bot) in cg.bots.iter_mut().enumerate() {
        let r = Rect::from_min_size(Pos2::new(body.left(), body.top() + i as f32 * rh), Vec2::new(body.width(), rh));
        g.row_fill(r, false, false, i % 2 == 1);
        g.text_fit(Pos2::new(r.left() + LEAD, r.center().y), bot_label(&bot.name), 22.0, pal::SAND, name_w - 8.0);
        let mut cells = vec![bot.skill.label().to_string(), bot_tanks.get(bot.tank).map(|t| t.name.clone()).unwrap_or_default()];
        if teams {
            cells.push(bot.team.label().to_string());
        }
        for (k, (t, (x, w))) in cells.into_iter().zip(cell_x.iter().copied()).enumerate() {
            let cr = Rect::from_min_size(Pos2::new(r.left() + x - 4.0, r.top() + 6.0), Vec2::new(w, r.height() - 12.0));
            let resp = g.interact_button(cr, ("bot", i, k), true);
            if resp.hovered() {
                g.painter.rect_stroke(cr, 2.0, Stroke::new(1.0_f32, pal::BRASS), StrokeKind::Inside);
            }
            g.text(Pos2::new(cr.left() + 4.0, cr.center().y), &t, 22.0, if resp.hovered() { pal::SAND_HI } else { pal::BRASS }, Align2::LEFT_CENTER);
            if resp.clicked() {
                match k {
                    0 => bot.skill = BotSkill::ALL[(BotSkill::ALL.iter().position(|s| *s == bot.skill).unwrap_or(0) + 1) % 3],
                    1 => bot.tank = (bot.tank + 1) % bot_tanks.len().max(1),
                    _ => bot.team = Team::ALL[(Team::ALL.iter().position(|t| *t == bot.team).unwrap_or(0) + 1) % 3],
                }
            }
        }
        // Remove: a small steel plate centred in the last column, the
        // whole cell (row high) its hit area.
        let cell = Rect::from_min_max(Pos2::new(r.right() - X_COL, r.top()), r.right_bottom());
        let resp = g.interact_button(cell, ("botx", i), true);
        g.remove_button(Rect::from_center_size(cell.center(), Vec2::splat(26.0)), resp.hovered(), resp.is_pointer_button_down_on());
        if resp.clicked() {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        cg.bots.remove(i);
    }
    if cg.bots.len() < max_bots {
        let r = Rect::from_min_size(Pos2::new(body.left() + 10.0, body.top() + cg.bots.len() as f32 * rh + 8.0), Vec2::new(150.0, 34.0));
        if g.button("botadd", r, "+ Add bot", false, false, true).clicked() {
            let used: Vec<String> = cg.bots.iter().map(|b| b.name.clone()).collect();
            let name = names.iter().find(|n| !used.contains(n)).cloned().unwrap_or_else(|| format!("Bot {}", cg.bots.len() + 1));
            cg.bots.push(BotSetup { name, skill: BotSkill::Bronze, tank: 0, team: Team::Auto });
        }
    }
}

/// A bot's name in the bot table: every row is a bot, so without "[Bot]".
fn bot_label(name: &str) -> &str {
    let t = name.trim_end();
    match t.len().checked_sub(5).filter(|&k| t.is_char_boundary(k) && t[k..].eq_ignore_ascii_case("[bot]")) {
        Some(k) if !t[..k].trim().is_empty() => t[..k].trim_end(),
        _ => t,
    }
}
