//! The screens, in the "field garage" look of [`steel`]: riveted steel
//! plates, brass and olive drab like the OpenTT logo, Russo One for headings
//! and labels, the UI font (`tt_gui::uifont`) for body text. Layout
//! happens on a 1280x720 stage centred on the screen (scaled by
//! `pixels_per_point`).

use egui::{Align2, Color32, Key, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Vec2};

use crate::backend::*;
use crate::{sorted_modes, strip_markup, MenuAction, MenuState, Screen, SettingsTab};

mod online;
pub(crate) mod setup;
pub(crate) mod steel;

use steel::Plate;

const STAGE: Vec2 = Vec2::new(1280.0, 720.0);
const TOP_H: f32 = 108.0;
/// Height of the logo in the top bar (points).
const LOGO_H: f32 = 68.0;
/// Top of the logo below the top of the stage.
const LOGO_Y: f32 = 6.0;
/// Height of the name plate under the logo ([`Gfx::name_plate`]); its
/// bottom is [`TOP_H`] - 6, level with the version line on the right.
const PLATE_H: f32 = 22.0;
/// Content offset below the top bar (the layouts were made for an 86-point bar).
const SHIFT: f32 = 22.0;

/// The palette of the "field garage" look ([`steel`]): the materials of the
/// OpenTT logo. Contrast ratios are for text on the panel face (`PANEL`).
pub mod pal {
    use egui::Color32;
    /// The background: dark steel.
    pub const GUNMETAL: Color32 = Color32::from_rgb(0x23, 0x26, 0x2a);
    /// Button plates: olive-tinted steel.
    pub const PLATE: Color32 = Color32::from_rgb(0x35, 0x3a, 0x33);
    /// Content panels: the plate steel, a little darker and calmer.
    pub const PANEL: Color32 = Color32::from_rgb(0x2e, 0x32, 0x2c);
    /// Raised strips on panels (table headers).
    pub const PANEL_HI: Color32 = Color32::from_rgb(0x3a, 0x3f, 0x37);
    /// Wells pressed into the steel (lists, fields, tracks).
    pub const WELL: Color32 = Color32::from_rgb(0x17, 0x19, 0x16);
    /// Thin steel outlines and separators.
    pub const LINE: Color32 = Color32::from_rgb(0x4c, 0x52, 0x47);
    /// Olive drab: selection, the active tab, bar fills (`SAND_HI` on it: 5.4:1).
    pub const OLIVE: Color32 = Color32::from_rgb(0x5b, 0x63, 0x30);
    /// Brass (the logo lettering): primary actions, focus, edges of active
    /// plates, highlighted text (5.6:1).
    pub const BRASS: Color32 = Color32::from_rgb(0xc9, 0xb6, 0x48);
    /// Text (8:1).
    pub const SAND: Color32 = Color32::from_rgb(0xe3, 0xd6, 0xb0);
    /// Text on olive plates.
    pub const SAND_HI: Color32 = Color32::from_rgb(0xf3, 0xea, 0xcd);
    /// Secondary text (5.6:1).
    pub const SAND_MUTED: Color32 = Color32::from_rgb(0xb4, 0xaa, 0x8a);
    /// Disabled text and controls.
    pub const DISABLED: Color32 = Color32::from_rgb(0x74, 0x70, 0x5e);
    /// Rust, lifted for text: secondary info (4.9:1).
    pub const RUST: Color32 = Color32::from_rgb(0xe2, 0x8e, 0x52);
    /// Text on brass (8:1).
    pub const INK: Color32 = Color32::from_rgb(0x1d, 0x1b, 0x10);
    /// Brain-jar pink (rare secondary accent).
    pub const PINK: Color32 = Color32::from_rgb(0xe0, 0x7a, 0xc0);
    /// Errors and warnings (4.8:1).
    pub const RED: Color32 = Color32::from_rgb(0xf5, 0x80, 0x6e);
}

fn pressed(ctx: &egui::Context, k: Key) -> bool {
    ctx.input(|i| i.key_pressed(k))
}

/// Drawing/interaction context for one frame.
pub(crate) struct Gfx<'a> {
    ui: &'a mut egui::Ui,
    painter: Painter,
    stage: Rect,
    screen: Rect,
    /// Interaction disabled (a modal dialog is open on top).
    blocked: bool,
    /// Sounds requested while drawing (button presses, the tally).
    sounds: Vec<UiSound>,
    /// The enabled button under the pointer.
    hover: Option<egui::Id>,
}

impl<'a> Gfx<'a> {
    fn ctx(&self) -> egui::Context {
        self.ui.ctx().clone()
    }

