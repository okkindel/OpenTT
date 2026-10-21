//! Over the game: the pause menu and the round results.

use egui::{Align2, Color32, Pos2, Rect, Vec2};

use super::{list_keys, pal, Gfx};
use crate::backend::*;
use crate::{MenuAction, MenuState, Screen};

const PAUSE_ITEMS: [&str; 5] = ["Return to game", "Options", "Help", "Leave game", "Quit"];

pub(super) fn pause(g: &mut Gfx, st: &mut MenuState) -> Option<MenuAction> {
    let ctx = g.ctx();
    let mut chosen = list_keys(&ctx, &mut st.focus, PAUSE_ITEMS.len());
    g.painter.rect_filled(g.screen, 0.0, Color32::from_black_alpha(150));
    let r = Rect::from_center_size(g.stage.center(), Vec2::new(460.0, 470.0));
    g.steel_panel(r, 0.0);
    g.label(r.left_top() + Vec2::new(30.0, 26.0), "Paused", 34.0, pal::SAND, Align2::LEFT_TOP);
    for (i, l) in PAUSE_ITEMS.iter().enumerate() {
        let br = Rect::from_min_size(r.left_top() + Vec2::new(30.0, 90.0 + i as f32 * 70.0), Vec2::new(r.width() - 60.0, 54.0));
        let resp = g.button(("pause", i), br, l, i == 0, st.focus == i, true);
        if resp.hovered() {
            st.focus = i;
        }
        if resp.clicked() {
            chosen = Some(i);
        }
    }
    match chosen? {
        0 => Some(MenuAction::Resume),
        1 => {
            st.open(Screen::Settings);
            None
        }
        2 => {
            st.open(Screen::Help);
            None
        }
        3 => Some(MenuAction::LeaveGame),
        _ => Some(MenuAction::Quit),
    }
}

fn team_colour(t: Option<Team>) -> Color32 {
    match t {
        Some(Team::Green) => crate::theme::TEAM_GREEN,
        Some(Team::Blue) => crate::theme::TEAM_BLUE,
        _ => pal::RED,
    }
}

/// The round results over the game: team totals, the players by score with
/// K/D/A, the time to the next map / its loading, and Leave.
pub(super) fn results(g: &mut Gfx, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let ctx = g.ctx();
    let r = b.round_results().unwrap_or_default();
    g.painter.rect_filled(g.screen, 0.0, Color32::from_black_alpha(170));
    let row_h = 38.0;
    let teams_h = if r.teams.is_empty() { 0.0 } else { 78.0 };
    let rows = r.players.len().clamp(1, 12) as f32;
    let h = (96.0 + teams_h + 40.0 + rows * row_h + 4.0 + 106.0).min(g.stage.height() - 40.0);
    let panel = Rect::from_center_size(g.stage.center(), Vec2::new(860.0, h));
    g.steel_panel(panel, 0.0);
    let inner = panel.shrink2(Vec2::new(30.0, 22.0));
    g.label(inner.left_top() + Vec2::new(0.0, 4.0), "Final scores", 34.0, pal::SAND, Align2::LEFT_TOP);
    if !r.mission.is_empty() {
        g.text(inner.right_top() + Vec2::new(0.0, 12.0), &r.mission, 24.0, pal::SAND_MUTED, Align2::RIGHT_TOP);
    }
    let mut y = inner.top() + 66.0;
    // Team totals: a chip per team, the leader first.
    if !r.teams.is_empty() {
        let n = r.teams.len() as f32;
        let w = (inner.width() - 16.0 * (n - 1.0)) / n;
        for (i, t) in r.teams.iter().enumerate() {
            let c = Rect::from_min_size(Pos2::new(inner.left() + i as f32 * (w + 16.0), y), Vec2::new(w, 62.0));
            g.well(c);
            g.painter.rect_filled(Rect::from_min_size(c.min + Vec2::splat(2.0), Vec2::new(6.0, c.height() - 4.0)), 0.0, team_colour(t.team));
            g.text(c.left_center() + Vec2::new(22.0, 0.0), &t.name, 28.0, pal::SAND, Align2::LEFT_CENTER);
            g.text(c.right_center() - Vec2::new(18.0, 0.0), &t.score.to_string(), 40.0, if i == 0 { pal::BRASS } else { pal::SAND }, Align2::RIGHT_CENTER);
        }
        y += teams_h;
    }
    // The players.
    let well = g.well(Rect::from_min_max(Pos2::new(inner.left(), y), Pos2::new(inner.right(), inner.bottom() - 84.0)));
    let score_label = if r.scrum { "Goals" } else { "Score" };
    // (x of the column, label); the numbers are centred on x + 20.
    let mut cols: Vec<(f32, &str, bool)> = vec![(16.0, "#", false), (64.0, "Player", false), (470.0, score_label, true)];
    if r.kda {
        cols.extend([(580.0, "K", true), (650.0, "D", true), (720.0, "A", true)]);
    }
    let body = g.header(well, &[]);
    for (x, t, num) in &cols {
        let cy = body.top() - 20.0;
        if *num {
            g.text(Pos2::new(well.left() + x + 20.0, cy), t, 22.0, pal::SAND_MUTED, Align2::CENTER_CENTER);
        } else {
            g.text(Pos2::new(well.left() + x, cy), t, 22.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
        }
    }
    for (i, p) in r.players.iter().enumerate() {
        let rr = Rect::from_min_size(Pos2::new(body.left(), body.top() + i as f32 * row_h), Vec2::new(body.width(), row_h));
        if rr.bottom() > body.bottom() + 1.0 {
            break;
        }
        g.row_fill(rr, p.me, false, i % 2 == 1);
        let cy = rr.center().y;
        let c = if p.me { pal::SAND_HI } else { pal::SAND };
        g.text(Pos2::new(rr.left() + 16.0, cy), &(i + 1).to_string(), 24.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
        g.painter.circle_filled(Pos2::new(rr.left() + 46.0, cy), 6.0, team_colour(p.team));
        g.text_fit(Pos2::new(rr.left() + 64.0, cy), &p.name, 26.0, c, 380.0);
        let num = |g: &Gfx, x: f32, v: String, col: Color32| {
            g.text(Pos2::new(rr.left() + x + 20.0, cy), &v, 26.0, col, Align2::CENTER_CENTER);
        };
        num(g, 470.0, p.score.to_string(), c);
        if r.kda {
            num(g, 580.0, p.kills.to_string(), pal::SAND);
            num(g, 650.0, p.deaths.to_string(), pal::SAND);
            num(g, 720.0, p.assists.to_string(), pal::SAND);
        }
    }
    // The next map: the pause, then its loading.
    let bar = Rect::from_min_size(Pos2::new(inner.left(), inner.bottom() - 22.0), Vec2::new(inner.width() - 260.0, 14.0));
    let (label, v) = match (r.loading, r.next_in) {
        (Some(v), _) => (format!("Loading the next map  {}%", (v * 100.0).round()), v),
        (None, Some(s)) => (format!("Next map in {} s", s.ceil().max(0.0) as i64), 0.0),
        (None, None) => ("Next map".to_string(), 0.0),
    };
    g.text(bar.left_top() - Vec2::new(0.0, 8.0), &label, 22.0, pal::SAND_MUTED, Align2::LEFT_BOTTOM);
    g.bar(bar, v);
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    let leave = Rect::from_min_size(Pos2::new(inner.right() - 220.0, inner.bottom() - 50.0), Vec2::new(220.0, 50.0));
    let out = g.button("results-leave", leave, "Leave game", false, false, true).clicked();
    out.then_some(MenuAction::LeaveGame)
}
