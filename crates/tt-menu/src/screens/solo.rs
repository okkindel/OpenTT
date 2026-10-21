//! The solo campaign: the level briefing, the score tally after a level,
//! the level select between runs, and the high scores.

use egui::{Align2, Color32, Key, Pos2, Rect, Vec2};

use super::{pal, pressed, Gfx};
use crate::backend::*;
use crate::{strip_markup, MenuAction, MenuState, Screen};

/// Progress of the solo tally animation (`presentScore` / `advanceScore`).
#[derive(Default)]
pub(crate) struct TallyAnim {
    /// Which tally is being shown (level, start score, final score, steps).
    key: Option<(u32, i64, i64, usize)>,
    /// When it started (egui time, seconds).
    t0: f64,
    /// Steps already performed (their sounds played).
    played: usize,
    /// Completed or skipped (`finishScore`): show the final state, silently.
    done: bool,
}

impl TallyAnim {
    pub(crate) fn finish(&mut self) {
        self.done = true;
    }
}

/// `presentScore` waits 1000 ms before the first `advanceScore`.
const TALLY_FIRST_MS: f64 = 1000.0;

/// What the tally shows after its first `k` steps.
struct TallyView {
    score: i64,
    lives: u32,
    /// Time since "Bonus Points:" appeared (it fades in over 500 ms), if it did.
    bonus_since: Option<f64>,
    /// Per row kind: shown, kills counted.
    rows: [(bool, i64); 4],
    survival: bool,
    boss: bool,
    completion: bool,
    extra_life_text: bool,
}

fn tally_view(t: &SoloTally, k: usize, elapsed_ms: f64) -> TallyView {
    let mut v = TallyView { score: t.start_score, lives: t.start_lives, bonus_since: None, rows: [(false, 0); 4], survival: false, boss: false, completion: false, extra_life_text: false };
    let mut at = TALLY_FIRST_MS;
    for s in t.steps.iter().take(k) {
        v.score = s.score;
        v.lives = s.lives;
        match s.event {
            TallyEvent::BonusText => v.bonus_since = Some(elapsed_ms - at),
            TallyEvent::ShowRow(r) => v.rows[r as usize].0 = true,
            TallyEvent::Count(r) => v.rows[r as usize].1 += 1,
            TallyEvent::Survival => v.survival = true,
            TallyEvent::Boss => v.boss = true,
            TallyEvent::Completion => v.completion = true,
            _ => {}
        }
        // `soloExtraLifeText`: shown when a life is due, blinking, hidden otherwise.
        v.extra_life_text = match s.event {
            TallyEvent::ExtraLifeStart => true,
            TallyEvent::ExtraLifeFlash => !v.extra_life_text,
            _ => false,
        };
        at += s.delay_ms.max(0) as f64;
    }
    v
}