    /// Stage-relative rectangle.
    fn r(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.stage.min + Vec2::new(x, y + SHIFT), Vec2::new(w, h))
    }

    fn ppp(&self) -> f32 {
        self.ui.ctx().pixels_per_point()
    }

    /// The logo ([`crate::logo`]) of the top bar with its top left corner at
    /// `pos`, pre-scaled to its exact pixel size and pixel-aligned. Returns
    /// its rectangle.
    fn logo(&mut self, pos: Pos2) -> Rect {
        let ppp = self.ppp();
        let tex = crate::logo::texture(&self.ctx(), (LOGO_H * ppp).round() as u32);
        let snap = |v: f32| (v * ppp).round() / ppp;
        let r = Rect::from_min_size(Pos2::new(snap(pos.x), snap(pos.y)), tex.size_vec2() / ppp);
        self.painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        r
    }

    /// The logo of the top bar (left edge 40 from the stage's) and, below
    /// it, a [`Gfx::name_plate`] with `name` and `extra`. Returns the
    /// logo's rectangle.
    fn top_logo(&mut self, name: &str, extra: Option<&str>) -> Rect {
        let x0 = self.stage.left() + 40.0;
        let t = self.logo(Pos2::new(x0, self.stage.top() + LOGO_Y));
        let top = self.stage.top() + TOP_H - 6.0 - PLATE_H;
        debug_assert!(top >= t.bottom() + 5.0, "the name plate overlaps the logo");
        self.name_plate(Pos2::new(t.left() + 2.0, top), (t.width() - 4.0).max(240.0), name, extra);
        t
    }

    /// A small dog tag pressed into the header steel (no rivets: it is not
    /// a button): `name` in sand, then `extra` muted after a thin divider.
    /// As wide as its text, at most `max_w`; a long name is cut with "...",
    /// `extra` is kept (and cut itself only when there is no room for it).
    fn name_plate(&self, pos: Pos2, max_w: f32, name: &str, extra: Option<&str>) -> Rect {
        let size = 18.0;
        let pad = 10.0;
        let gap = 9.0;
        let room = max_w - 2.0 * pad;
        let fit = |t: &str, w: f32| -> String {
            if self.text_width(t, size) <= w {
                return t.to_string();
            }
            let mut c: Vec<char> = t.chars().collect();
            while !c.is_empty() && self.text_width(&format!("{}...", c.iter().collect::<String>().trim_end()), size) > w {
                c.pop();
            }
            format!("{}...", c.iter().collect::<String>().trim_end())
        };
        let extra = extra.filter(|e| !e.is_empty()).map(|e| fit(e, room * 0.55));
        let ew = extra.as_ref().map_or(0.0, |e| self.text_width(e, size) + 2.0 * gap + 1.0);
        let name = fit(name, (room - ew).max(24.0));
        let nw = self.text_width(&name, size);
        let r = Rect::from_min_size(pos, Vec2::new((nw + ew + 2.0 * pad).min(max_w), PLATE_H));
        self.inset(r, steel::shade(pal::WELL, -0.2), steel::shade(pal::WELL, 0.12), steel::shade(pal::LINE, 0.05), 4.0);
        let cy = r.center().y + 0.5;
        self.text(Pos2::new(r.left() + pad, cy), &name, size, pal::SAND, Align2::LEFT_CENTER);
        if let Some(e) = extra {
            let x = r.left() + pad + nw + gap;
            self.painter.line_segment([Pos2::new(x, r.top() + 5.0), Pos2::new(x, r.bottom() - 5.0)], Stroke::new(1.0_f32, pal::LINE));
            self.text(Pos2::new(x + 1.0 + gap, cy), &e, size, pal::SAND_MUTED, Align2::LEFT_CENTER);
        }
        r
    }

    // ------------------------------------------------------------ text

    /// Width of `text` at `size` (`size` is the line box height).
    fn text_width(&self, text: &str, size: f32) -> f32 {
        self.painter.layout_no_wrap(text.into(), crate::theme::game_font(size), Color32::WHITE).size().x
    }

    fn line_height(&self, size: f32) -> f32 {
        size
    }

    /// Text in the game lettering (the UI font, cap height 0.62 of `size`,
    /// baseline at 0.8 of the line box); `anchor` positions its box at `pos`.
    fn text(&self, pos: Pos2, text: &str, size: f32, color: Color32, anchor: Align2) -> Rect {
        self.text_in(pos, text, crate::theme::game_font(size), size, color, anchor)
    }

    /// [`Gfx::text`] in `font`.
    fn text_in(&self, pos: Pos2, text: &str, font: egui::FontId, size: f32, color: Color32, anchor: Align2) -> Rect {
        let galley = self.painter.layout_no_wrap(text.into(), font, color);
        let w = galley.size().x;
        let r = anchor.anchor_size(pos, Vec2::new(w, size));
        let baseline = galley.rows.first().and_then(|row| row.glyphs.first()).map(|g| g.pos.y).unwrap_or(size * 0.8);
        // Whole pixels: egui rasterises glyphs at the pixel size, keep them on the grid.
        let ppp = self.ppp();
        let snap = |v: f32| (v * ppp).round() / ppp;
        self.painter.galley(Pos2::new(snap(r.min.x), snap(r.min.y + size * 0.8) - baseline), galley, color);
        r
    }

    /// Text clipped with "..." to `max_w`.
    fn text_fit(&self, pos: Pos2, text: &str, size: f32, color: Color32, max_w: f32) {
        let mut t = text.to_string();
        if self.text_width(&t, size) > max_w {
            while !t.is_empty() && self.text_width(&format!("{t}..."), size) > max_w {
                t.pop();
            }
            t.push_str("...");
        }
        self.text(pos, &t, size, color, Align2::LEFT_CENTER);
    }

    /// Word-wrapped text; returns the height used.
    fn paragraph(&self, rect: Rect, text: &str, size: f32, color: Color32) -> f32 {
        let lh = self.line_height(size) + 3.0;
        let mut y = rect.top();
        for para in text.split('\n') {
            let mut line = String::new();
            for word in para.split(' ') {
                let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if self.text_width(&cand, size) > rect.width() && !line.is_empty() {
                    if y + lh <= rect.bottom() {
                        self.text(Pos2::new(rect.left(), y), &line, size, color, Align2::LEFT_TOP);
                    }
                    y += lh;
                    line = word.to_string();
                } else {
                    line = cand;
                }
            }
            if y + lh <= rect.bottom() {
                self.text(Pos2::new(rect.left(), y), &line, size, color, Align2::LEFT_TOP);
            }
            y += lh;
        }
        y - rect.top()
    }

    fn interact(&mut self, rect: Rect, id: impl std::hash::Hash) -> Response {
        let sense = if self.blocked { Sense::hover() } else { Sense::click_and_drag() };
        self.ui.interact(rect, egui::Id::new(("tt-menu", id)), sense)
    }

    fn interact_button(&mut self, rect: Rect, id: impl std::hash::Hash, enabled: bool) -> Response {
        let resp = self.interact(rect, id);
        if enabled && !self.blocked && resp.hovered() {
            self.hover = Some(resp.id);
            if self.ui.input(|i| i.pointer.primary_pressed()) {
                self.sounds.push(UiSound::ButtonDown);
            }
        }
        resp
    }

    // ------------------------------------------------------------ pieces

    /// Section caption (top-left of the content area).
    fn caption(&self, pos: Pos2, text: &str) {
        self.label(pos, text, 22.0, pal::SAND_MUTED, Align2::LEFT_TOP);
    }

    /// Content panel (calm steel, no rivets); returns its padded inside.
    fn panel(&self, r: Rect) -> Rect {
        self.steel_panel(r, 16.0)
    }

    /// Dark well pressed into the steel (lists, pictures, previews); returns the inside.
    fn well(&self, r: Rect) -> Rect {
        self.inset(r, steel::shade(pal::WELL, -0.25), pal::WELL, steel::shade(pal::LINE, 0.1), 4.0);
        r.shrink(2.0)
    }

    /// Column header row of a table; returns the area below it.
    fn header(&self, inner: Rect, cols: &[(f32, &str)]) -> Rect {
        let h = 40.0;
        let r = Rect::from_min_size(inner.min, Vec2::new(inner.width(), h));
        self.fill_grad(&[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], r.top(), r.bottom(), steel::shade(pal::PANEL_HI, 0.04), steel::shade(pal::PANEL_HI, -0.12));
        self.painter.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, Color32::from_black_alpha(140)));
        for (x, t) in cols {
            self.label(Pos2::new(r.left() + x, r.center().y), t, 20.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
        }
        Rect::from_min_max(Pos2::new(inner.left(), r.bottom()), inner.max)
    }

    /// A riveted plate button: primary = the brass one (one per screen),
    /// others steel; `focused` lights it like the pointer does.
    fn button(&mut self, id: impl std::hash::Hash, rect: Rect, label: &str, primary: bool, focused: bool, enabled: bool) -> Response {
        let size = (rect.height() * 0.5).min(26.0);
        self.plate_button(id, rect, label, primary, focused, enabled, size)
    }

    /// Small square arrow button (a steel plate with a painted triangle).
    fn arrow(&mut self, id: impl std::hash::Hash, rect: Rect, left: bool) -> Response {
        let resp = self.interact_button(rect, (id, left), true);
        let hot = resp.hovered();
        let face = self.plate(rect, Plate::Steel, hot, resp.is_pointer_button_down_on(), false, 4.0);
        let c = face.center();
        let s = rect.height() * 0.2;
        let pts = if left {
            vec![c + Vec2::new(-s, 0.0), c + Vec2::new(s * 0.8, -s), c + Vec2::new(s * 0.8, s)]
        } else {
            vec![c + Vec2::new(s, 0.0), c + Vec2::new(-s * 0.8, -s), c + Vec2::new(-s * 0.8, s)]
        };
        self.painter.add(egui::Shape::convex_polygon(pts, if hot { pal::BRASS } else { pal::SAND_MUTED }, Stroke::NONE));
        resp
    }

    /// `[<] value [>]` selector; returns -1/0/+1.
    fn selector(&mut self, id: &str, rect: Rect, value: &str, focused: bool) -> i32 {
        let bw = rect.height();
        let mid = Rect::from_min_max(Pos2::new(rect.left() + bw + 4.0, rect.top()), Pos2::new(rect.right() - bw - 4.0, rect.bottom()));
        self.inset(mid, steel::shade(pal::WELL, -0.25), pal::WELL, if focused { pal::BRASS } else { steel::shade(pal::LINE, 0.1) }, 3.0);
        let mut d = 0;
        if self.arrow((id, 0), Rect::from_min_size(rect.min, Vec2::splat(bw)), true).clicked() {
            d = -1;
        }
        if self.arrow((id, 1), Rect::from_min_size(Pos2::new(rect.right() - bw, rect.top()), Vec2::splat(bw)), false).clicked() {
            d = 1;
        }
        self.text(mid.center(), value, 24.0, pal::SAND, Align2::CENTER_CENTER);
        d
    }

    /// A check box (an inset square with a brass stud when on) in `b`; `hot` under the pointer.
    fn check_box(&self, b: Rect, on: bool, hot: bool) {
        self.inset(b, steel::shade(pal::WELL, -0.25), pal::WELL, if hot { pal::BRASS } else { steel::shade(pal::LINE, 0.1) }, 3.0);
        if on {
            let s = b.shrink(b.width() * 0.22);
            self.fill_grad(&steel::chamfer(s, 2.0), s.top(), s.bottom(), steel::shade(pal::BRASS, 0.15), steel::shade(pal::BRASS, -0.2));
            self.painter.add(egui::Shape::closed_line(steel::chamfer(s, 2.0), Stroke::new(1.0_f32, Color32::from_black_alpha(120))));
        }
    }

    fn checkbox(&mut self, id: &str, rect: Rect, value: &mut bool) {
        let s = (rect.height() * 0.72).clamp(22.0, 30.0);
        let b = Rect::from_min_size(Pos2::new(rect.left(), rect.center().y - s / 2.0), Vec2::splat(s));
        let resp = self.interact_button(rect, id, true);
        if resp.clicked() {
            *value = !*value;
        }
        self.check_box(b, *value, resp.hovered());
        self.text(Pos2::new(b.right() + 12.0, b.center().y), if *value { "On" } else { "Off" }, 24.0, if *value { pal::SAND } else { pal::SAND_MUTED }, Align2::LEFT_CENTER);
    }

    /// Pick one of `labels`: one recessed track holding the segments edge
    /// to edge (thin dividers); the chosen one is a raised olive plate with
    /// a brass edge sitting in it, the others flat and muted (lighter under
    /// the pointer). A radio dot in each, filled for the chosen one.
    /// Returns the clicked index.
    fn segmented(&mut self, id: &str, rect: Rect, labels: &[&str], sel: usize) -> Option<usize> {
        let n = labels.len().max(1);
        self.inset(rect, steel::shade(pal::WELL, -0.3), steel::shade(pal::WELL, 0.04), steel::shade(pal::LINE, 0.1), 4.0);
        let track = rect.shrink(2.0);
        let w = track.width() / n as f32;
        let seg = |i: usize| Rect::from_min_size(Pos2::new(track.left() + i as f32 * w, track.top()), Vec2::new(w, track.height()));
        // Dividers between two unchosen segments.
        for i in 1..n {
            if i != sel && i != sel + 1 {
                let x = track.left() + i as f32 * w;
                let (t, b) = (track.top() + 6.0, track.bottom() - 6.0);
                self.painter.line_segment([Pos2::new(x, t), Pos2::new(x, b)], Stroke::new(1.0_f32, Color32::from_black_alpha(160)));
                self.painter.line_segment([Pos2::new(x + 1.0, t), Pos2::new(x + 1.0, b)], Stroke::new(1.0_f32, steel::shade(pal::WELL, 0.12)));
            }
        }
        let size = (rect.height() * 0.62).min(23.0);
        let mut out = None;
        for (i, l) in labels.iter().enumerate() {
            let r = seg(i);
            let on = i == sel;
            let resp = self.interact_button(r, (id, i), !on);
            let hot = !on && resp.hovered();
            let face = if on {
                self.plate(r, Plate::Olive, false, false, false, 3.0)
            } else {
                if hot {
                    self.painter.rect_filled(r.shrink(1.0), 2.0, Color32::from_white_alpha(14));
                }
                r
            };
            // The dot and the label, centred together.
            let dot_r = (size * 0.22).max(3.5);
            let tw = self.text_width(l, size);
            let total = dot_r * 2.0 + 8.0 + tw;
            let x0 = face.center().x - total / 2.0;
            let dot = Pos2::new(x0 + dot_r, face.center().y);
            if on {
                self.painter.circle_filled(dot, dot_r + 1.0, Color32::from_black_alpha(140));
                self.painter.circle_filled(dot, dot_r, pal::BRASS);
                self.painter.circle_filled(dot + Vec2::new(-dot_r * 0.3, -dot_r * 0.3), dot_r * 0.35, Color32::from_white_alpha(120));
            } else {
                self.painter.circle_stroke(dot, dot_r, Stroke::new(1.5_f32, if hot { pal::SAND } else { pal::DISABLED }));
            }
            let c = if on {
                pal::SAND_HI
            } else if hot {
                pal::SAND
            } else {
                pal::SAND_MUTED
            };
            self.text(Pos2::new(x0 + dot_r * 2.0 + 8.0, face.center().y), l, size, c, Align2::LEFT_CENTER);
            if resp.clicked() {
                out = Some(i);
            }
        }
        out
    }

    /// The inset olive plate under the active tab or option.
    fn tab_inset(&self, r: Rect) {
        self.inset(r, steel::shade(pal::OLIVE, -0.3), steel::shade(pal::OLIVE, -0.05), steel::shade(pal::BRASS, -0.05), 5.0);
    }

    /// A groove with an olive-to-brass fill (progress, sliders), `v` in 0..1.
    fn bar(&self, track: Rect, v: f32) {
        self.inset(track, steel::shade(pal::WELL, -0.25), pal::WELL, steel::shade(pal::LINE, 0.1), 2.0);
        let w = (track.width() - 2.0) * v.clamp(0.0, 1.0);
        if w > 0.5 {
            let f = Rect::from_min_size(track.min + Vec2::splat(1.0), Vec2::new(w, track.height() - 2.0));
            self.fill_grad(&[f.left_top(), f.right_top(), f.right_bottom(), f.left_bottom()], f.top(), f.bottom(), steel::shade(pal::BRASS, -0.1), steel::shade(pal::OLIVE, -0.1));
        }
    }

    /// Groove with a brass-and-olive fill and a steel knob.
    fn slider(&mut self, id: &str, rect: Rect, value: &mut f32) {
        self.slider_pct(id, rect, value, 0.0, 100.0);
    }

    /// [`Gfx::slider`] whose label shows `lo + value * (hi - lo)` percent.
    fn slider_pct(&mut self, id: &str, rect: Rect, value: &mut f32, lo: f32, hi: f32) {
        let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 8.0));
        let resp = self.interact(rect, id);
        if let Some(p) = resp.interact_pointer_pos() {
            if resp.is_pointer_button_down_on() || resp.clicked() || resp.dragged() {
                *value = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
            }
        }
        let v = value.clamp(0.0, 1.0);
        self.bar(track, v);
        let k = Rect::from_center_size(Pos2::new(track.left() + track.width() * v, track.center().y), Vec2::new(16.0, 28.0));
        let hot = resp.hovered() || resp.dragged();
        self.plate(k, if hot { Plate::Brass } else { Plate::Steel }, false, false, false, 3.0);
        self.text(Pos2::new(rect.right() + 16.0, rect.center().y), &format!("{}%", (lo + v * (hi - lo)).round()), 24.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
    }

    /// Fill of a table row: selected (olive, brass edge), under the pointer, or zebra.
    fn row_fill(&self, r: Rect, selected: bool, hovered: bool, odd: bool) {
        if selected {
            self.painter.rect_filled(r, 0.0, steel::shade(pal::OLIVE, -0.18));
            self.painter.rect_filled(Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, pal::BRASS);
        } else if hovered {
            self.painter.rect_filled(r, 0.0, Color32::from_rgba_premultiplied(25, 27, 13, 70));
        } else if odd {
            self.painter.rect_filled(r, 0.0, Color32::from_rgba_premultiplied(6, 6, 6, 0));
        }
    }

    /// Table rows in `inner`; each cell is (x offset, text, colour). Grey
    /// selection bar, faint zebra stripes. Returns (clicked row, double-clicked).
    #[allow(clippy::type_complexity)]
    fn table(&mut self, id: &str, inner: Rect, rows: &[Vec<(f32, String, Color32)>], sel: Option<usize>, rh: f32) -> (Option<usize>, bool) {
        self.table_sized(id, inner, rows, sel, rh, CELL)
    }

    /// [`Gfx::table`] with text `size`. A cell with a line break shows two
    /// lines: the first in `size`, the second smaller and muted.
    #[allow(clippy::type_complexity)]
    fn table_sized(&mut self, id: &str, inner: Rect, rows: &[Vec<(f32, String, Color32)>], sel: Option<usize>, rh: f32, size: f32) -> (Option<usize>, bool) {
        let mut out = (None, false);
        // Scrolling: rows that don't fit are reached with the mouse wheel;
        // a selection change (keys, code) scrolls it into view.
        let visible = (((inner.height() + 1.0) / rh).floor() as usize).max(1);
        let max_off = rows.len().saturating_sub(visible);
        let key = table_key(id);
        let ctx = self.ctx();
        let (mut off, last_sel): (usize, Option<usize>) = ctx.data(|d| d.get_temp(key)).unwrap_or((0, None));
        if sel != last_sel {
            if let Some(s) = sel {
                if s < off {
                    off = s;
                } else if s >= off + visible {
                    off = s + 1 - visible;
                }
            }
        }
        if !self.blocked && ctx.pointer_hover_pos().is_some_and(|p| inner.contains(p)) {
            let dy = ctx.input(|i| i.raw_scroll_delta.y);
            if dy > 0.5 {
                off = off.saturating_sub(1.max((dy / rh).round() as usize));
            } else if dy < -0.5 {
                off += 1.max((-dy / rh).round() as usize);
            }
        }
        let off = off.min(max_off);
        ctx.data_mut(|d| d.insert_temp(key, (off, sel)));
        // Rows leave room for the scroll bar, drawn after them.
        let row_w = if max_off > 0 { inner.width() - 8.0 } else { inner.width() };
        for (i, row) in rows.iter().enumerate().skip(off).take(visible) {
            let r = Rect::from_min_size(Pos2::new(inner.left(), inner.top() + (i - off) as f32 * rh), Vec2::new(row_w, rh));
            let resp = self.interact(r, (id, i));
            self.row_fill(r, sel == Some(i), resp.hovered(), i % 2 == 1);
            for (k, (x, t, c)) in row.iter().enumerate() {
                // Brass text would not read on the olive selection.
                let c = if sel == Some(i) && *c == pal::BRASS { pal::SAND_HI } else { *c };
                // Each cell ends where the next one starts.
                let end = row.get(k + 1).map_or(r.width(), |n| n.0) - 10.0;
                let w = (end - x).max(20.0);
                match t.split_once('\n') {
                    Some((a, b)) => {
                        let small = size * 0.76;
                        let top = r.center().y - (size + small) / 2.0;
                        self.text_fit(Pos2::new(r.left() + x, top + size / 2.0), a, size, c, w);
                        let muted = if c == pal::DISABLED { c } else { pal::SAND_MUTED };
                        self.text_fit(Pos2::new(r.left() + x, top + size + small / 2.0), b, small, muted, w);
                    }
                    None => self.text_fit(Pos2::new(r.left() + x, r.center().y), t, size, c, w),
                }
            }
            if resp.clicked() {
                out.0 = Some(i);
            }
            if resp.double_clicked() {
                out = (Some(i), true);
            }
        }
        if max_off > 0 {
            let track = Rect::from_min_max(Pos2::new(inner.right() - 5.0, inner.top() + 2.0), Pos2::new(inner.right() - 1.0, inner.bottom() - 2.0));
            let h = (track.height() * visible as f32 / rows.len() as f32).max(16.0);
            let y = track.top() + (track.height() - h) * off as f32 / max_off as f32;
            self.painter.rect_filled(Rect::from_min_size(Pos2::new(track.left(), y), Vec2::new(track.width(), h)), 2.0, pal::LINE);
        }
        out
    }

    /// Segmented spec bar (12 brass cells in a groove).
    fn spec_bar(&self, b: Rect, v: f32) {
        self.inset(b, steel::shade(pal::WELL, -0.25), pal::WELL, steel::shade(pal::LINE, 0.1), 0.0);
        let n = 12;
        let seg = (b.width() - 2.0) / n as f32;
        for k in 0..n {
            if (k as f32 + 0.5) / n as f32 <= v.clamp(0.0, 1.0) {
                let s = Rect::from_min_size(Pos2::new(b.left() + 1.0 + k as f32 * seg + 1.0, b.top() + 2.0), Vec2::new(seg - 2.0, b.height() - 4.0));
                self.fill_grad(&[s.left_top(), s.right_top(), s.right_bottom(), s.left_bottom()], s.top(), s.bottom(), steel::shade(pal::BRASS, 0.1), steel::shade(pal::BRASS, -0.25));
            }
        }
    }

    /// Centred dialog: a steel panel over the dimmed screen, the title in
    /// the display face; returns (content rect, button row centre y).
    fn dialog(&self, size: Vec2, title: &str) -> (Rect, f32) {
        self.painter.rect_filled(self.screen, 0.0, Color32::from_black_alpha(160));
        let r = Rect::from_center_size(self.stage.center(), size);
        self.steel_panel(r, 0.0);
        self.label(r.left_top() + Vec2::new(28.0, 24.0), title, 30.0, pal::SAND, Align2::LEFT_TOP);
        (Rect::from_min_max(r.min + Vec2::new(28.0, 76.0), r.max - Vec2::new(28.0, 80.0)), r.bottom() - 46.0)
    }

    fn text_field(&mut self, rect: Rect, text: &str) {
        self.inset(rect, steel::shade(pal::WELL, -0.25), pal::WELL, pal::BRASS, 4.0);
        let end = self.text(rect.left_center() + Vec2::new(12.0, 0.0), text, 30.0, pal::SAND, Align2::LEFT_CENTER);
        let t = self.ui.ctx().input(|i| i.time);
        if (t * 2.0) as i64 % 2 == 0 {
            let x = if text.is_empty() { rect.left() + 13.0 } else { end.right() + 3.0 };
            self.painter.line_segment([Pos2::new(x, rect.top() + 9.0), Pos2::new(x, rect.bottom() - 9.0)], Stroke::new(2.0_f32, pal::BRASS));
        }
        self.ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }

    /// Bottom action row: button rect `w` wide ending `right_off` from the stage's right edge.
    fn action(&self, right_off: f32, w: f32) -> Rect {
        Rect::from_min_size(Pos2::new(self.stage.right() - 40.0 - right_off - w, self.stage.bottom() - 76.0), Vec2::new(w, 54.0))
    }

    fn action_left(&self, left_off: f32, w: f32) -> Rect {
        Rect::from_min_size(Pos2::new(self.stage.left() + 40.0 + left_off, self.stage.bottom() - 76.0), Vec2::new(w, 54.0))
    }

    /// The content panel of a screen with second-level tabs (outer rect):
    /// the tabs stand on its top edge ([`Gfx::subtabs`]).
    fn tab_panel_rect(&self) -> Rect {
        self.r(40.0, 146.0, 1200.0, 458.0)
    }

    /// Second-level tabs: folder-tab plates standing on the top edge of
    /// `panel` (the outer rect of the panel below, already drawn). The
    /// inactive ones are darker plates behind the panel's edge; the active
    /// one is raised, made of the panel's own steel with a brass top edge,
    /// and runs into the panel.
    fn subtabs(&mut self, id: &str, labels: &[&str], active: usize, panel: Rect) -> Option<usize> {
        const H: f32 = 36.0;
        const LIFT: f32 = 4.0;
        let base = panel.top();
        let mut x = panel.left() + 14.0;
        let mut rects = Vec::with_capacity(labels.len());
        for l in labels {
            let w = self.label_width(l, 24.0) + 40.0;
            rects.push(Rect::from_min_max(Pos2::new(x, base - H), Pos2::new(x + w, base)));
            x += w + 4.0;
        }
        let mut out = None;
        for (i, (l, r)) in labels.iter().zip(rects).enumerate() {
            let on = i == active;
            let r = if on { Rect::from_min_max(r.min - Vec2::new(0.0, LIFT), r.max) } else { r };
            let resp = self.interact_button(r, (id, i), !on);
            let hot = !on && resp.hovered();
            self.folder_tab(r, on, hot);
            let (size, c) = if on {
                (24.0, pal::SAND_HI)
            } else if hot {
                (22.0, pal::SAND)
            } else {
                (22.0, pal::SAND_MUTED)
            };
            self.label(Pos2::new(r.center().x, r.center().y + if on { 1.0 } else { 0.0 }), l, size, c, Align2::CENTER_CENTER);
            if resp.clicked() && !on {
                out = Some(i);
            }
        }
        out
    }

    /// Column offsets of a table `inner_w` wide whose first column starts at
    /// `lead`: each column (header, its values in the table text) as wide as
    /// its widest value (text `size`; the first line of a two-line cell) or
    /// header plus a gap, the `flex` one (a name) gets the rest. Room is
    /// left for the scroll bar, so the last column never truncates.
    fn columns(&self, inner_w: f32, lead: f32, flex: usize, cols: &[(&str, Vec<String>)], size: f32) -> Vec<f32> {
        let gap = 22.0;
        let n = cols.len();
        let widths: Vec<f32> = cols
            .iter()
            .enumerate()
            .map(|(i, (h, vals))| {
                let w = vals.iter().map(|v| self.text_width(v.split('\n').next().unwrap_or(""), size)).fold(self.label_width(h, 20.0) + 18.0, f32::max);
                if i + 1 < n {
                    w + gap
                } else {
                    w
                }
            })
            .collect();
        let room = inner_w - lead - 18.0;
        let fixed: f32 = widths.iter().enumerate().filter(|(i, _)| *i != flex).map(|(_, w)| w).sum();
        let mut xs = Vec::with_capacity(cols.len());
        let mut x = lead;
        for (i, w) in widths.iter().enumerate() {
            xs.push(x);
            x += if i == flex { (room - fixed).max(60.0) } else { *w };
        }
        xs
    }

    /// Run `f` with drawing clipped to `clip`.
    fn clipped<R>(&mut self, clip: Rect, f: impl FnOnce(&mut Self) -> R) -> R {
        let old = self.painter.clone();
        self.painter = old.with_clip_rect(clip.intersect(old.clip_rect()));
        let out = f(self);
        self.painter = old;
        out
    }

    /// A vertical scroll region `view` showing content `content_h` high:
    /// the mouse wheel over it scrolls; a thin bar on the right shows where.
    /// The first row a table (`table_sized` with `id`) shows.
    pub(crate) fn table_offset(&self, id: &str) -> usize {
        self.ctx().data(|d| d.get_temp::<(usize, Option<usize>)>(table_key(id))).map_or(0, |v| v.0)
    }

    /// Returns the offset (points) to draw the content at.
    fn scroll_offset(&mut self, id: &str, view: Rect, content_h: f32) -> f32 {
        let key = egui::Id::new(("tt-menu-scroll-px", id));
        let ctx = self.ctx();
        let max = (content_h - view.height()).max(0.0);
        let mut off: f32 = ctx.data(|d| d.get_temp(key)).unwrap_or(0.0);
        if !self.blocked && ctx.pointer_hover_pos().is_some_and(|p| view.contains(p)) {
            off -= ctx.input(|i| i.raw_scroll_delta.y);
        }
        let off = off.clamp(0.0, max);
        ctx.data_mut(|d| d.insert_temp(key, off));
        if max > 0.0 {
            let track = Rect::from_min_max(Pos2::new(view.right() - 4.0, view.top() + 2.0), Pos2::new(view.right(), view.bottom() - 2.0));
            let h = (track.height() * view.height() / content_h).max(16.0);
            let y = track.top() + (track.height() - h) * off / max;
            self.painter.rect_filled(Rect::from_min_size(Pos2::new(track.left(), y), Vec2::new(track.width(), h)), 2.0, pal::LINE);
        }
        off
    }
}

