//! The Players tab: the player list, the New player and Delete dialogs
//! and the tutorial prompt for a new player.

use egui::{Align2, Color32, Key, Pos2, Rect, Vec2};

use super::tutorial::Slides;
use super::{edit_text, frame, list_keys, pal, pressed, Gfx, Typed};
use crate::backend::*;
use crate::{MenuAction, MenuState, Screen};

pub(super) fn player_select(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let ctx = g.ctx();
    let players = b.players();
    if let Some(cur) = b.current_player() {
        if !st.player_init {
            st.player_init = true;
            st.player_sel = players.iter().position(|p| p.name == cur).unwrap_or(0);
        }
    }
    let mut accept = false;
    if !g.blocked && !players.is_empty() {
        if pressed(&ctx, Key::ArrowDown) {
            st.player_sel = (st.player_sel + 1) % players.len();
        }
        if pressed(&ctx, Key::ArrowUp) {
            st.player_sel = (st.player_sel + players.len() - 1) % players.len();
        }
        accept = pressed(&ctx, Key::Enter);
        if pressed(&ctx, Key::Delete) {
            st.confirm_delete = players.get(st.player_sel).map(|p| p.name.clone());
        }
        if pressed(&ctx, Key::N) && ctx.input(|i| i.modifiers.ctrl) {
            st.new_player = Some(String::new());
        }
    }
    st.player_sel = st.player_sel.min(players.len().saturating_sub(1));
    g.caption(g.r(40.0, 104.0, 0.0, 0.0).min, &format!("Players  {}/{}", players.len(), MAX_PLAYERS));
    let well = g.well(g.r(40.0, 134.0, 1200.0, 420.0));
    let body = g.header(well, &[(24.0, "Name"), (520.0, "Combat ratio"), (760.0, "Combat exp"), (980.0, "Solo hi-score")]);
    let cur = b.current_player();
    let rows: Vec<Vec<(f32, String, Color32)>> = players
        .iter()
        .map(|p| {
            let c = if cur.as_deref() == Some(p.name.as_str()) { pal::BRASS } else { pal::SAND };
            vec![(24.0, p.name.clone(), c), (520.0, p.combat_ratio.clone(), pal::SAND), (760.0, p.combat_exp.clone(), pal::SAND), (980.0, p.solo_high_score.clone(), pal::SAND)]
        })
        .collect();
    let (clicked, dbl) = g.table("players", body, &rows, Some(st.player_sel), 52.0);
    if let Some(i) = clicked {
        st.player_sel = i;
    }
    accept |= dbl;
    if players.is_empty() {
        g.text(body.center(), "No players yet - create one", 26.0, pal::SAND_MUTED, Align2::CENTER_CENTER);
    }
    if let Some(m) = st.message.clone() {
        g.text(Pos2::new(g.stage.left() + 40.0, g.stage.bottom() - 100.0), &m, 22.0, pal::RED, Align2::LEFT_CENTER);
    }
    let can_new = players.len() < MAX_PLAYERS;
    if g.button("new", g.action_left(0.0, 220.0), "New player", false, false, can_new).clicked() && can_new {
        st.new_player = Some(String::new());
    }
    let can_del = players.len() > 1;
    if g.button("del", g.action_left(240.0, 160.0), "Delete", false, false, can_del).clicked() && can_del {
        st.confirm_delete = players.get(st.player_sel).map(|p| p.name.clone());
    }
    // loadPlayerGui's tutorial button: the slides with the selected player
    // (not there when the game data has no tutorial art).
    let can_tut = !players.is_empty();
    if b.tutorial_available() && g.button("tutorial", g.action_left(420.0, 200.0), "Tutorial", false, false, can_tut).clicked() && can_tut && !g.blocked {
        if let Some(p) = players.get(st.player_sel) {
            match b.select_player(&p.name) {
                Ok(()) => {
                    st.reset(Screen::MainMenu);
                    st.open(Screen::Tutorial);
                    st.slides = Slides::default();
                    return None;
                }
                Err(e) => st.message = Some(e),
            }
        }
    }
    accept |= g.button("accept", g.action(0.0, 220.0), "Accept", true, false, !players.is_empty()).clicked();
    if accept && !g.blocked {
        if let Some(p) = players.get(st.player_sel) {
            match b.select_player(&p.name) {
                Ok(()) => return finish_player(st, b),
                Err(e) => st.message = Some(e),
            }
        }
    }
    None
}

