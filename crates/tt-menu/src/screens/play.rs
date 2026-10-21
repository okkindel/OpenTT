//! The Play tab (the main menu), mission loading and the quit dialog.

use egui::{Align2, Key, Pos2, Rect, Vec2};

use super::steel::{Button, Plate};
use super::{list_keys, pal, pressed, Gfx};
use crate::backend::*;
use crate::{MenuAction, MenuState, Screen};

const MAIN_ITEMS: [(&str, &str); 4] = [
    ("Solo campaign", "Fight through the worlds level by level with three lives. Bots get tougher, bosses guard the brains, and your best runs go into the high score table."),
    ("Quick play", "Jump straight into a deathmatch on a random map, against six bots."),
    ("Custom game", "Any map, any mode: pick the arena, game type, round length and your own bot opponents."),
    ("On-line", "Play with others: join a game on your network or by address, or host one that friends join while you play."),
];

pub(super) fn main_menu(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let ctx = g.ctx();
    let mut chosen = if g.blocked { None } else { list_keys(&ctx, &mut st.focus, MAIN_ITEMS.len()) };
    let list = g.r(40.0, 120.0, 420.0, 440.0);
    let (ih, gap) = (95.0, 20.0);
    let mut y = list.top();
    for (i, (label, _)) in MAIN_ITEMS.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(list.left(), y), Vec2::new(list.width(), ih));
        let resp = g.interact_button(r, ("mode", i), true);
        if resp.hovered() && !g.blocked {
            st.focus = i;
        }
        let on = st.focus == i;
        let down = resp.is_pointer_button_down_on();
        let face = g.plate(r, if on { Plate::Olive } else { Plate::Steel }, false, down, true, 10.0);
        g.label(face.left_center() + Vec2::new(34.0, 0.0), label, 34.0, if on { pal::SAND_HI } else { pal::SAND }, Align2::LEFT_CENTER);
        if resp.clicked() {
            chosen = Some(i);
        }
        y += ih + gap;
    }
    // Details of the focused mode.
    let inner = g.steel_panel(g.r(490.0, 120.0, 750.0, 440.0), 26.0);
    let (label, desc) = MAIN_ITEMS[st.focus.min(3)];
    g.label(inner.min, label, 30.0, pal::SAND, Align2::LEFT_TOP);
    if st.focus == 0 {
        let s = b.solo_status();
        let mut y = inner.top() + 50.0;
        if let Some(t) = b.art(&ctx, &Art::Mission(s.mission.clone())) {
            y = g.bezel_picture(&t, Pos2::new(inner.left(), y), Vec2::new(inner.width(), 230.0)).bottom() + 18.0;
        }
        g.text(Pos2::new(inner.left(), y), &format!("Next: level {}  \u{b7}  {}", s.level_number, s.title), 24.0, pal::RUST, Align2::LEFT_TOP);
        g.paragraph(Rect::from_min_max(Pos2::new(inner.left(), y + 36.0), inner.max), desc, 22.0, pal::SAND_MUTED);
    } else {
        g.paragraph(Rect::from_min_max(inner.min + Vec2::new(0.0, 56.0), inner.max), desc, 26.0, pal::SAND_MUTED);
    }
    let play = Rect::from_min_max(g.stage.right_bottom() - Vec2::new(40.0 + 240.0, 22.0 + 58.0), g.stage.right_bottom() - Vec2::new(40.0, 22.0));
    if g.plate_button("play", play, "Play", Button::PRIMARY, 32.0).clicked() {
        chosen = Some(st.focus);
    }
    // A newer release (the update check): a small steel plate under the list.
    if let Some(tag) = b.update_available() {
        let r = Rect::from_min_size(Pos2::new(g.stage.left() + 40.0, play.top() + 8.0), Vec2::new(250.0, 42.0));
        g.text(Pos2::new(r.right() + 16.0, r.center().y), "A new version is out", 20.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
        if g.plate_button("update", r, &format!("Get {tag}"), Button::PLAIN, 22.0).clicked() {
            b.open_releases();
        }
    }
    match chosen? {
        0 => st.open(Screen::Solo),
        1 => return Some(MenuAction::QuickPlay),
        2 => st.open(Screen::CustomGame),
        _ => {
            st.queried = false;
            st.field = None;
            st.open(Screen::Online);
        }
    }
    None
}

/// Mission loading (`loadingGui`): Escape / Cancel aborts.
pub(super) fn loading(g: &mut Gfx, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let ctx = g.ctx();
    let l = b.loading().unwrap_or(Loading { map: String::new(), phase: "Loading".into(), progress: 0.0 });
    let (inner, y) = g.dialog(Vec2::new(700.0, 300.0), "Loading");
    g.label(inner.min, &l.map, 30.0, pal::SAND, Align2::LEFT_TOP);
    g.text(inner.min + Vec2::new(0.0, 46.0), &l.phase, 22.0, pal::SAND_MUTED, Align2::LEFT_TOP);
    let pb = Rect::from_min_size(inner.min + Vec2::new(0.0, 84.0), Vec2::new(inner.width(), 12.0));
    g.bar(pb, l.progress);
    let cancel = g.button("cancel", Rect::from_min_size(Pos2::new(inner.right() - 170.0, y - 25.0), Vec2::new(170.0, 50.0)), "Cancel", false, false, true).clicked();
    (cancel || pressed(&ctx, Key::Escape)).then_some(MenuAction::LeaveGame)
}

pub(super) fn quit_confirm(g: &mut Gfx, st: &mut MenuState) -> Option<MenuAction> {
    let ctx = g.ctx();
    let mut chosen = list_keys(&ctx, &mut st.focus, 2);
    let (inner, y) = g.dialog(Vec2::new(620.0, 270.0), &format!("Quit {}?", tt_assets::product::NAME));
    g.paragraph(inner, "Your players and scores are saved.", 24.0, pal::SAND_MUTED);
    let right = inner.right();
    if g.button("q-no", Rect::from_min_size(Pos2::new(right - 300.0, y - 25.0), Vec2::new(140.0, 50.0)), "Stay", false, st.focus == 1, true).clicked() {
        chosen = Some(1);
    }
    if g.button("q-yes", Rect::from_min_size(Pos2::new(right - 140.0, y - 25.0), Vec2::new(140.0, 50.0)), "Quit", true, st.focus == 0, true).clicked() {
        chosen = Some(0);
    }
    match chosen? {
        0 => Some(MenuAction::Quit),
        _ => {
            st.back();
            None
        }
    }
}