/// The life icons of `soloPlayGui::showLives`: up to 4 icons, from 5 lives
/// on one icon and "X n".
fn draw_lives(g: &Gfx, ctx: &egui::Context, b: &mut dyn MenuBackend, right_top: Pos2, lives: u32) {
    let icon = b.art(ctx, &Art::Life);
    let n = if lives < 5 { lives } else { 1 };
    let size = 36.0;
    let label = (lives > 4).then(|| format!("x {lives}"));
    let label_w = label.as_ref().map_or(0.0, |l| g.text_width(l, 30.0) + 10.0);
    let w = n as f32 * (size + 6.0) + label_w;
    let mut x = right_top.x - w;
    for _ in 0..n {
        let ir = Rect::from_min_size(Pos2::new(x, right_top.y), Vec2::splat(size));
        match &icon {
            Some(t) => {
                g.painter.image(t.id(), ir, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                g.painter.circle_filled(ir.center(), 14.0, pal::PINK);
            }
        }
        x += size + 6.0;
    }
    if let Some(l) = label {
        g.text(Pos2::new(x + 4.0, right_top.y + size / 2.0), &l, 30.0, pal::SAND, Align2::LEFT_CENTER);
    }
}

pub(super) fn solo(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let ctx = g.ctx();
    let s = b.solo_status();
    let mut launch = pressed(&ctx, Key::Enter);
    // Level select between runs: the chosen level is previewed.
    let picking = !s.in_run && s.levels.len() > 1;
    let reached = s.levels.iter().filter(|l| l.unlocked).map(|l| l.position).max().unwrap_or(1);
    let chosen = if picking {
        let sel = *st.solo_level.get_or_insert(reached);
        s.levels.iter().find(|l| l.position == sel && l.unlocked).or_else(|| s.levels.first())
    } else {
        None
    };
    let (number, mission, title, briefing) = match chosen {
        Some(l) => (l.position, l.mission.clone(), solo_level_title(l), l.briefing.clone()),
        None => (s.level_number, s.mission.clone(), s.title.clone(), s.briefing.clone()),
    };
    g.caption(g.r(40.0, 104.0, 0.0, 0.0).min, &format!("Solo campaign  /  level {number}"));
    let inner = g.panel(g.r(40.0, 134.0, 760.0, 420.0));
    g.label(inner.min, &title, 30.0, pal::SAND, Align2::LEFT_TOP);
    if let Some(l) = chosen {
        g.text(inner.right_top(), &l.level_type, 22.0, pal::RUST, Align2::RIGHT_TOP);
    }
    let mut y = inner.top() + 50.0;
    if let Some(t) = b.art(&ctx, &Art::Mission(mission)) {
        y = g.bezel_picture(&t, Pos2::new(inner.left(), y), Vec2::new(inner.width(), 214.0)).bottom() + 16.0;
    }
    g.paragraph(Rect::from_min_max(Pos2::new(inner.left(), y), inner.max), &strip_markup(&briefing), 22.0, pal::SAND_MUTED);
    if picking {
        let start = solo_level_list(g, st, &s.levels);
        launch |= start;
        let sel = chosen.map_or(1, |l| l.position);
        let back = g.button("back", g.action_left(0.0, 200.0), "Back", false, false, true).clicked();
        let scores = g.button("scores", g.action(260.0, 230.0), "High scores", false, false, true).clicked();
        let label = if sel <= 1 { "Launch".to_string() } else { format!("Start level {sel}") };
        launch |= g.button("launch", g.action(0.0, 240.0), &label, true, false, true).clicked();
        if scores {
            st.open(Screen::HighScores);
            return None;
        }
        if back {
            st.back();
            return None;
        }
        return (launch && !g.blocked).then_some(if sel <= 1 { MenuAction::StartSolo } else { MenuAction::StartSoloAt(sel) });
    }

    // The tally: advance the animation to now and play the sounds of the
    // steps reached (each `advanceScore` with `doSound`).
    let now = ctx.input(|i| i.time);
    let view = s.tally_anim.as_ref().map(|t| {
        let key = (s.level_number, t.start_score, t.steps.last().map_or(t.start_score, |x| x.score), t.steps.len());
        let a = &mut st.tally;
        if a.key != Some(key) {
            *a = TallyAnim { key: Some(key), t0: now, played: 0, done: false };
        }
        let elapsed = (now - a.t0) * 1000.0;
        let mut k = 0;
        let mut at = TALLY_FIRST_MS;
        while k < t.steps.len() && (a.done || elapsed >= at) {
            at += t.steps[k].delay_ms.max(0) as f64;
            k += 1;
        }
        if a.done {
            a.played = k;
        }
        while a.played < k {
            if let Some(snd) = t.steps[a.played].event.sound() {
                g.sounds.push(snd);
            }
            a.played += 1;
        }
        if k < t.steps.len() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        } else {
            a.done = true;
        }
        (tally_view(t, k, if a.done { f64::INFINITY } else { elapsed }), t)
    });

    let side = g.panel(g.r(830.0, 134.0, 410.0, 420.0));
    let (score, lives) = view.as_ref().map_or((s.score, s.lives), |(v, _)| (v.score, v.lives));
    g.text(side.min, "Score", 22.0, pal::SAND_MUTED, Align2::LEFT_TOP);
    g.text(side.min + Vec2::new(0.0, 28.0), &score.to_string(), 40.0, pal::BRASS, Align2::LEFT_TOP);
    g.text(side.right_top(), "Lives", 22.0, pal::SAND_MUTED, Align2::RIGHT_TOP);
    draw_lives(g, &ctx, b, side.right_top() + Vec2::new(0.0, 30.0), lives);
    if let Some((v, t)) = &view {
        if v.extra_life_text {
            g.text(side.right_top() + Vec2::new(0.0, 74.0), "EXTRA LIFE!", 24.0, pal::PINK, Align2::RIGHT_TOP);
        }
        if let Some(since) = v.bonus_since {
            // `soloBonusText.animateAlpha(1, 0, 500)`.
            let a = (since / 500.0).clamp(0.0, 1.0) as f32;
            g.text(Pos2::new(side.left(), side.top() + 104.0), "Bonus points", 22.0, pal::SAND_MUTED.gamma_multiply(a), Align2::LEFT_TOP);
        }
        let mut y = side.top() + 140.0;
        let row_h = 44.0;
        let value = |g: &Gfx, y: f32, v: i64| {
            g.text(Pos2::new(side.right(), y + row_h / 2.0), &v.to_string(), 26.0, pal::SAND, Align2::RIGHT_CENTER);
            g.text(Pos2::new(side.right() - 84.0, y + row_h / 2.0), "=", 24.0, pal::SAND_MUTED, Align2::RIGHT_CENTER);
        };
        for kind in TallyKind::ALL {
            let (shown, mult) = v.rows[kind as usize];
            if !shown {
                continue;
            }
            let per = t.pts_per[kind as usize];
            let icon_box = Rect::from_min_size(Pos2::new(side.left(), y + 4.0), Vec2::new(40.0, row_h - 8.0));
            match b.art(&ctx, &Art::TallyIcon(kind)) {
                Some(tex) => {
                    let sz = tex.size_vec2();
                    let k = (icon_box.width() / sz.x).min(icon_box.height() / sz.y);
                    let r = Rect::from_center_size(icon_box.center(), sz * k);
                    g.painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    g.painter.circle_filled(icon_box.center(), 12.0, pal::SAND_MUTED);
                }
            }
            let cy = y + row_h / 2.0;
            g.text(Pos2::new(side.left() + 52.0, cy), &format!("{per} points"), 22.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
            let m = if kind == TallyKind::Brain { format!("x {mult}/{}", t.max_brains) } else { format!("x {mult}") };
            g.text(Pos2::new(side.left() + 176.0, cy), &m, 24.0, pal::SAND, Align2::LEFT_CENTER);
            value(g, y, mult * per);
            y += row_h;
        }
        for (on, label, pts) in [(v.survival, "Survival bonus", t.survival_pts), (v.boss, "Boss bonus", t.boss_pts), (v.completion, "Completion bonus", t.completion_pts)] {
            if on {
                g.text(Pos2::new(side.left() + 52.0, y + row_h / 2.0), label, 22.0, pal::RUST, Align2::LEFT_CENTER);
                value(g, y, pts);
                y += row_h;
            }
        }
    }
    let back = g.button("back", g.action_left(0.0, 200.0), if s.in_run { "Abandon" } else { "Back" }, false, false, true).clicked();
    let scores = g.button("scores", g.action(240.0, 230.0), "High scores", false, false, true).clicked();
    launch |= g.button("launch", g.action(0.0, 220.0), if s.in_run { "Next level" } else { "Launch" }, true, false, true).clicked();
    if scores {
        st.open(Screen::HighScores);
        return None;
    }
    if back {
        st.back();
        return s.in_run.then_some(MenuAction::LeaveGame);
    }
    (launch && !g.blocked).then_some(MenuAction::StartSolo)
}

/// "Medulla Mesa", "Medulla Mesa II" on the second pass, …
fn solo_level_title(l: &SoloLevel) -> String {
    const ROMAN: [&str; 6] = ["", " II", " III", " IV", " V", " VI"];
    match ROMAN.get(l.cycle.saturating_sub(1) as usize) {
        Some(r) => format!("{}{r}", l.title),
        None => format!("{} ({})", l.title, l.cycle),
    }
}

/// The level select list (right panel): position, map, level type; levels
/// not reached yet are dimmed and can't be chosen. Arrow keys move the
/// selection among the unlocked ones. Returns true on a double click
/// (start that level).
fn solo_level_list(g: &mut Gfx, st: &mut MenuState, levels: &[SoloLevel]) -> bool {
    let ctx = g.ctx();
    g.caption(g.r(830.0, 104.0, 0.0, 0.0).min, "Choose a level");
    let well = g.well(g.r(830.0, 134.0, 410.0, 420.0));
    let body = g.header(well, &[(12.0, "#"), (56.0, "Level"), (318.0, "Type")]);
    let unlocked: Vec<u32> = levels.iter().filter(|l| l.unlocked).map(|l| l.position).collect();
    let mut sel = st.solo_level.unwrap_or(1);
    if !g.blocked && !unlocked.is_empty() {
        let i = unlocked.iter().position(|&p| p == sel).unwrap_or(0);
        if pressed(&ctx, Key::ArrowDown) {
            sel = unlocked[(i + 1).min(unlocked.len() - 1)];
        }
        if pressed(&ctx, Key::ArrowUp) {
            sel = unlocked[i.saturating_sub(1)];
        }
    }
    let rows: Vec<Vec<(f32, String, Color32)>> = levels
        .iter()
        .map(|l| {
            let (c, m) = if l.unlocked { (if l.position == sel { pal::BRASS } else { pal::SAND }, pal::SAND_MUTED) } else { (pal::DISABLED, pal::DISABLED) };
            let kind = if l.unlocked { l.level_type.clone() } else { "Locked".to_string() };
            vec![(12.0, l.position.to_string(), m), (56.0, solo_level_title(l), c), (318.0, kind, m)]
        })
        .collect();
    let row = levels.iter().position(|l| l.position == sel);
    let (clicked, double) = g.table("solo-levels", body, &rows, row, 36.0);
    let mut start = false;
    if let Some(l) = clicked.and_then(|i| levels.get(i)) {
        if l.unlocked {
            sel = l.position;
            start = double;
        }
    }
    st.solo_level = Some(sel);
    start
}

pub(super) fn high_scores(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let ctx = g.ctx();
    let scores = b.high_scores();
    let rank = b.last_run_rank();
    g.caption(g.r(40.0, 104.0, 0.0, 0.0).min, "Solo high scores");
    let well = g.well(g.r(240.0, 134.0, 800.0, 470.0));
    let body = g.header(well, &[(24.0, "#"), (90.0, "Player"), (470.0, "Score"), (680.0, "Level")]);
    let rows: Vec<Vec<(f32, String, Color32)>> = scores
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let c = if rank == Some(i) { pal::BRASS } else { pal::SAND };
            // A run started with the level select shows its start level.
            let level = match s.start_level {
                Some(from) => format!("{}  (from {from})", s.level),
                None => s.level.to_string(),
            };
            vec![(24.0, format!("{}", i + 1), pal::SAND_MUTED), (90.0, s.name.clone(), c), (470.0, s.score.to_string(), c), (680.0, level, pal::SAND_MUTED)]
        })
        .collect();
    g.table("scores", body, &rows, rank, 42.0);
    if g.button("ok", g.action(0.0, 200.0), "OK", true, false, true).clicked() || (pressed(&ctx, Key::Enter) && !g.blocked) {
        st.back();
    }
    None
}