/// Text size of table cells ([`Gfx::table`]).
const CELL: f32 = 26.0;

/// Option rows (the Options tabs): one rhythm for every kind of row. A row
/// is [`OPT_ROW`] high; its label (left), control (from [`OPT_CTL_X`],
/// [`OPT_CTL_H`] high) and value are centred on the row's middle.
const OPT_ROW: f32 = 52.0;
const OPT_CTL_H: f32 = 40.0;
const OPT_CTL_X: f32 = 340.0;
/// Width of every option control but check boxes.
const OPT_CTL_W: f32 = 480.0;

/// The rows of an options panel (`inner`: the panel's padded inside);
/// `first` the first row shown (more rows than fit scroll, a row at a time).
#[derive(Clone, Copy)]
struct OptRows {
    inner: Rect,
    first: usize,
}

impl OptRows {
    fn new(inner: Rect) -> OptRows {
        OptRows { inner, first: 0 }
    }

    /// For `n` rows: the rows that fit are shown, the wheel over the panel
    /// scrolls the rest into view (a bar on the right shows where).
    /// Returns the rows and the indices to draw.
    fn scrolled(self, g: &mut Gfx, id: &str, n: usize) -> (OptRows, std::ops::Range<usize>) {
        let fit = ((self.inner.height() + 1.0) / OPT_ROW).floor().max(1.0) as usize;
        let max = n.saturating_sub(fit);
        let key = egui::Id::new(("tt-menu-opt-rows", id));
        let ctx = g.ctx();
        let mut first: usize = ctx.data(|d| d.get_temp(key)).unwrap_or(0);
        if !g.blocked && ctx.pointer_hover_pos().is_some_and(|p| self.inner.expand(16.0).contains(p)) {
            let dy = ctx.input(|i| i.raw_scroll_delta.y);
            if dy > 0.5 {
                first = first.saturating_sub(1);
            } else if dy < -0.5 {
                first += 1;
            }
        }
        let first = first.min(max);
        ctx.data_mut(|d| d.insert_temp(key, first));
        if max > 0 {
            let track = Rect::from_min_max(Pos2::new(self.inner.right() + 6.0, self.inner.top()), Pos2::new(self.inner.right() + 10.0, self.inner.bottom()));
            g.painter.rect_filled(track, 2.0, Color32::from_black_alpha(90));
            let h = track.height() * fit as f32 / n as f32;
            let y = track.top() + (track.height() - h) * first as f32 / max as f32;
            g.painter.rect_filled(Rect::from_min_size(Pos2::new(track.left(), y), Vec2::new(track.width(), h)), 2.0, pal::LINE);
        }
        (OptRows { first, ..self }, first..(first + fit).min(n))
    }