pub(super) fn new_player_dialog(ctx: &egui::Context, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let mut name = st.new_player.take().unwrap_or_default();
    let Typed { enter: mut create, escape: mut cancel, .. } = edit_text(ctx, &mut name, 16);
    let msg = st.message.clone();
    frame(ctx, st, egui::Order::Foreground, false, |g, _| {
        let (inner, y) = g.dialog(Vec2::new(660.0, 330.0), "New player");
        g.text(inner.min, "Name", 22.0, pal::SAND_MUTED, Align2::LEFT_TOP);
        g.text_field(Rect::from_min_size(inner.min + Vec2::new(0.0, 30.0), Vec2::new(inner.width(), 54.0)), &name);
        if let Some(m) = &msg {
            g.text(inner.min + Vec2::new(0.0, 96.0), m, 20.0, pal::RED, Align2::LEFT_TOP);
        }
        let right = inner.right();
        cancel |= g.button("np-cancel", Rect::from_min_size(Pos2::new(right - 330.0, y - 25.0), Vec2::new(170.0, 50.0)), "Cancel", false, false, true).clicked();
        create |= g.button("np-ok", Rect::from_min_size(Pos2::new(right - 140.0, y - 25.0), Vec2::new(140.0, 50.0)), "Create", true, false, true).clicked();
    });
    if cancel {
        st.message = None;
        return None;
    }
    if create {
        match b.create_player(&name) {
            Ok(()) => {
                st.message = None;
                st.player_sel = b.players().iter().position(|p| p.name == name.trim()).unwrap_or(0);
                return finish_player(st, b);
            }
            Err(e) => st.message = Some(e),
        }
    }
    st.new_player = Some(name);
    None
}

pub(super) fn delete_dialog(ctx: &egui::Context, st: &mut MenuState, b: &mut dyn MenuBackend) {
    let name = st.confirm_delete.take().unwrap_or_default();
    let (mut yes, mut no) = (false, pressed(ctx, Key::Escape));
    frame(ctx, st, egui::Order::Foreground, false, |g, _| {
        let (inner, y) = g.dialog(Vec2::new(660.0, 300.0), "Delete player");
        g.paragraph(inner, &format!("Delete \"{name}\" and all of their scores?"), 26.0, pal::SAND);
        let right = inner.right();
        no |= g.button("del-no", Rect::from_min_size(Pos2::new(right - 300.0, y - 25.0), Vec2::new(140.0, 50.0)), "Keep", false, false, true).clicked();
        yes |= g.button("del-yes", Rect::from_min_size(Pos2::new(right - 140.0, y - 25.0), Vec2::new(140.0, 50.0)), "Delete", true, false, true).clicked();
    });
    if yes {
        if let Err(e) = b.delete_player(&name) {
            st.message = Some(e);
        }
    } else if !no {
        st.confirm_delete = Some(name);
    }
}

pub(crate) fn finish_player(st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    if b.wants_tutorial_prompt() && b.tutorial_available() {
        st.reset(Screen::TutorialPrompt);
    } else {
        st.reset(Screen::MainMenu);
    }
    None
}

pub(super) fn tutorial_prompt(g: &mut Gfx, st: &mut MenuState) -> Option<MenuAction> {
    let ctx = g.ctx();
    let mut chosen = list_keys(&ctx, &mut st.focus, 2);
    let (inner, y) = g.dialog(Vec2::new(700.0, 330.0), "Welcome, recruit");
    g.paragraph(inner, "Take a quick look at the tutorial and practice on the target range, or jump right into battle?", 26.0, pal::SAND);
    let right = inner.right();
    if g.button("tut-no", Rect::from_min_size(Pos2::new(right - 380.0, y - 25.0), Vec2::new(150.0, 50.0)), "Skip", false, st.focus == 1, true).clicked() {
        chosen = Some(1);
    }
    if g.button("tut-yes", Rect::from_min_size(Pos2::new(right - 210.0, y - 25.0), Vec2::new(210.0, 50.0)), "Tutorial", true, st.focus == 0, true).clicked() {
        chosen = Some(0);
    }
    let c = chosen?;
    st.reset(Screen::MainMenu);
    if c == 0 {
        // The tutorial button: `MarqueeGui.show()` (the slides), Target at the end.
        st.open(Screen::Tutorial);
        st.slides = Slides::default();
    }
    None
}
