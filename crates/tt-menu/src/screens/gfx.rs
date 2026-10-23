//! The drawing context of a frame ([`Gfx`]): stage layout, text, basic
//! pieces (panels, wells, dialogs, the action row) and pictures. The steel
//! materials are in [`super::steel`], the controls in [`super::widgets`].

use egui::{Align2, Color32, Painter, Pos2, Rect, Response, Sense, Stroke, Vec2};

use super::{pal, steel, LOGO_H, LOGO_Y, PLATE_H, SHIFT, TOP_H};
use crate::backend::UiSound;

/// Drawing/interaction context for one frame.
pub(crate) struct Gfx<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) painter: Painter,
    pub(super) stage: Rect,
    pub(super) screen: Rect,
    /// Interaction disabled (a modal dialog is open on top).
    pub(super) blocked: bool,
    /// Sounds requested while drawing (button presses, the tally).
    pub(super) sounds: Vec<UiSound>,
    /// The enabled button under the pointer.
    pub(super) hover: Option<egui::Id>,
}

impl<'a> Gfx<'a> {
    pub(super) fn ctx(&self) -> egui::Context {
        self.ui.ctx().clone()
    }

    /// Stage-relative rectangle.
    pub(super) fn r(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.stage.min + Vec2::new(x, y + SHIFT), Vec2::new(w, h))
    }

    pub(super) fn ppp(&self) -> f32 {
        self.ui.ctx().pixels_per_point()
    }

    /// The logo ([`crate::logo`]) of the top bar with its top left corner at
    /// `pos`, pre-scaled to its exact pixel size and pixel-aligned. Returns
    /// its rectangle.
    pub(super) fn logo(&mut self, pos: Pos2) -> Rect {
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
    pub(super) fn top_logo(&mut self, name: &str, extra: Option<&str>) -> Rect {
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
    pub(super) fn name_plate(&self, pos: Pos2, max_w: f32, name: &str, extra: Option<&str>) -> Rect {
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
    pub(super) fn text_width(&self, text: &str, size: f32) -> f32 {
        self.painter.layout_no_wrap(text.into(), crate::theme::game_font(size), Color32::WHITE).size().x
    }

    pub(super) fn line_height(&self, size: f32) -> f32 {
        size
    }

    /// Text in the game lettering (the UI font, cap height 0.62 of `size`,
    /// baseline at 0.8 of the line box); `anchor` positions its box at `pos`.
    pub(super) fn text(&self, pos: Pos2, text: &str, size: f32, color: Color32, anchor: Align2) -> Rect {
        self.text_in(pos, text, crate::theme::game_font(size), size, color, anchor)
    }

    /// [`Gfx::text`] in `font`.
    pub(super) fn text_in(&self, pos: Pos2, text: &str, font: egui::FontId, size: f32, color: Color32, anchor: Align2) -> Rect {
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
    pub(super) fn text_fit(&self, pos: Pos2, text: &str, size: f32, color: Color32, max_w: f32) {
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
    pub(super) fn paragraph(&self, rect: Rect, text: &str, size: f32, color: Color32) -> f32 {
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

    pub(super) fn interact(&mut self, rect: Rect, id: impl std::hash::Hash) -> Response {
        let sense = if self.blocked { Sense::hover() } else { Sense::click_and_drag() };
        self.ui.interact(rect, egui::Id::new(("tt-menu", id)), sense)
    }

    pub(super) fn interact_button(&mut self, rect: Rect, id: impl std::hash::Hash, enabled: bool) -> Response {
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
    pub(super) fn caption(&self, pos: Pos2, text: &str) {
        self.label(pos, text, 22.0, pal::SAND_MUTED, Align2::LEFT_TOP);
    }

    /// Content panel (calm steel, no rivets); returns its padded inside.
    pub(super) fn panel(&self, r: Rect) -> Rect {
        self.steel_panel(r, 16.0)
    }

    /// Dark well pressed into the steel (lists, pictures, previews); returns the inside.
    pub(super) fn well(&self, r: Rect) -> Rect {
        self.inset(r, steel::shade(pal::WELL, -0.25), pal::WELL, steel::shade(pal::LINE, 0.1), 4.0);
        r.shrink(2.0)
    }

    /// Column header row of a table; returns the area below it.
    pub(super) fn header(&self, inner: Rect, cols: &[(f32, &str)]) -> Rect {
        let h = 40.0;
        let r = Rect::from_min_size(inner.min, Vec2::new(inner.width(), h));
        self.fill_grad(&[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], r.top(), r.bottom(), steel::shade(pal::PANEL_HI, 0.04), steel::shade(pal::PANEL_HI, -0.12));
        self.painter.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, Color32::from_black_alpha(140)));
        for (x, t) in cols {
            self.label(Pos2::new(r.left() + x, r.center().y), t, 20.0, pal::SAND_MUTED, Align2::LEFT_CENTER);
        }
        Rect::from_min_max(Pos2::new(inner.left(), r.bottom()), inner.max)
    }

    /// Centred dialog: a steel panel over the dimmed screen, the title in
    /// the display face; returns (content rect, button row centre y).
    pub(super) fn dialog(&self, size: Vec2, title: &str) -> (Rect, f32) {
        self.painter.rect_filled(self.screen, 0.0, Color32::from_black_alpha(160));
        let r = Rect::from_center_size(self.stage.center(), size);
        self.steel_panel(r, 0.0);
        self.label(r.left_top() + Vec2::new(28.0, 24.0), title, 30.0, pal::SAND, Align2::LEFT_TOP);
        (Rect::from_min_max(r.min + Vec2::new(28.0, 76.0), r.max - Vec2::new(28.0, 80.0)), r.bottom() - 46.0)
    }

    pub(super) fn text_field(&mut self, rect: Rect, text: &str) {
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
    pub(super) fn action(&self, right_off: f32, w: f32) -> Rect {
        Rect::from_min_size(Pos2::new(self.stage.right() - 40.0 - right_off - w, self.stage.bottom() - 76.0), Vec2::new(w, 54.0))
    }

    pub(super) fn action_left(&self, left_off: f32, w: f32) -> Rect {
        Rect::from_min_size(Pos2::new(self.stage.left() + 40.0 + left_off, self.stage.bottom() - 76.0), Vec2::new(w, 54.0))
    }

    /// Run `f` with drawing clipped to `clip`.
    /// (Clicks outside `clip` miss the controls drawn in it.)
    pub(super) fn clipped<R>(&mut self, clip: Rect, f: impl FnOnce(&mut Self) -> R) -> R {
        let old = self.painter.clone();
        let old_ui = self.ui.clip_rect();
        self.painter = old.with_clip_rect(clip.intersect(old.clip_rect()));
        self.ui.set_clip_rect(clip.intersect(old_ui));
        let out = f(self);
        self.ui.set_clip_rect(old_ui);
        self.painter = old;
        out
    }

    /// A vertical scroll region `view` showing content `content_h` high:
    /// the mouse wheel over it scrolls; a thin bar on the right shows where.
    /// Returns the offset (points) to draw the content at.
    pub(super) fn scroll_offset(&mut self, id: &str, view: Rect, content_h: f32) -> f32 {
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

/// Returns its rect.
pub(super) fn native_picture(g: &Gfx, tex: &egui::TextureHandle, min: Pos2, max: Vec2) -> Rect {
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
pub(super) fn picture(g: &Gfx, tex: Option<egui::TextureId>, r: Rect, aspect: f32) {
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
