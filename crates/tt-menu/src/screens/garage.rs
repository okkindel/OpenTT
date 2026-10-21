//! The Garage tab: the tank list, its preview and specs.

use egui::{Align2, Key, Pos2, Rect, Vec2};

use super::gfx::picture;
use super::steel::Plate;
use super::{pal, pressed, Gfx};
use crate::backend::*;
use crate::{MenuAction, MenuState, Screen};

pub(super) fn garage(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let ctx = g.ctx();
    let tanks = b.tanks();
    let mut sel = b.selected_tank().min(tanks.len().saturating_sub(1));
    let step = |sel: usize, d: i32| -> usize {
        let mut i = sel as i32;
        for _ in 0..tanks.len() {
            i = (i + d).rem_euclid(tanks.len().max(1) as i32);
            if !tanks[i as usize].locked {
                break;
            }
        }
        i as usize
    };
    if !g.blocked {
        if pressed(&ctx, Key::ArrowDown) || pressed(&ctx, Key::ArrowRight) {
            sel = step(sel, 1);
        }
        if pressed(&ctx, Key::ArrowUp) || pressed(&ctx, Key::ArrowLeft) {
            sel = step(sel, -1);
        }
    }
    g.caption(g.r(40.0, 104.0, 0.0, 0.0).min, "Tanks");
    let list = g.r(40.0, 134.0, 330.0, 420.0);
    let mut y = list.top();
    for (i, t) in tanks.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(list.left(), y), Vec2::new(list.width(), 74.0));
        let resp = g.interact_button(r, ("tank", i), !t.locked);
        let on = i == sel;
        let down = !t.locked && resp.is_pointer_button_down_on();
        let face = g.plate(r, if on { Plate::Olive } else { Plate::Steel }, false, down, !t.locked, 8.0);
        g.label(face.left_center() + Vec2::new(26.0, 0.0), &t.name, 30.0, if t.locked { pal::DISABLED } else if on { pal::SAND_HI } else { pal::SAND }, Align2::LEFT_CENTER);
        if t.locked {
            g.text(face.right_center() - Vec2::new(20.0, 0.0), "locked", 20.0, pal::RUST, Align2::RIGHT_CENTER);
        }
        if resp.clicked() && !t.locked {
            sel = i;
        }
        y += 84.0;
    }
    // Preview and specs.
    let pv = g.r(400.0, 134.0, 520.0, 420.0);
    let ppp = g.ppp();
    let tex = b.tank_preview(sel, [(pv.width() * ppp) as u32, (pv.height() * ppp) as u32]);
    picture(g, tex, pv, pv.width() / pv.height());
    if tex.is_none() {
        g.text(pv.center(), "3D preview", 26.0, pal::DISABLED, Align2::CENTER_CENTER);
    }
    let specs = g.panel(g.r(940.0, 134.0, 300.0, 420.0));
    if let Some(t) = tanks.get(sel) {
        g.label(specs.min, &t.name, 30.0, pal::SAND, Align2::LEFT_TOP);
        let mut y = specs.top() + 60.0;
        for (l, v) in [("Speed", t.speed), ("Acceleration", t.acceleration), ("Handling", t.handling), ("Armor", t.armor)] {
            g.text(Pos2::new(specs.left(), y), l, 22.0, pal::SAND_MUTED, Align2::LEFT_TOP);
            g.spec_bar(Rect::from_min_size(Pos2::new(specs.left(), y + 28.0), Vec2::new(specs.width(), 12.0)), v);
            y += 64.0;
        }
        if let Some(reason) = tanks.iter().find_map(|t| t.lock_reason.clone()) {
            g.paragraph(Rect::from_min_max(Pos2::new(specs.left(), y + 6.0), specs.max), &format!("Baby Boss: {reason}."), 20.0, pal::RUST);
        }
    }
    if sel != b.selected_tank() && tanks.get(sel).is_some_and(|t| !t.locked) {
        b.select_tank(sel);
    }
    if g.button("ok", g.action(0.0, 200.0), "OK", true, false, true).clicked() || (pressed(&ctx, Key::Enter) && !g.blocked) {
        st.reset(Screen::MainMenu);
    }
    None
}