    fn mid(&self, i: usize) -> f32 {
        self.inner.top() + (i as f32 - self.first as f32) * OPT_ROW + OPT_ROW / 2.0
    }
    fn label(&self, g: &Gfx, i: usize, t: &str) {
        g.text(Pos2::new(self.inner.left(), self.mid(i)), t, 26.0, pal::SAND, Align2::LEFT_CENTER);
    }
    /// The control of row `i`, `w` wide.
    fn ctl(&self, i: usize, w: f32) -> Rect {
        Rect::from_min_size(Pos2::new(self.inner.left() + OPT_CTL_X, self.mid(i) - OPT_CTL_H / 2.0), Vec2::new(w, OPT_CTL_H))
    }
    /// A short note right of row `i`'s control `ctl` (one line, like the
    /// sliders' values; cut with "..." at the panel's edge).
    fn note(&self, g: &Gfx, ctl: Rect, i: usize, t: &str) {
        let x = ctl.right() + 16.0;
        g.text_fit(Pos2::new(x, self.mid(i)), t, 22.0, pal::SAND_MUTED, self.inner.right() - x);
    }
    /// Top of the space below the first `n` rows.
    fn below(&self, n: usize) -> f32 {
        self.inner.top() + n as f32 * OPT_ROW
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

/// Top-level tabs of the launcher.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Play,
    Garage,
    Players,
    Options,
    Help,
}

const TABS: [(Tab, &str); 5] = [(Tab::Play, "Play"), (Tab::Garage, "Garage"), (Tab::Players, "Players"), (Tab::Options, "Options"), (Tab::Help, "Help")];

fn tab_of(s: Screen) -> Option<Tab> {
    Some(match s {
        Screen::MainMenu | Screen::Solo | Screen::HighScores | Screen::CustomGame | Screen::Online => Tab::Play,
        Screen::Garage => Tab::Garage,
        Screen::PlayerSelect | Screen::TutorialPrompt | Screen::Tutorial => Tab::Players,
        Screen::Settings => Tab::Options,
        Screen::Help | Screen::Credits => Tab::Help,
        Screen::Loading | Screen::QuitConfirm | Screen::Pause | Screen::Results => return None,
    })
}

/// Run `f` with a full-screen [`Gfx`] on `layer`.
fn frame<R>(ctx: &egui::Context, st: &mut MenuState, layer: egui::Order, blocked: bool, f: impl FnOnce(&mut Gfx, &mut MenuState) -> R) -> R {
    let screen = ctx.screen_rect();
    let stage = Rect::from_center_size(screen.center(), STAGE);
    let body = |ui: &mut egui::Ui| {
        let painter = ui.painter().clone();
        let mut g = Gfx { ui, painter, stage, screen, blocked, sounds: Vec::new(), hover: None };
        let out = f(&mut g, st);
        st.sounds.append(&mut g.sounds);
        if g.hover.is_some() {
            st.hover_now = g.hover;
        }
        out
    };
    // The screen itself is a frameless central panel (interactive from the
    // first frame); dialogs go on a foreground area above it.
    if layer == egui::Order::Background {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, body).inner
    } else {
        egui::Area::new(egui::Id::new(("tt-menu-layer", layer as u8)))
            .order(layer)
            .fade_in(false)
            .fixed_pos(screen.min)
            .default_size(screen.size())
            .show(ctx, |ui| {
                ui.set_min_size(screen.size());
                body(ui)
            })
            .inner
    }
}

/// Background, title and the tab bar; `picture` (a map picture) shows
/// blurred behind the content. Returns a clicked tab, or `Err` for Quit.
fn chrome(g: &mut Gfx, screen: Screen, b: &dyn MenuBackend, picture: Option<&egui::TextureHandle>) -> Option<Result<Tab, ()>> {
    let s = g.screen;
    g.backdrop(s);
    let top = Rect::from_min_max(s.min, Pos2::new(s.right(), g.stage.top() + TOP_H));
    if let Some(t) = picture {
        let px = t.size_vec2();
        g.backdrop_picture(Rect::from_min_max(Pos2::new(s.left(), top.bottom()), s.max), t.id(), px.x / px.y.max(1.0));
    }
    g.vignette(s);
    g.header_band(top);
    let tank = b.tanks().get(b.selected_tank()).map(|tk| format!("{} tank", tk.name));
    let t = match b.current_player() {
        Some(p) => g.top_logo(&p, tank.as_deref()),
        None => g.top_logo("No player", None),
    };
    let mut out = None;
    let active = tab_of(screen);
    let interactive = active.is_some() && b.current_player().is_some();
    let cy = g.stage.top() + 54.0;
    let quit_w = 112.0;
    // Fit the tabs between the title and the Quit button (text widths
    // depend on the font, so measure).
    let left = (t.right() + 36.0).max(g.stage.left() + 360.0);
    let right = g.stage.right() - 40.0 - quit_w - 24.0;
    let mut size = 30.0;
    let mut pad = 40.0;
    let gap = 6.0;
    for _ in 0..6 {
        let total: f32 = TABS.iter().map(|(_, l)| g.label_width(l, size) + pad + gap).sum();
        if left + total <= right {
            break;
        }
        if pad > 24.0 {
            pad -= 8.0;
        } else {
            size -= 3.0;
        }
    }
    let mut x = left;
    for (tab, label) in TABS {
        let w = g.label_width(label, size) + pad;
        let r = Rect::from_center_size(Pos2::new(x + w / 2.0, cy), Vec2::new(w, 50.0));
        let resp = g.interact_button(r, ("tab", label), interactive && active != Some(tab));
        let on = active == Some(tab);
        if on {
            g.tab_inset(r);
        }
        let c = if on {
            pal::SAND_HI
        } else if interactive && resp.hovered() {
            pal::SAND
        } else {
            pal::SAND_MUTED
        };
        g.label(r.center(), label, size, c, Align2::CENTER_CENTER);
        if interactive && resp.clicked() {
            out = Some(Ok(tab));
        }
        x += w + gap;
    }
    let q = Rect::from_center_size(Pos2::new(g.stage.right() - 40.0 - quit_w / 2.0, cy), Vec2::new(quit_w, 46.0));
    if g.plate_button("top-quit", q, "Quit", false, false, true, 24.0).clicked() {
        out = Some(Err(()));
    }
    out
}

