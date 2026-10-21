//! The Help tab: How to play and Credits.

use egui::{Align2, Pos2, Rect, Vec2};

use super::{pal, Gfx};
use crate::backend::*;
use crate::{strip_markup, MenuAction, MenuState, Screen};

/// The How to play / Credits tabs on `panel` (outer rect, drawn).
fn help_tabs(g: &mut Gfx, st: &mut MenuState, credits: bool, panel: Rect) {
    if let Some(i) = g.subtabs("htab", &["How to play", "Credits"], usize::from(credits), panel) {
        st.reset(Screen::MainMenu);
        st.open(if i == 1 { Screen::Credits } else { Screen::Help });
    }
}

/// The essentials on one screen: the controls (as currently bound) and the
/// rules in a few lines.
const HELP_RULES: [(&str, &str); 6] = [
    ("Goal", "Destroy the other tanks. Every kill scores a point."),
    ("Power-ups", "Drive through them: weapon power-ups give special shots, health power-ups heal you fully."),
    ("Reload", "Out of shots? Drive through the arch to reload."),
    ("Pads", "Boost and jump pads launch you: escape or charge."),
    ("Scrum", "Grab the ball and drive it into any goal."),
    ("Solo", "Three lives. Quick kills in a row multiply the points."),
];

pub(super) fn help(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    // Controls.
    let outer = g.tab_panel_rect();
    let left_outer = Rect::from_min_size(outer.min, Vec2::new(520.0, outer.height()));
    let left = g.panel(left_outer);
    help_tabs(g, st, false, left_outer);
    g.text(left.left_top(), "Controls", 28.0, pal::BRASS, Align2::LEFT_TOP);
    let lh = 34.0;
    let mut y = left.top() + 48.0;
    for k in b.key_bindings() {
        if y + lh > left.bottom() {
            break;
        }
        g.text(Pos2::new(left.left(), y), &k.action, 22.0, pal::SAND_MUTED, Align2::LEFT_TOP);
        g.text(Pos2::new(left.right(), y), k.keys.first().map(String::as_str).unwrap_or(""), 22.0, pal::SAND, Align2::RIGHT_TOP);
        y += lh;
    }
    // Rules.
    let right = g.panel(Rect::from_min_max(Pos2::new(left_outer.right() + 20.0, outer.top()), outer.max));
    g.text(right.left_top(), "Rules", 28.0, pal::BRASS, Align2::LEFT_TOP);
    let mut y = right.top() + 48.0;
    for (title, text) in HELP_RULES {
        g.text(Pos2::new(right.left(), y), title, 22.0, pal::RUST, Align2::LEFT_TOP);
        let body = Rect::from_min_max(Pos2::new(right.left() + 130.0, y), right.right_bottom());
        y += g.paragraph(body, text, 22.0, pal::SAND).max(lh) + 12.0;
    }
    None
}

pub(super) fn credits(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    // `{product}` in the text: the product name (`tt_assets::product::NAME`).
    let text = strip_markup(&b.credits()).replace("{product}", tt_assets::product::NAME);
    let outer = g.tab_panel_rect();
    let panel = g.panel(outer);
    help_tabs(g, st, true, outer);
    let lh = g.line_height(24.0) + 4.0;
    let off = g.scroll_offset("credits", panel, text.lines().count() as f32 * lh);
    g.clipped(panel, |g| {
        for (i, line) in text.lines().enumerate() {
            let y = panel.top() + i as f32 * lh - off;
            if y + lh >= panel.top() && y <= panel.bottom() {
                g.text(Pos2::new(panel.center().x, y), line, 24.0, if i == 0 { pal::BRASS } else { pal::SAND }, Align2::CENTER_TOP);
            }
        }
    });
    None
}
