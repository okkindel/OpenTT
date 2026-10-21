//! The controls drawn with [`Gfx`]: buttons, selectors, check boxes,
//! segmented choices, sliders, tables and second-level tabs.

use egui::{Align2, Color32, Pos2, Rect, Response, Stroke, Vec2};

use super::steel::{self, Button, Plate};
use super::{pal, Gfx};

/// Text size of table cells ([`Gfx::table`]).
pub(super) const CELL: f32 = 26.0;

impl Gfx<'_> {
    /// A riveted plate button: primary = the brass one (one per screen),
    /// others steel; `focused` lights it like the pointer does.
    pub(super) fn button(&mut self, id: impl std::hash::Hash, rect: Rect, label: &str, primary: bool, focused: bool, enabled: bool) -> Response {
        let size = (rect.height() * 0.5).min(26.0);
        self.plate_button(id, rect, label, Button { primary, focused, enabled }, size)
    }

    /// Small square arrow button (a steel plate with a painted triangle).
    pub(super) fn arrow(&mut self, id: impl std::hash::Hash, rect: Rect, left: bool) -> Response {
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
    pub(super) fn selector(&mut self, id: &str, rect: Rect, value: &str, focused: bool) -> i32 {
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
    pub(super) fn check_box(&self, b: Rect, on: bool, hot: bool) {
        self.inset(b, steel::shade(pal::WELL, -0.25), pal::WELL, if hot { pal::BRASS } else { steel::shade(pal::LINE, 0.1) }, 3.0);
        if on {
            let s = b.shrink(b.width() * 0.22);
            self.fill_grad(&steel::chamfer(s, 2.0), s.top(), s.bottom(), steel::shade(pal::BRASS, 0.15), steel::shade(pal::BRASS, -0.2));
            self.painter.add(egui::Shape::closed_line(steel::chamfer(s, 2.0), Stroke::new(1.0_f32, Color32::from_black_alpha(120))));
        }
    }

    pub(super) fn checkbox(&mut self, id: &str, rect: Rect, value: &mut bool) {
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
    pub(super) fn segmented(&mut self, id: &str, rect: Rect, labels: &[&str], sel: usize) -> Option<usize> {
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
    pub(super) fn tab_inset(&self, r: Rect) {
        self.inset(r, steel::shade(pal::OLIVE, -0.3), steel::shade(pal::OLIVE, -0.05), steel::shade(pal::BRASS, -0.05), 5.0);
    }

    /// A groove with an olive-to-brass fill (progress, sliders), `v` in 0..1.
    pub(super) fn bar(&self, track: Rect, v: f32) {
        self.inset(track, steel::shade(pal::WELL, -0.25), pal::WELL, steel::shade(pal::LINE, 0.1), 2.0);
        let w = (track.width() - 2.0) * v.clamp(0.0, 1.0);
        if w > 0.5 {
            let f = Rect::from_min_size(track.min + Vec2::splat(1.0), Vec2::new(w, track.height() - 2.0));
            self.fill_grad(&[f.left_top(), f.right_top(), f.right_bottom(), f.left_bottom()], f.top(), f.bottom(), steel::shade(pal::BRASS, -0.1), steel::shade(pal::OLIVE, -0.1));
        }
    }

    /// Groove with a brass-and-olive fill and a steel knob.
    pub(super) fn slider(&mut self, id: &str, rect: Rect, value: &mut f32) {
        self.slider_pct(id, rect, value, 0.0, 100.0);
    }

    /// [`Gfx::slider`] whose label shows `lo + value * (hi - lo)` percent.
    pub(super) fn slider_pct(&mut self, id: &str, rect: Rect, value: &mut f32, lo: f32, hi: f32) {
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
    pub(super) fn row_fill(&self, r: Rect, selected: bool, hovered: bool, odd: bool) {
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
    pub(super) fn table(&mut self, id: &str, inner: Rect, rows: &[Vec<(f32, String, Color32)>], sel: Option<usize>, rh: f32) -> (Option<usize>, bool) {
        self.table_sized(id, inner, rows, sel, rh, CELL)
    }

    /// [`Gfx::table`] with text `size`. A cell with a line break shows two
    /// lines: the first in `size`, the second smaller and muted.
    #[allow(clippy::type_complexity)]
    pub(super) fn table_sized(&mut self, id: &str, inner: Rect, rows: &[Vec<(f32, String, Color32)>], sel: Option<usize>, rh: f32, size: f32) -> (Option<usize>, bool) {
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
    pub(super) fn spec_bar(&self, b: Rect, v: f32) {
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

    /// The content panel of a screen with second-level tabs (outer rect):
    /// the tabs stand on its top edge ([`Gfx::subtabs`]).
    pub(super) fn tab_panel_rect(&self) -> Rect {
        self.r(40.0, 146.0, 1200.0, 458.0)
    }

    /// Second-level tabs: folder-tab plates standing on the top edge of
    /// `panel` (the outer rect of the panel below, already drawn). The
    /// inactive ones are darker plates behind the panel's edge; the active
    /// one is raised, made of the panel's own steel with a brass top edge,
    /// and runs into the panel.
    pub(super) fn subtabs(&mut self, id: &str, labels: &[&str], active: usize, panel: Rect) -> Option<usize> {
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
    pub(super) fn columns(&self, inner_w: f32, lead: f32, flex: usize, cols: &[(&str, Vec<String>)], size: f32) -> Vec<f32> {
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

    /// The first row a table (`table_sized` with `id`) shows.
    pub(crate) fn table_offset(&self, id: &str) -> usize {
        self.ctx().data(|d| d.get_temp::<(usize, Option<usize>)>(table_key(id))).map_or(0, |v| v.0)
    }
}

/// The egui memory key of a table's (scroll offset, last selection).
fn table_key(id: &str) -> egui::Id {
    egui::Id::new(("tt-menu-scroll", id))
}