pub(crate) fn ui(st: &mut MenuState, ctx: &egui::Context, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let screen = st.screen();
    if screen != Screen::Solo {
        // Leaving the solo screen completes its tally (`soloPlayGui::hide` → `finishScore`).
        st.tally.finish();
    }
    if !matches!(screen, Screen::Solo | Screen::HighScores) {
        // The level select starts at the highest level reached again.
        st.solo_level = None;
    }
    let message = b.script_message();
    let modal = message.is_some() || st.new_player.is_some() || st.confirm_delete.is_some() || st.password.is_some() || screen == Screen::QuitConfirm;
    let shown = if screen == Screen::QuitConfirm { st.stack.iter().rev().nth(1).copied().unwrap_or(Screen::MainMenu) } else { screen };
    // The Play tab's mission picture, blurred behind the content.
    let backdrop = if shown == Screen::MainMenu && st.focus == 0 {
        let mission = b.solo_status().mission;
        b.art(ctx, &Art::Mission(mission))
    } else {
        None
    };
    let action = frame(ctx, st, egui::Order::Background, modal, |g, st| {
        if !matches!(screen, Screen::Pause | Screen::Results) {
            match chrome(g, shown, b, backdrop.as_ref()) {
                Some(Ok(tab)) => {
                    let target = match tab {
                        Tab::Play => Screen::MainMenu,
                        Tab::Garage => Screen::Garage,
                        Tab::Players => Screen::PlayerSelect,
                        Tab::Options => Screen::Settings,
                        Tab::Help => Screen::Help,
                    };
                    if tab_of(screen) != Some(tab) {
                        st.reset(Screen::MainMenu);
                        if target != Screen::MainMenu {
                            st.open(target);
                        }
                        return None;
                    }
                }
                Some(Err(())) => {
                    st.open(Screen::QuitConfirm);
                    return None;
                }
                None => {}
            }
            let v = b.version();
            g.text(Pos2::new(g.stage.right() - 40.0, g.stage.top() + TOP_H - 6.0), &v, 14.0, pal::SAND_MUTED, Align2::RIGHT_BOTTOM);
            if screen == Screen::QuitConfirm {
                // Draw the screen below the dialog without interaction.
                let below = st.stack.iter().rev().nth(1).copied();
                if let Some(s2) = below {
                    let mut tmp = st.stack.clone();
                    tmp.pop();
                    let saved = std::mem::replace(&mut st.stack, tmp);
                    let _ = draw_screen(g, st, b, s2);
                    st.stack = saved;
                }
                return None;
            }
        }
        draw_screen(g, st, b, screen)
    });
    if screen == Screen::QuitConfirm {
        return frame(ctx, st, egui::Order::Foreground, false, quit_confirm);
    }
    if let Some(m) = message {
        let mut answer = None;
        if pressed(ctx, Key::Enter) {
            answer = Some(true);
        } else if pressed(ctx, Key::Escape) {
            answer = Some(!m.yes_no);
        }
        frame(ctx, st, egui::Order::Foreground, false, |g, _| {
            let (inner, y) = g.dialog(Vec2::new(660.0, 330.0), if m.yes_no { "Question" } else { "Message" });
            g.paragraph(inner, &strip_markup(&m.text), 26.0, pal::SAND);
            let right = inner.right();
            if m.yes_no {
                if g.button("msg-no", Rect::from_min_size(Pos2::new(right - 290.0, y - 25.0), Vec2::new(130.0, 50.0)), "No", false, false, true).clicked() {
                    answer = Some(false);
                }
                if g.button("msg-yes", Rect::from_min_size(Pos2::new(right - 140.0, y - 25.0), Vec2::new(140.0, 50.0)), "Yes", true, false, true).clicked() {
                    answer = Some(true);
                }
            } else if g.button("msg-ok", Rect::from_min_size(Pos2::new(right - 140.0, y - 25.0), Vec2::new(140.0, 50.0)), "OK", true, false, true).clicked() {
                answer = Some(true);
            }
        });
        if let Some(a) = answer {
            b.answer_script_message(a);
        }
        return None;
    }
    if st.new_player.is_some() {
        return new_player_dialog(ctx, st, b);
    }
    if st.confirm_delete.is_some() {
        delete_dialog(ctx, st, b);
        return None;
    }
    if st.password.is_some() {
        return online::password_dialog(ctx, st);
    }
    if std::mem::take(&mut st.esc_eaten) {
        return action;
    }
    if action.is_none() && st.screen() == screen && pressed(ctx, Key::Escape) {
        match screen {
            Screen::Pause => return Some(MenuAction::Resume),
            Screen::Loading | Screen::Results => {}
            Screen::MainMenu => st.open(Screen::QuitConfirm),
            Screen::PlayerSelect if b.current_player().is_none() => st.open(Screen::QuitConfirm),
            Screen::TutorialPrompt => st.reset(Screen::MainMenu),
            _ => {
                if !st.back() {
                    st.reset(Screen::MainMenu);
                }
            }
        }
    }
    action
}

fn draw_screen(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend, screen: Screen) -> Option<MenuAction> {
    match screen {
        Screen::PlayerSelect => player_select(g, st, b),
        Screen::TutorialPrompt | Screen::Tutorial if !b.tutorial_available() => {
            // No tutorial art in the game data: no tutorial.
            st.reset(Screen::MainMenu);
            None
        }
        Screen::TutorialPrompt => tutorial_prompt(g, st),
        Screen::Tutorial => tutorial(g, st, b),
        Screen::MainMenu => main_menu(g, st, b),
        Screen::Garage => garage(g, st, b),
        Screen::Solo => solo(g, st, b),
        Screen::HighScores => high_scores(g, st, b),
        Screen::Loading => loading(g, b),
        Screen::CustomGame => custom_game(g, st, b),
        Screen::Online => online::online(g, st, b),
        Screen::Settings => settings(g, st, b),
        Screen::Help => help(g, st, b),
        Screen::Credits => credits(g, st, b),
        Screen::QuitConfirm => None,
        Screen::Pause => pause(g, st),
        Screen::Results => results(g, b),
    }
}

/// Keyboard handling for a vertical list of `n` entries: `Some(index)` on Enter.
fn list_keys(ctx: &egui::Context, focus: &mut usize, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    if pressed(ctx, Key::ArrowDown) {
        *focus = (*focus + 1) % n;
    }
    if pressed(ctx, Key::ArrowUp) {
        *focus = (*focus + n - 1) % n;
    }
    *focus = (*focus).min(n - 1);
    (pressed(ctx, Key::Enter) || pressed(ctx, Key::Space)).then_some(*focus)
}

/// Returns its rect.
fn native_picture(g: &Gfx, tex: &egui::TextureHandle, min: Pos2, max: Vec2) -> Rect {
    let ppp = g.ppp();
    let px = tex.size_vec2();
    let k = (max.x / (px.x / ppp)).min(max.y / (px.y / ppp)).min(1.0);
    let size = px / ppp * k;
    let snap = |v: f32| (v * ppp).round() / ppp;
    let r = Rect::from_min_size(Pos2::new(snap(min.x), snap(min.y)), Vec2::new(snap(size.x), snap(size.y)));
    g.painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    r
}

/// A picture in a steel bezel filling `r`, cover-fitted (engine-rendered
/// textures made for the size).
fn picture(g: &Gfx, tex: Option<egui::TextureId>, r: Rect, aspect: f32) {
    let inner = r.shrink(Gfx::BEZEL);
    g.bezel(inner);
    if let Some(t) = tex {
        let ra = inner.width() / inner.height();
        let uv = if ra > aspect {
            let h = aspect / ra;
            Rect::from_min_max(Pos2::new(0.0, 0.5 - h / 2.0), Pos2::new(1.0, 0.5 + h / 2.0))
        } else {
            let w = ra / aspect;
            Rect::from_min_max(Pos2::new(0.5 - w / 2.0, 0.0), Pos2::new(0.5 + w / 2.0, 1.0))
        };
        g.painter.image(t, inner, uv, Color32::WHITE);
    }
    g.bezel_lip(inner);
}

// ---------------------------------------------------------------- play (main menu)

const MAIN_ITEMS: [(&str, &str); 4] = [
    ("Solo campaign", "Fight through the worlds level by level with three lives. Bots get tougher, bosses guard the brains, and your best runs go into the high score table."),
    ("Quick play", "Jump straight into a deathmatch on a random map, against six bots."),
    ("Custom game", "Any map, any mode: pick the arena, game type, round length and your own bot opponents."),
    ("On-line", "Play with others: join a game on your network or by address, or host one that friends join while you play."),
];

fn main_menu(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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
    if g.plate_button("play", play, "Play", true, false, true, 32.0).clicked() {
        chosen = Some(st.focus);
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

// ---------------------------------------------------------------- players

fn player_select(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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

fn new_player_dialog(ctx: &egui::Context, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let mut name = st.new_player.take().unwrap_or_default();
    let (mut create, mut cancel) = (false, false);
    ctx.input(|i| {
        for e in &i.events {
            match e {
                egui::Event::Text(t) => {
                    for c in t.chars().filter(|c| !c.is_control()) {
                        if name.chars().count() < 16 {
                            name.push(c);
                        }
                    }
                }
                egui::Event::Key { key: Key::Backspace, pressed: true, .. } => {
                    name.pop();
                }
                egui::Event::Key { key: Key::Enter, pressed: true, .. } => create = true,
                egui::Event::Key { key: Key::Escape, pressed: true, .. } => cancel = true,
                _ => {}
            }
        }
    });
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

fn delete_dialog(ctx: &egui::Context, st: &mut MenuState, b: &mut dyn MenuBackend) {
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

fn tutorial_prompt(g: &mut Gfx, st: &mut MenuState) -> Option<MenuAction> {
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

// ---------------------------------------------------------------- garage

fn garage(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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

// ---------------------------------------------------------------- solo

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

fn solo(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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

// ---------------------------------------------------------------- tutorial (MarqueeGui)

/// The tutorial slides.
const SLIDE_TEXT: [&str; 8] = [
    "Charge in, take aim, take off.",
    "Use the W,A,S,D keys to drive the tank.",
    "Use the mouse to aim and fire the weapon.",
    "Pick up power-ups for special weapons.",
    "Pick up health power-ups to fully heal yourself.",
    "Drive through the ammo station to reload.",
    "When the head pops off the tank is destroyed.",
    "Try target practice now!!!.",
];

/// Height of the picture in the slide bitmaps (406 of 480 lines; a black
/// strip for the text and the buttons follows).
const SLIDE_PICTURE_V: f32 = 1624.0 / 1920.0;

/// A slide stays 5 s, then the next one comes.
const SLIDE_MS: f64 = 5000.0;
const SHELL_FADE_MS: f64 = 150.0;

/// The slide show of `MarqueeScreen` (`nextBmp` / `prevBmp`).
#[derive(Default)]
pub(crate) struct Slides {
    pub(crate) started: bool,
    /// Current slide, 1..8.
    pub(crate) cur: u32,
    /// When it was put up (egui time, s) and its fade-in (ms).
    pub(crate) shown_at: f64,
    pub(crate) fade_in_ms: f64,
    /// Changing to (slide, start time, fade ms): fades out, switches after 1.5 fades.
    pub(crate) pending: Option<(u32, f64, f64)>,
}

impl Slides {
    /// Advance to `now`; returns the slide and its opacity.
    fn update(&mut self, now: f64) -> (u32, f32) {
        if !self.started {
            *self = Slides { started: true, cur: 1, shown_at: now, fade_in_ms: SHELL_FADE_MS, pending: None };
        }
        if let Some((to, t, fade)) = self.pending {
            let ms = (now - t) * 1000.0;
            if ms < 1.5 * fade {
                return (self.cur, (1.0 - ms / fade).clamp(0.0, 1.0) as f32);
            }
            self.cur = to;
            self.shown_at = t + 1.5 * fade / 1000.0;
            self.fade_in_ms = fade;
            self.pending = None;
        }
        let ms = (now - self.shown_at) * 1000.0;
        if ms >= SLIDE_MS && self.cur < 8 {
            self.go(self.cur + 1, now, 2.0 * SHELL_FADE_MS);
        }
        (self.cur, (ms / self.fade_in_ms.max(1.0)).clamp(0.0, 1.0) as f32)
    }

    /// `nextBmp`/`prevBmp` with `fadeTime` (0.5 for the arrow buttons, 2 for the thread).
    fn go(&mut self, to: u32, now: f64, fade_ms: f64) {
        // A change during a fade completes the previous one first.
        if let Some((p, _, _)) = self.pending.take() {
            self.cur = p;
        }
        if (1..=8).contains(&to) {
            self.pending = Some((to, now, fade_ms));
        }
    }
}

fn tutorial(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let ctx = g.ctx();
    let now = ctx.input(|i| i.time);
    let (cur, alpha) = st.slides.update(now);
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
    let shown = st.slides.pending.map_or(cur, |p| p.0);
    g.caption(g.r(40.0, 104.0, 0.0, 0.0).min, &format!("Tutorial  /  {shown} of 8"));
    // The slide (640x480 art with alpha) on a black well, the text below.
    // The picture part of the 640x480 screens (the bottom strip held the
    // text and the buttons): the backdrop, the slide over it with its alpha.
    // MarqueeScreen fades, the backdrop stays.
    let well = g.well(g.r(326.0, 134.0, 628.0, 398.0));
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, SLIDE_PICTURE_V));
    if let Some(tex) = b.art(&ctx, &Art::SlideBackdrop) {
        g.painter.image(tex.id(), well, uv, Color32::WHITE);
    }
    if let Some(tex) = b.art(&ctx, &Art::Slide(cur)) {
        g.painter.image(tex.id(), well, uv, Color32::WHITE.gamma_multiply(alpha));
    }
    g.text(Pos2::new(g.stage.center().x, well.bottom() + 34.0), SLIDE_TEXT[(cur - 1) as usize], 30.0, pal::SAND.gamma_multiply(alpha), Align2::CENTER_CENTER);
    // marqueePrev / marqueeNext (hidden at the ends).
    let mid = well.center().y;
    let mut step = 0;
    if shown > 1 && g.arrow("slide", Rect::from_center_size(Pos2::new(well.left() - 60.0, mid), Vec2::splat(64.0)), true).clicked() {
        step = -1;
    }
    if shown < 8 && g.arrow("slide", Rect::from_center_size(Pos2::new(well.right() + 60.0, mid), Vec2::splat(64.0)), false).clicked() {
        step = 1;
    }
    if !g.blocked {
        if pressed(&ctx, Key::ArrowLeft) {
            step = -1;
        }
        if pressed(&ctx, Key::ArrowRight) {
            step = 1;
        }
    }
    let last = shown == 8;
    let mut go = g.button("tut-go", g.action(0.0, 280.0), if last { "Target practice" } else { "Next" }, true, false, true).clicked();
    go |= !g.blocked && (pressed(&ctx, Key::Enter) || pressed(&ctx, Key::Space));
    let back = g.button("tut-back", g.action_left(0.0, 200.0), "Back", false, false, true).clicked();
    if back {
        if !st.back() {
            st.reset(Screen::MainMenu);
        }
        return None;
    }
    if go {
        if last {
            st.reset(Screen::MainMenu);
            return Some(MenuAction::StartTutorial);
        }
        step = 1;
    }
    if step != 0 {
        // The arrow buttons: `nextBmp(0.5)` / `prevBmp(0.5)`.
        st.slides.go((shown as i32 + step) as u32, now, 0.5 * SHELL_FADE_MS);
    }
    None
}

fn high_scores(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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

/// Mission loading (`loadingGui`): Escape / Cancel aborts.
fn loading(g: &mut Gfx, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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

// ---------------------------------------------------------------- custom game

fn custom_game(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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
                // A map without a picture (OpenTT's own maps): its name in the
                // empty well.
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
    bot_table(g, &mut cg, teams, &bot_tanks, &names, max_bots, false, g.r(860.0, 134.0, 380.0, 470.0));
    let back = g.button("back", g.action_left(0.0, 200.0), "Back", false, false, true).clicked();
    let start = g.button("launch", g.action(0.0, 220.0), "Launch", true, false, true).clicked() || (pressed(&ctx, Key::Enter) && ctx.input(|i| i.modifiers.ctrl));
    let out = (start && !g.blocked).then(|| MenuAction::StartCustom(cg.clone()));
    st.custom = Some(cg);
    if back {
        st.back();
    }
    out
}

/// Text colour of a map's world.
fn world_color(w: &str) -> Color32 {
    match w {
        "Green" => Color32::from_rgb(0x5c, 0xc8, 0x46),
        "Blue" => Color32::from_rgb(0x4a, 0x8c, 0xe0),
        "Red" => Color32::from_rgb(0xd8, 0x5a, 0x40),
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

/// "Health: at least N" on the Custom Game and Host screens. The Auto /
/// Green / Blue team choice. The egui memory key of a table's (scroll
/// offset, last selection).
fn table_key(id: &str) -> egui::Id {
    egui::Id::new(("tt-menu-scroll", id))
}

pub(crate) fn team_selector(g: &mut Gfx, id: &str, rect: Rect, team: &mut Team) {
    let i = Team::ALL.iter().position(|t| t == team).unwrap_or(0);
    if let Some(n) = g.segmented(id, rect, &Team::ALL.map(Team::label), i) {
        *team = Team::ALL[n];
    }
}

pub(crate) fn health_packs_selector(g: &mut Gfx, b: &mut dyn MenuBackend, id: &str, rect: Rect) {
    let n = b.min_health_packs().min(3);
    let text = if n == 0 { "classic".to_string() } else { format!("at least {n}") };
    let d = g.selector(id, rect, &text, false);
    if d != 0 {
        b.set_min_health_packs((n as i32 + d).clamp(0, 3) as u32);
    }
}

/// The bot table of the Custom Game and Host screens (name, skill, tank,
/// team; click a cell to cycle; add / remove) with Replace bots, in the
/// well `well` (its caption above it).
#[allow(clippy::too_many_arguments)]
pub(crate) fn bot_table(g: &mut Gfx, cg: &mut CustomGame, teams: bool, bot_tanks: &[TankInfo], names: &[String], max_bots: usize, hosted: bool, well: Rect) {
    g.caption(well.min - Vec2::new(0.0, 30.0), &format!("Bots  {}/{}", cg.bots.len(), max_bots));
    if hosted {
        replace_bots_toggle(g, cg, well);
    }
    let bw = g.well(well);
    bot_rows(g, cg, teams, bot_tanks, names, max_bots, bw)
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
fn bot_rows(g: &mut Gfx, cg: &mut CustomGame, teams: bool, bot_tanks: &[TankInfo], names: &[String], max_bots: usize, bw: Rect) {
    // Columns as wide as their widest value (every skill, tank and team,
    // not just the ones shown, so they don't move when a cell cycles); the
    // name gets the rest, up to the remove buttons' column on the right.
    const RH: f32 = 44.0;
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
    let mut remove = None;
    for (i, bot) in cg.bots.iter_mut().enumerate() {
        let r = Rect::from_min_size(Pos2::new(body.left(), body.top() + i as f32 * RH), Vec2::new(body.width(), RH));
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
        let r = Rect::from_min_size(Pos2::new(body.left() + 10.0, body.top() + cg.bots.len() as f32 * RH + 8.0), Vec2::new(150.0, 34.0));
        if g.button("botadd", r, "+ Add bot", false, false, true).clicked() {
            let used: Vec<String> = cg.bots.iter().map(|b| b.name.clone()).collect();
            let name = names.iter().find(|n| !used.contains(n)).cloned().unwrap_or_else(|| format!("Bot {}", cg.bots.len() + 1));
            cg.bots.push(BotSetup { name, skill: BotSkill::Bronze, tank: 0, team: Team::Auto });
        }
    }
}

// ---------------------------------------------------------------- settings

fn settings(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
    let ctx = g.ctx();
    let mut s = st.settings.take().unwrap_or_else(|| b.settings());
    let modes = sorted_modes(&b.display_modes(), b.desktop_mode());
    let desktop = b.desktop_mode();
    let bindings = b.key_bindings();
    let mods_supported = *st.mods_supported.get_or_insert_with(|| b.mods().is_some());
    let mut tabs = vec![SettingsTab::Display, SettingsTab::Graphics, SettingsTab::Audio, SettingsTab::Controls, SettingsTab::GameData];
    let mut labels = vec!["Display", "Graphics", "Sound", "Controls", "Game data"];
    if mods_supported {
        tabs.push(SettingsTab::Mods);
        labels.push("Mods");
    }
    let cur_tab = tabs.iter().position(|t| *t == st.settings_tab).unwrap_or(0);
    if pressed(&ctx, Key::Tab) {
        st.settings_tab = tabs[(cur_tab + 1) % tabs.len()];
    }
    let mut action = None;
    let outer = g.tab_panel_rect();
    let panel = g.panel(outer);
    if let Some(i) = g.subtabs("stab", &labels, cur_tab, outer) {
        st.settings_tab = tabs[i];
    }
    if st.settings_tab != SettingsTab::Mods {
        // Read again when the tab opens (mods may have been added meanwhile).
        st.mods = None;
    }
    // Every options tab: the same rows (label, control, value).
    let rows = OptRows::new(panel);
    let label = |g: &Gfx, i: usize, t: &str| rows.label(g, i, t);
    let ctl = |i: usize, w: f32| rows.ctl(i, w);
    match st.settings_tab {
        SettingsTab::Display => {
            label(g, 0, "Resolution");
            let cur = modes.iter().position(|m| (m.width, m.height) == s.resolution).unwrap_or(0);
            let m = modes.get(cur).copied().unwrap_or(desktop);
            let text = format!("{} x {}{}", m.width, m.height, if m == desktop { "  (desktop)" } else { "" });
            let d = g.selector("res", ctl(0, OPT_CTL_W), &text, false);
            if d != 0 && !modes.is_empty() {
                let n = (cur as i32 - d).rem_euclid(modes.len() as i32) as usize;
                s.resolution = (modes[n].width, modes[n].height);
            }
            label(g, 1, "Window");
            let wi = WindowMode::ALL.iter().position(|w| *w == s.window_mode).unwrap_or(0);
            if let Some(n) = g.segmented("win", ctl(1, OPT_CTL_W), &WindowMode::ALL.map(WindowMode::label), wi) {
                s.window_mode = WindowMode::ALL[n];
            }
            label(g, 2, "Detail");
            if let Some(n) = g.segmented("detail", ctl(2, OPT_CTL_W), &["Low", "Medium", "High"], s.detail_level as usize) {
                s.detail_level = n as u8;
            }
            label(g, 3, "VSync");
            g.checkbox("VSync", ctl(3, 160.0), &mut s.vsync);
            // 50..150 % on the slider's 0..1.
            label(g, 4, "HUD size");
            let mut hud = s.hud_scale - 0.5;
            g.slider_pct("hudsize", ctl(4, OPT_CTL_W), &mut hud, 50.0, 150.0);
            s.hud_scale = (hud + 0.5).clamp(0.5, 1.5);
            label(g, 5, "FPS limit");
            // Six choices: a stepper like Resolution (a row of six would not fit the column).
            const LIMITS: [(i32, &str); 6] = [(0, "Monitor rate"), (30, "30 FPS"), (60, "60 FPS"), (120, "120 FPS"), (144, "144 FPS"), (-1, "Unlimited")];
            let cur = LIMITS.iter().position(|l| l.0 == s.max_fps).unwrap_or(0);
            let fps = ctl(5, OPT_CTL_W);
            let d = g.selector("fpslimit", fps, LIMITS[cur].1, false);
            if d != 0 {
                s.max_fps = LIMITS[(cur as i32 + d).clamp(0, LIMITS.len() as i32 - 1) as usize].0;
            }
            label(g, 6, "Shadows");
            if let Some(n) = g.segmented("shadows", ctl(6, OPT_CTL_W), &["Classic", "Soft", "High"], s.shadows.min(2) as usize) {
                s.shadows = n as u8;
            }
            if s.max_fps < 0 && s.vsync {
                // One line, centred on the row like the sliders' values.
                rows.note(g, fps, 5, "Turn VSync off to uncap");
            }
        }
        SettingsTab::Graphics => {
            // Port extras; "Classic" turns them all off. More rows than
            // fit: the panel scrolls (a row at a time).
            let (rows, shown) = rows.scrolled(g, "graphics", 10);
            let label = |g: &Gfx, i: usize, t: &str| rows.label(g, i, t);
            let ctl = |i: usize, w: f32| rows.ctl(i, w);
            for i in shown {
                match i {
                    0 => {
                        label(g, 0, "Look");
                        let modern = s.glass_jar && s.hdr && s.normal_maps && s.ssao > 0 && s.dynamic_lights && s.shadows > 0;
                        let classic = !s.glass_jar && !s.hdr && !s.normal_maps && s.ssao == 0 && !s.dynamic_lights && s.shadows == 0;
                        let cur = if classic { 0 } else if modern { 1 } else { 2 };
                        if let Some(n) = g.segmented("gfxpreset", ctl(0, OPT_CTL_W), &["Classic", "Modern", "Custom"], cur) {
                            if n < 2 {
                                s.set_graphics_preset(n == 1);
                            }
                        }
                    }
                    1 => {
                        label(g, 1, "Glass brain jars");
                        g.checkbox("glass", ctl(1, 160.0), &mut s.glass_jar);
                    }
                    2 => {
                        label(g, 2, "HDR and glow");
                        g.checkbox("hdr", ctl(2, 160.0), &mut s.hdr);
                    }
                    3 => {
                        label(g, 3, "Bloom strength");
                        g.slider("bloom", ctl(3, OPT_CTL_W), &mut s.bloom);
                    }
                    4 => {
                        label(g, 4, "Normal maps");
                        g.checkbox("normals", ctl(4, 160.0), &mut s.normal_maps);
                    }
                    5 => {
                        // The slider shows 0..100 %: 50 % = the default strength
                        // (SURFACE_DETAIL_DEFAULT), 100 % = twice it. The pref keeps the
                        // actual strength; one above 100 % stays until the slider moves.
                        label(g, 5, "Surface detail");
                        let full = 2.0 * SURFACE_DETAIL_DEFAULT;
                        let mut detail = (s.normal_strength / full).min(1.0);
                        let before = detail;
                        g.slider_pct("surfdetail", ctl(5, OPT_CTL_W), &mut detail, 0.0, 100.0);
                        if detail != before {
                            s.normal_strength = detail * full;
                        }
                    }
                    6 => {
                        label(g, 6, "Ambient occlusion");
                        if let Some(n) = g.segmented("ssao", ctl(6, OPT_CTL_W), &["Off", "Low", "High"], s.ssao.min(2) as usize) {
                            s.ssao = n as u8;
                        }
                    }
                    7 => {
                        label(g, 7, "AO strength");
                        let mut ao = s.ao_strength / 2.0;
                        g.slider_pct("aostrength", ctl(7, OPT_CTL_W), &mut ao, 0.0, 200.0);
                        s.ao_strength = (ao * 2.0).clamp(0.0, 2.0);
                    }
                    8 => {
                        // Dynamic lights: one slider, 0 % = off.
                        label(g, 8, "Dynamic lights");
                        let mut dl = if s.dynamic_lights { s.dynamic_light_strength / 2.0 } else { 0.0 };
                        g.slider_pct("dynlights", ctl(8, OPT_CTL_W), &mut dl, 0.0, 200.0);
                        s.dynamic_light_strength = (dl * 2.0).clamp(0.0, 2.0);
                        s.dynamic_lights = s.dynamic_light_strength > 0.005;
                    }
                    _ => {
                        label(g, 9, "Show FPS");
                        g.checkbox("showfps", ctl(9, 160.0), &mut s.show_fps);
                    }
                }
            }
        }
        SettingsTab::Audio => {
            for (i, (l, v)) in [("Master", &mut s.master_volume), ("Music", &mut s.music_volume), ("Sound effects", &mut s.sound_volume)].into_iter().enumerate() {
                label(g, i, l);
                g.slider(l, ctl(i, OPT_CTL_W), v);
            }
        }
        SettingsTab::Controls => {
            label(g, 0, "Mouse sensitivity");
            g.slider("mouse", ctl(0, OPT_CTL_W), &mut s.mouse_sensitivity);
            label(g, 1, "Invert mouse");
            g.checkbox("invert", ctl(1, 160.0), &mut s.invert_mouse);
            label(g, 2, "Invert steering when reversing");
            g.checkbox("revsteer", ctl(2, 160.0), &mut s.reverse_steering);
            label(g, 3, "Aiming");
            if let Some(n) = g.segmented("aim", ctl(3, OPT_CTL_W), &["Classic", "Camera"], s.aim_mode.min(1) as usize) {
                s.aim_mode = n as u8;
            }
            // The bindings, in two columns below a rule; they scroll if
            // they don't fit (and end with the panel's padding).
            let rule = rows.below(4) + 8.0;
            g.painter.line_segment([Pos2::new(panel.left(), rule), Pos2::new(panel.right(), rule)], Stroke::new(1.0_f32, pal::LINE));
            let view = Rect::from_min_max(Pos2::new(panel.left(), rule + 10.0), panel.max);
            let lh = 30.0;
            let half = bindings.len().div_ceil(2).max(1);
            let off = g.scroll_offset("bindings", view, half as f32 * lh);
            let colw = panel.width() / 2.0;
            g.clipped(view, |g| {
                for (i, k) in bindings.iter().enumerate() {
                    let col = if i < half { 0.0 } else { colw };
                    let cy = view.top() + (i % half) as f32 * lh + lh / 2.0 - off;
                    g.text_fit(Pos2::new(panel.left() + col, cy), &k.action, 22.0, pal::SAND_MUTED, 220.0);
                    g.text_fit(Pos2::new(panel.left() + col + 230.0, cy), &k.keys.join(", "), 22.0, pal::SAND, colw - 250.0);
                }
            });
        }
        SettingsTab::GameData => {
            let info = b.game_data();
            // Values in the controls' column, the same size as the selectors' text.
            let value = |g: &Gfx, i: usize, t: &str| g.text_fit(Pos2::new(panel.left() + OPT_CTL_X, rows.mid(i)), t, 24.0, pal::SAND, panel.width() - OPT_CTL_X);
            let unknown = "not recorded";
            label(g, 0, "Imported from");
            value(g, 0, info.as_ref().map_or(unknown, |i| if i.source.is_empty() { unknown } else { i.source.as_str() }));
            label(g, 1, "Imported on");
            value(g, 1, info.as_ref().map_or(unknown, |i| if i.imported.is_empty() { unknown } else { i.imported.as_str() }));
            label(g, 2, "Stored in");
            value(g, 2, info.as_ref().map_or("", |i| i.location.as_str()));
            label(g, 3, "Game data");
            let re = ctl(3, 220.0);
            if g.plate_button("reimport", re, "Re-import...", false, false, info.is_some(), 20.0).clicked() {
                action = Some(MenuAction::ReimportGameData);
            }
            rows.note(g, re, 3, "Converts it again; the game restarts afterwards.");
            which_folder(g, Rect::from_min_max(Pos2::new(panel.left(), rows.below(4) + 14.0), panel.max));
        }
        SettingsTab::Mods => {
            if let Some(a) = mods_tab(g, st, b, panel) {
                action = Some(a);
            }
        }
    }
    let back = g.button("back", g.action_left(0.0, 200.0), "Back", false, false, true).clicked();
    let apply = g.button("apply", g.action(0.0, 200.0), "Apply", true, false, true).clicked();
    if apply {
        b.apply_settings(&s);
    }
    st.settings = Some(s);
    if (back || apply) && !st.back() {
        st.reset(Screen::MainMenu);
    }
    action
}

/// Options → Game data: the compact form of the setup screen's "Which
/// folder?" hint (its steps and example folders) in `r`, below a rule, with
/// the setup hint's brass strip down its left edge.
fn which_folder(g: &mut Gfx, r: Rect) {
    g.painter.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0_f32, pal::LINE));
    let r = Rect::from_min_max(r.min + Vec2::new(0.0, 14.0), r.max);
    let inner = Rect::from_min_max(Pos2::new(r.left() + 18.0, r.top()), r.max);
    let mut y = inner.top();
    g.label(Pos2::new(inner.left(), y), "Which folder?", 22.0, pal::BRASS, Align2::LEFT_TOP);
    y += 32.0;
    let num_w = 28.0;
    for (i, s) in setup::STEPS.iter().enumerate() {
        g.label(Pos2::new(inner.left(), y), &format!("{}.", i + 1), 20.0, pal::BRASS, Align2::LEFT_TOP);
        y += g.paragraph(Rect::from_min_max(Pos2::new(inner.left() + num_w, y), Pos2::new(inner.right(), inner.bottom())), s, 20.0, pal::SAND) + 4.0;
    }
    y += 4.0;
    let y = setup::examples(g, inner, y, num_w);
    // The strip runs as far as the hint does.
    let strip = Rect::from_min_max(r.min, Pos2::new(r.left() + 4.0, y.min(r.bottom())));
    g.fill_grad(&[strip.left_top(), strip.right_top(), strip.right_bottom(), strip.left_bottom()], strip.top(), strip.bottom(), steel::shade(pal::BRASS, 0.1), steel::shade(pal::BRASS, -0.3));
}

/// Options → Mods: the installed mods in load order (a checkbox enables
/// one; the later of two enabled mods wins), the selected one's details,
/// its place in the load order, the mods folder. Changes are saved at once
/// and apply when the game restarts.
fn mods_tab(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend, panel: Rect) -> Option<MenuAction> {
    if st.mods.is_none() {
        st.mods = b.mods();
    }
    let mut info = st.mods.take()?;
    let mut changed = false;
    let mut action = None;
    if st.mods_sel.as_ref().is_none_or(|id| !info.mods.iter().any(|m| &m.id == id)) {
        st.mods_sel = info.mods.first().map(|m| m.id.clone());
    }
    let sel = st.mods_sel.as_ref().and_then(|id| info.mods.iter().position(|m| &m.id == id));
    // The list.
    let list = Rect::from_min_max(panel.min, Pos2::new(panel.left() + 760.0, panel.bottom()));
    let inner = g.well(list);
    let name_of = |m: &ModEntry| if m.version.is_empty() { m.name.clone() } else { format!("{}  {}", m.name, m.version) };
    let kind_of = |m: &ModEntry| {
        let kind = if m.gameplay { "Gameplay" } else { "Cosmetic" };
        if m.example {
            format!("{kind} · Example")
        } else {
            kind.to_string()
        }
    };
    let xs = g.columns(inner.width(), 70.0, 0, &[("Mod", info.mods.iter().map(name_of).collect()), ("Author", info.mods.iter().map(|m| m.author.clone()).collect()), ("Kind", info.mods.iter().map(kind_of).chain(["Gameplay".into(), "Cosmetic".into()]).collect())], CELL);
    let body = g.header(inner, &[(16.0, "On"), (xs[0], "Mod"), (xs[1], "Author"), (xs[2], "Kind")]);
    let rh = 44.0;
    let rows: Vec<Vec<(f32, String, Color32)>> = info
        .mods
        .iter()
        .map(|m| {
            let c = if m.enabled { pal::SAND } else { pal::SAND_MUTED };
            vec![(xs[0], name_of(m), c), (xs[1], m.author.clone(), pal::SAND_MUTED), (xs[2], kind_of(m), if m.gameplay { pal::RUST } else { pal::SAND_MUTED })]
        })
        .collect();
    let (clicked, _) = g.table("mods", body, &rows, sel, rh);
    if let Some(i) = clicked {
        st.mods_sel = Some(info.mods[i].id.clone());
    }
    // The checkboxes over the first column (the table draws text only).
    let visible = ((body.height() + 1.0) / rh).floor() as usize;
    let off = g.table_offset("mods");
    let mut toggle = None;
    for (k, m) in info.mods.iter().enumerate().skip(off).take(visible) {
        let r = Rect::from_min_size(Pos2::new(body.left() + 12.0, body.top() + (k - off) as f32 * rh), Vec2::new(36.0, rh));
        let bx = Rect::from_center_size(r.center(), Vec2::splat(24.0));
        let resp = g.interact_button(r, ("modon", &m.id), true);
        g.check_box(bx, m.enabled, resp.hovered());
        if resp.clicked() {
            toggle = Some(k);
        }
    }
    if info.mods.is_empty() {
        g.paragraph(Rect::from_min_size(body.min + Vec2::new(24.0, 24.0), Vec2::new(body.width() - 48.0, 200.0)), &format!("No mods installed. A mod is a folder in {} that holds replacement files laid out like the game data (models/, sound/, missions/, ...).", info.folder), 22.0, pal::SAND_MUTED);
    }
    // The selected mod.
    let side = Rect::from_min_max(Pos2::new(list.right() + 24.0, panel.top()), panel.max);
    let sel = st.mods_sel.as_ref().and_then(|id| info.mods.iter().position(|m| &m.id == id));
    let bw = (side.width() - 12.0) / 2.0;
    // The restart note sits above the mod's buttons (Enable, Move, Open mods folder).
    let note_y = side.bottom() - 3.0 * 50.0 - 2.0 * 12.0 - 62.0;
    let mut y = side.top();
    if let Some(i) = sel {
        let m = info.mods[i].clone();
        g.text_fit(Pos2::new(side.left(), y + 14.0), &m.name, 28.0, pal::SAND, side.width());
        y += 36.0;
        let by = [(!m.version.is_empty()).then(|| format!("version {}", m.version)), (!m.author.is_empty()).then(|| format!("by {}", m.author))].into_iter().flatten().collect::<Vec<_>>().join(", ");
        g.text_fit(Pos2::new(side.left(), y + 10.0), &by, 20.0, pal::SAND_MUTED, side.width());
        y += 28.0;
        let kind = if m.gameplay { "Changes the game: everyone in a network game needs it." } else { "Cosmetic: other players need not have it." };
        y += g.paragraph(Rect::from_min_size(Pos2::new(side.left(), y), Vec2::new(side.width(), 60.0)), kind, 20.0, if m.gameplay { pal::RUST } else { pal::SAND_MUTED }) + 6.0;
        if m.example {
            y += g.paragraph(Rect::from_min_size(Pos2::new(side.left(), y), Vec2::new(side.width(), 60.0)), "Example mod that ships with the game: copy its folder to your mods folder to start your own (its README.md says how).", 20.0, pal::BRASS) + 6.0;
        }
        g.paragraph(Rect::from_min_size(Pos2::new(side.left(), y), Vec2::new(side.width(), (note_y - y - 8.0).max(0.0))), &m.description, 20.0, pal::SAND);
        let enabled_n = info.mods.iter().filter(|e| e.enabled).count();
        let row = side.bottom() - 2.0 * 50.0 - 12.0;
        let label = if m.enabled { "Disable" } else { "Enable" };
        if g.button("modtoggle", Rect::from_min_size(Pos2::new(side.left(), row - 62.0), Vec2::new(side.width(), 50.0)), label, false, false, true).clicked() {
            toggle = Some(i);
        }
        // Up: earlier in the load order (lower priority); down: later (wins).
        if g.button("modup", Rect::from_min_size(Pos2::new(side.left(), row), Vec2::new(bw, 50.0)), "Move up", false, false, m.enabled && i > 0).clicked() {
            info.mods.swap(i, i - 1);
            changed = true;
        }
        if g.button("moddown", Rect::from_min_size(Pos2::new(side.left() + bw + 12.0, row), Vec2::new(bw, 50.0)), "Move down", false, false, m.enabled && i + 1 < enabled_n).clicked() {
            info.mods.swap(i, i + 1);
            changed = true;
        }
    }
    if g.button("modfolder", Rect::from_min_size(Pos2::new(side.left(), side.bottom() - 50.0), Vec2::new(side.width(), 50.0)), "Open mods folder", false, false, true).clicked() {
        b.open_mods_folder();
    }
    if let Some(k) = toggle {
        // Enabling puts a mod last (it wins over the others); the enabled
        // ones stay in front of the disabled ones.
        let mut m = info.mods.remove(k);
        m.enabled = !m.enabled;
        let at = info.mods.iter().filter(|e| e.enabled).count();
        info.mods.insert(at, m);
        changed = true;
    }
    if changed {
        b.set_mods(&info.enabled());
    }
    let restart = info.restart_needed();
    let note = if restart { "Changes apply when the game restarts." } else { "Lower in the list wins when two mods change the same file." };
    g.paragraph(Rect::from_min_size(Pos2::new(side.left(), note_y), Vec2::new(side.width(), 52.0)), note, 20.0, if restart { pal::RUST } else { pal::SAND_MUTED });
    if restart && g.button("modrestart", g.action(220.0, 240.0), "Restart now", false, false, true).clicked() {
        action = Some(MenuAction::Restart);
    }
    st.mods = Some(info);
    action
}

// ---------------------------------------------------------------- help / credits / notices

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

fn help(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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

fn credits(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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

fn quit_confirm(g: &mut Gfx, st: &mut MenuState) -> Option<MenuAction> {
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

const PAUSE_ITEMS: [&str; 5] = ["Return to game", "Options", "Help", "Leave game", "Quit"];

fn pause(g: &mut Gfx, st: &mut MenuState) -> Option<MenuAction> {
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

// ---------------------------------------------------------------- round results

fn team_colour(t: Option<Team>) -> Color32 {
    match t {
        Some(Team::Green) => crate::theme::TEAM_GREEN,
        Some(Team::Blue) => crate::theme::TEAM_BLUE,
        _ => pal::RED,
    }
}

/// The round results over the game: team totals, the players by score with
/// K/D/A, the time to the next map / its loading, and Leave.
fn results(g: &mut Gfx, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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
