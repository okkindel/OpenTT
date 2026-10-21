//! The "field garage" look: the menus made of the same material as the
//! OpenTT logo (riveted, olive-tinted steel and brass), drawn entirely by
//! code. Clickable things are chamfered plates with thickness (a light top
//! edge, a dark bottom side) and small rivets; content panels are calmer
//! (chamfered, no rivets); the primary action is the only brass plate.
//!
//! The background steel (page, header band, content panels) is brushed: a
//! tileable texture generated in memory at startup (no texture files) under
//! the gradients. Clickable plates stay smooth.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, Pos2, Rect, Shape, Stroke, TextureId, Vec2};

use super::{pal, Gfx};

// ---------------------------------------------------------------- brushed steel

/// Side of the (tileable) brushed-steel texture, pixels.
const TEX: usize = 512;
/// Mean brightness of the texture; vertex colours are divided by it so a
/// surface keeps its average colour.
const TEX_MEAN: f32 = 0.88;

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// Deterministic noise in [0, 1) for a lattice point.
fn rnd(a: usize, b: usize, seed: u32) -> f32 {
    let h = hash((a as u32).wrapping_mul(0x9e37_79b9) ^ hash((b as u32) ^ seed.wrapping_mul(0x85eb_ca6b)));
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Brushed steel, grey levels: fine horizontal streaks (per-row value noise,
/// smooth along x at several scales, periodic over the tile) and grain.
fn brushed_pixels() -> Vec<f32> {
    let mut v = vec![0.0f32; TEX * TEX];
    let octaves: [(usize, f32); 4] = [(256, 0.45), (64, 0.3), (16, 0.17), (4, 0.08)];
    for y in 0..TEX {
        let row = rnd(y, 7, 99);
        // An occasional brighter or darker scratch.
        let scratch = match rnd(y, 3, 41) {
            s if s > 0.985 => 0.5,
            s if s < 0.012 => -0.5,
            _ => 0.0,
        };
        for x in 0..TEX {
            let mut s = 0.0;
            for (k, (cell, amp)) in octaves.iter().enumerate() {
                let n = TEX / cell;
                let fx = x as f32 / *cell as f32;
                let i0 = fx.floor() as usize;
                let t = fx - i0 as f32;
                let t = t * t * (3.0 - 2.0 * t);
                let a = rnd(y, i0 % n, k as u32 + 1);
                let b = rnd(y, (i0 + 1) % n, k as u32 + 1);
                s += amp * (a + (b - a) * t);
            }
            let grain = rnd(x, y, 17);
            v[y * TEX + x] = 0.5 * s + 0.3 * row + 0.2 * grain + scratch * (0.6 + 0.4 * s);
        }
    }
    // Soften across rows ([1 2 1], wrapping) so the streaks are 2-3 pixels.
    let mut out = vec![0.0f32; TEX * TEX];
    for y in 0..TEX {
        let (up, dn) = ((y + TEX - 1) % TEX, (y + 1) % TEX);
        for x in 0..TEX {
            out[y * TEX + x] = 0.25 * v[up * TEX + x] + 0.5 * v[y * TEX + x] + 0.25 * v[dn * TEX + x];
        }
    }
    // Normalise: mean TEX_MEAN, standard deviation DEPTH.
    let mean = out.iter().sum::<f32>() / out.len() as f32;
    let dev = (out.iter().map(|p| (p - mean) * (p - mean)).sum::<f32>() / out.len() as f32).sqrt().max(1e-4);
    const DEPTH: f32 = 0.055;
    for p in &mut out {
        *p = (TEX_MEAN + (*p - mean) / dev * DEPTH).clamp(0.0, 1.0);
    }
    out
}

/// The brushed-steel texture (made once per context).
fn steel_texture(ctx: &egui::Context) -> TextureId {
    let id = egui::Id::new("tt-steel");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t.id();
    }
    static PIXELS: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let px = PIXELS.get_or_init(brushed_pixels);
    let img = egui::ColorImage::from_gray_iter([TEX, TEX], px.iter().map(|p| (p * 255.0).round() as u8));
    let tex = ctx.load_texture("tt-steel", img, egui::TextureOptions::LINEAR_REPEAT);
    let tid = tex.id();
    ctx.data_mut(|d| d.insert_temp(id, tex));
    tid
}

// ---------------------------------------------------------------- colour helpers

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// `c` lightened (`k` > 0, towards white) or darkened (`k` < 0, towards black).
pub(crate) fn shade(c: Color32, k: f32) -> Color32 {
    if k >= 0.0 {
        mix(c, Color32::WHITE, k)
    } else {
        mix(c, Color32::BLACK, -k)
    }
}

/// A chamfered rectangle (corners cut by `c`), clockwise from the top left.
pub(crate) fn chamfer(r: Rect, c: f32) -> Vec<Pos2> {
    let c = c.min(r.width() / 2.0).min(r.height() / 2.0);
    vec![
        Pos2::new(r.left() + c, r.top()),
        Pos2::new(r.right() - c, r.top()),
        Pos2::new(r.right(), r.top() + c),
        Pos2::new(r.right(), r.bottom() - c),
        Pos2::new(r.right() - c, r.bottom()),
        Pos2::new(r.left() + c, r.bottom()),
        Pos2::new(r.left(), r.bottom() - c),
        Pos2::new(r.left(), r.top() + c),
    ]
}

/// The top edge of a chamfered rectangle (with the upper chamfers).
fn top_edge(r: Rect, c: f32) -> Vec<Pos2> {
    let c = c.min(r.width() / 2.0).min(r.height() / 2.0);
    vec![Pos2::new(r.left(), r.top() + c), Pos2::new(r.left() + c, r.top()), Pos2::new(r.right() - c, r.top()), Pos2::new(r.right(), r.top() + c)]
}

/// The bottom edge of a chamfered rectangle (with the lower chamfers).
fn bottom_edge(r: Rect, c: f32) -> Vec<Pos2> {
    let c = c.min(r.width() / 2.0).min(r.height() / 2.0);
    vec![Pos2::new(r.left(), r.bottom() - c), Pos2::new(r.left() + c, r.bottom()), Pos2::new(r.right() - c, r.bottom()), Pos2::new(r.right(), r.bottom() - c)]
}

/// Colours of a plate.
#[derive(Clone, Copy)]
pub(crate) struct Metal {
    /// Face gradient, top and bottom.
    top: Color32,
    bot: Color32,
    /// The lit top edge.
    hi: Color32,
    /// The side below the face (thickness).
    side: Color32,
    /// Outline against the background.
    line: Color32,
}

/// Kinds of plate.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plate {
    /// Olive-tinted steel (buttons, list entries).
    Steel,
    /// The selected entry: olive drab with a brass edge.
    Olive,
    /// The primary action.
    Brass,
}

/// How a plate button shows: `primary` is the brass one; `focused` (the
/// keyboard focus) lights it like the pointer and edges it in brass.
#[derive(Clone, Copy)]
pub(crate) struct Button {
    pub(crate) primary: bool,
    pub(crate) focused: bool,
    pub(crate) enabled: bool,
}

impl Button {
    /// An enabled steel button.
    pub(crate) const PLAIN: Button = Button { primary: false, focused: false, enabled: true };
    /// The enabled brass button.
    pub(crate) const PRIMARY: Button = Button { primary: true, ..Button::PLAIN };

    /// [`Button::PLAIN`], enabled only when `enabled`.
    pub(crate) fn enabled(enabled: bool) -> Button {
        Button { enabled, ..Button::PLAIN }
    }
}

impl Plate {
    fn metal(self, hot: bool) -> Metal {
        let lift = if hot { 0.07 } else { 0.0 };
        match self {
            Plate::Steel => Metal {
                top: shade(shade(pal::PLATE, 0.08), lift),
                bot: shade(shade(pal::PLATE, -0.12), lift),
                hi: shade(pal::PLATE, 0.28),
                side: shade(pal::PLATE, -0.55),
                line: Color32::from_rgb(0x10, 0x12, 0x10),
            },
            Plate::Olive => Metal {
                top: shade(shade(pal::OLIVE, -0.03), lift),
                bot: shade(shade(pal::OLIVE, -0.18), lift),
                hi: shade(pal::BRASS, 0.1),
                side: shade(pal::OLIVE, -0.6),
                line: shade(pal::BRASS, -0.15),
            },
            Plate::Brass => Metal {
                top: shade(shade(pal::BRASS, 0.12), lift * 0.8),
                bot: shade(shade(pal::BRASS, -0.16), lift * 0.8),
                hi: shade(pal::BRASS, 0.62),
                side: shade(pal::BRASS, -0.6),
                line: Color32::from_rgb(0x1a, 0x17, 0x08),
            },
        }
    }
}

// ---------------------------------------------------------------- drawing

impl Gfx<'_> {
    /// A vertex of a steel surface: brushed-texture coordinates in screen
    /// pixels (one texel per pixel at any scale); plain if not `textured`
    /// (shadows, overlays).
    fn steel_vertex(&self, pos: Pos2, color: Color32, textured: bool) -> Vertex {
        if textured {
            let k = self.ppp() / TEX as f32;
            let g = |c: u8| ((c as f32 / TEX_MEAN).round()).min(255.0) as u8;
            Vertex { pos, uv: Pos2::new(pos.x * k, pos.y * k), color: Color32::from_rgba_premultiplied(g(color.r()), g(color.g()), g(color.b()), color.a()) }
        } else {
            Vertex { pos, uv: WHITE_UV, color }
        }
    }

    fn steel_mesh(&self, textured: bool) -> Mesh {
        if textured {
            Mesh::with_texture(steel_texture(&self.ctx()))
        } else {
            Mesh::default()
        }
    }

    /// Fill the convex polygon `pts` with a smooth vertical gradient (`top`
    /// at `y0`, `bot` at `y1`): plates, buttons, wells.
    pub(crate) fn fill_grad(&self, pts: &[Pos2], y0: f32, y1: f32, top: Color32, bot: Color32) {
        self.fill_steel(pts, y0, y1, top, bot, false);
    }

    /// [`Gfx::fill_grad`], brushed (`textured`) for the background surfaces
    /// (header band, content panels); clickable plates stay smooth.
    pub(crate) fn fill_steel(&self, pts: &[Pos2], y0: f32, y1: f32, top: Color32, bot: Color32, textured: bool) {
        let mut m = self.steel_mesh(textured);
        let at = |y: f32| mix(top, bot, ((y - y0) / (y1 - y0).max(1e-3)).clamp(0.0, 1.0));
        let c = pts.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / pts.len() as f32;
        let c = c.to_pos2();
        m.vertices.push(self.steel_vertex(c, at(c.y), textured));
        for p in pts {
            m.vertices.push(self.steel_vertex(*p, at(p.y), textured));
        }
        let n = pts.len() as u32;
        for i in 0..n {
            m.add_triangle(0, 1 + i, 1 + (i + 1) % n);
        }
        self.painter.add(Shape::mesh(m));
    }

    /// A grid mesh over `r` coloured by `f(u, v)` (u, v in 0..1), steel-textured if `textured`.
    fn grid(&self, r: Rect, nx: usize, ny: usize, textured: bool, f: impl Fn(f32, f32) -> Color32) {
        let mut m = self.steel_mesh(textured);
        for j in 0..=ny {
            for i in 0..=nx {
                let (u, v) = (i as f32 / nx as f32, j as f32 / ny as f32);
                let p = Pos2::new(r.left() + u * r.width(), r.top() + v * r.height());
                m.vertices.push(self.steel_vertex(p, f(u, v), textured));
            }
        }
        let w = nx as u32 + 1;
        for j in 0..ny as u32 {
            for i in 0..nx as u32 {
                let a = j * w + i;
                m.add_triangle(a, a + 1, a + w);
                m.add_triangle(a + 1, a + w + 1, a + w);
            }
        }
        self.painter.add(Shape::mesh(m));
    }

    /// The background: gunmetal with a soft light from above.
    pub(crate) fn backdrop(&self, r: Rect) {
        self.grid(r, 1, 8, true, |_, v| shade(pal::GUNMETAL, 0.05 - 0.12 * v));
    }

    /// A map picture behind the content, blurred (overlapping offset draws)
    /// and darkened into the gunmetal.
    pub(crate) fn backdrop_picture(&self, r: Rect, tex: TextureId, aspect: f32) {
        let ra = r.width() / r.height();
        let uv = if ra > aspect {
            let h = aspect / ra;
            Rect::from_min_max(Pos2::new(0.0, 0.5 - h / 2.0), Pos2::new(1.0, 0.5 + h / 2.0))
        } else {
            let w = ra / aspect;
            Rect::from_min_max(Pos2::new(0.5 - w / 2.0, 0.0), Pos2::new(0.5 + w / 2.0, 1.0))
        };
        // Taps on two rings; tap k at alpha 1/(k+1) averages them all.
        let mut taps = vec![Vec2::ZERO];
        for (rad, n) in [(14.0f32, 6), (28.0, 10)] {
            for i in 0..n {
                let a = i as f32 / n as f32 * std::f32::consts::TAU + rad;
                taps.push(Vec2::angled(a) * rad);
            }
        }
        let p = self.painter.with_clip_rect(r.intersect(self.painter.clip_rect()));
        // Dim: the picture shows at about a quarter of its brightness.
        let dim = 0.26;
        for (k, d) in taps.iter().enumerate() {
            let a = 1.0 / (k as f32 + 1.0);
            let tint = Color32::from_rgba_unmultiplied((255.0 * dim) as u8, (255.0 * dim * 0.97) as u8, (255.0 * dim * 0.88) as u8, (a * 255.0).round() as u8);
            p.image(tex, r.expand(36.0).translate(*d), uv, tint);
        }
        // Blend back towards the gunmetal at the top and bottom.
        self.grid(r, 1, 8, false, |_, v| {
            let e = ((v - 0.5).abs() * 2.0).powf(2.2);
            Color32::from_rgba_unmultiplied(pal::GUNMETAL.r(), pal::GUNMETAL.g(), pal::GUNMETAL.b(), (115.0 + 140.0 * e) as u8)
        });
    }

    /// Darkened edges and corners of `r`.
    pub(crate) fn vignette(&self, r: Rect) {
        let aspect = r.width() / r.height();
        self.grid(r, 24, 14, false, |u, v| {
            let dx = (u - 0.5) * 2.0 * aspect.sqrt();
            let dy = (v - 0.45) * 2.0;
            let d = ((dx * dx + dy * dy) / (aspect + 1.0)).sqrt();
            let t = ((d - 0.45) / 0.65).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            Color32::from_black_alpha((t * 150.0) as u8)
        });
    }

    /// The header band (raised steel) with its shadow on the content below.
    pub(crate) fn header_band(&self, r: Rect) {
        let base = shade(pal::GUNMETAL, 0.06);
        self.fill_steel(&[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], r.top(), r.bottom(), shade(base, 0.03), shade(base, -0.08), true);
        let y = r.bottom();
        self.grid(Rect::from_min_max(Pos2::new(r.left(), y), Pos2::new(r.right(), y + 14.0)), 1, 4, false, |_, v| Color32::from_black_alpha(((1.0 - v).powi(2) * 120.0) as u8));
        self.painter.line_segment([Pos2::new(r.left(), y - 1.5), Pos2::new(r.right(), y - 1.5)], Stroke::new(1.0_f32, shade(base, 0.16)));
        self.painter.line_segment([Pos2::new(r.left(), y), Pos2::new(r.right(), y)], Stroke::new(1.5_f32, Color32::from_rgb(0x0c, 0x0d, 0x0c)));
    }

    /// A raised plate: chamfered face with a light top edge, a dark
    /// side below it (thicker when not pressed) and, if `rivets`, a rivet
    /// in each corner. Returns the face rectangle.
    pub(crate) fn plate(&self, r: Rect, kind: Plate, hot: bool, down: bool, rivets: bool, c: f32) -> Rect {
        let m = kind.metal(hot);
        let thick = if down { 1.0 } else { (r.height() * 0.05).clamp(2.0, 4.0) };
        let face = Rect::from_min_max(Pos2::new(r.left(), r.top() + if down { thick + 1.0 } else { 0.0 }), Pos2::new(r.right(), r.bottom() - thick));
        let face = if down { Rect::from_min_max(face.min, Pos2::new(face.right(), r.bottom() - 1.0)) } else { face };
        // Drop shadow, side, face.
        self.painter.add(Shape::convex_polygon(chamfer(r.translate(Vec2::new(0.0, 2.0)), c), Color32::from_black_alpha(70), Stroke::NONE));
        self.painter.add(Shape::convex_polygon(chamfer(r, c), m.side, Stroke::NONE));
        self.fill_grad(&chamfer(face, c), face.top(), face.bottom(), m.top, m.bot);
        self.painter.add(Shape::line(top_edge(face.shrink2(Vec2::new(0.5, 0.5)), c - 0.2), Stroke::new(1.0_f32, m.hi)));
        self.painter.add(Shape::closed_line(chamfer(r, c), Stroke::new(1.0_f32, m.line)));
        if rivets {
            let rad = (face.height() * 0.036).clamp(2.2, 3.4);
            let ins = c * 0.6 + rad + 4.0;
            for p in [
                Pos2::new(face.left() + ins, face.top() + ins),
                Pos2::new(face.right() - ins, face.top() + ins),
                Pos2::new(face.left() + ins, face.bottom() - ins),
                Pos2::new(face.right() - ins, face.bottom() - ins),
            ] {
                self.rivet(p, rad, mix(m.top, m.bot, (p.y - face.top()) / face.height()));
            }
        }
        face
    }

    /// A rivet head on a surface of colour `on`: shadow, head, highlight.
    pub(crate) fn rivet(&self, p: Pos2, r: f32, on: Color32) {
        self.painter.circle_filled(p + Vec2::new(0.4, 0.8), r + 0.7, Color32::from_black_alpha(170));
        self.painter.circle_filled(p, r, shade(on, -0.1));
        self.painter.circle_filled(p + Vec2::new(-r * 0.25, -r * 0.3), r * 0.62, Color32::from_white_alpha(60));
        self.painter.circle_filled(p + Vec2::new(-r * 0.35, -r * 0.4), r * 0.28, Color32::from_white_alpha(130));
    }

    /// A content panel: a chamfered steel plate without rivets, a little
    /// darker and calmer than the buttons. Returns the padded inside.
    pub(crate) fn steel_panel(&self, r: Rect, pad: f32) -> Rect {
        let c = 12.0;
        let base = pal::PANEL;
        self.painter.add(Shape::convex_polygon(chamfer(r.translate(Vec2::new(0.0, 3.0)), c), Color32::from_black_alpha(80), Stroke::NONE));
        self.fill_steel(&chamfer(r, c), r.top(), r.bottom(), shade(base, 0.05), shade(base, -0.1), true);
        self.painter.add(Shape::line(top_edge(r.shrink(1.0), c - 0.4), Stroke::new(1.0_f32, shade(base, 0.16))));
        self.painter.add(Shape::line(bottom_edge(r.shrink(1.0), c - 0.4), Stroke::new(1.0_f32, shade(base, -0.4))));
        self.painter.add(Shape::closed_line(chamfer(r, c), Stroke::new(1.0_f32, Color32::from_rgb(0x10, 0x12, 0x10))));
        r.shrink(pad)
    }

    /// A small square remove button: a steel plate with a painted cross
    /// (centred geometry, not a glyph).
    pub(crate) fn remove_button(&self, r: Rect, hot: bool, down: bool) {
        let face = self.plate(r, Plate::Steel, hot, down, false, 4.0);
        let c = face.center();
        let s = face.height().min(face.width()) * 0.2;
        let stroke = Stroke::new(2.0_f32, if hot { pal::SAND_HI } else { pal::SAND_MUTED });
        self.painter.line_segment([c + Vec2::new(-s, -s), c + Vec2::new(s, s)], stroke);
        self.painter.line_segment([c + Vec2::new(-s, s), c + Vec2::new(s, -s)], stroke);
    }

    /// A folder tab standing on a [`Gfx::steel_panel`] whose top edge is at
    /// `r.bottom()`. `active`: the panel's own steel (same gradient where
    /// they meet) with a brass top edge, its face running over the panel's
    /// edge so the two read as one piece. Otherwise a darker plate behind
    /// the panel, outlined, a little lighter when `hot`.
    pub(crate) fn folder_tab(&self, r: Rect, active: bool, hot: bool) {
        let c = 7.0;
        let outline = |r: Rect| vec![r.left_bottom(), Pos2::new(r.left(), r.top() + c), Pos2::new(r.left() + c, r.top()), Pos2::new(r.right() - c, r.top()), Pos2::new(r.right(), r.top() + c), r.right_bottom()];
        let base = pal::PANEL;
        let line = Color32::from_rgb(0x10, 0x12, 0x10);
        if active {
            // A soft shadow on the steel around it, then the face, which
            // ends in the panel's top colour (`steel_panel`) below its edge.
            self.painter.add(Shape::convex_polygon(outline(r.expand2(Vec2::new(2.0, 0.0)).translate(Vec2::new(0.0, -1.0))), Color32::from_black_alpha(60), Stroke::NONE));
            let face = Rect::from_min_max(r.min, Pos2::new(r.right(), r.bottom() + 2.5));
            self.fill_steel(&outline(face), r.top(), r.bottom(), shade(base, 0.12), shade(base, 0.05), true);
            self.painter.add(Shape::line(outline(r), Stroke::new(1.0_f32, line)));
            self.painter.add(Shape::line(top_edge(r.shrink2(Vec2::new(1.5, 1.5)), c - 0.6), Stroke::new(2.0_f32, pal::BRASS)));
            // The lit inner sides, down into the panel.
            let s = Stroke::new(1.0_f32, shade(base, 0.2));
            self.painter.line_segment([Pos2::new(r.left() + 1.0, r.top() + c + 1.0), Pos2::new(r.left() + 1.0, r.bottom() + 1.0)], s);
        } else {
            let k = if hot { -0.06 } else { -0.2 };
            self.fill_steel(&outline(r), r.top(), r.bottom(), shade(base, k + 0.02), shade(base, k - 0.16), true);
            self.painter.add(Shape::line(top_edge(r.shrink2(Vec2::new(1.0, 1.0)), c - 0.4), Stroke::new(1.0_f32, shade(base, if hot { 0.2 } else { 0.06 }))));
            self.painter.add(Shape::line(outline(r), Stroke::new(1.0_f32, line)));
            // Its foot goes behind the panel: a shadow line along the edge.
            self.painter.line_segment([Pos2::new(r.left(), r.bottom() - 1.0), Pos2::new(r.right(), r.bottom() - 1.0)], Stroke::new(2.0_f32, Color32::from_black_alpha(90)));
        }
    }

    /// An inset (pressed-in) plate: dark upper inner edge, light lower edge.
    pub(crate) fn inset(&self, r: Rect, top: Color32, bot: Color32, edge: Color32, c: f32) {
        // The rim (just outside), the face, then the inner shadow along the
        // top and the lit lip along the bottom, both inside the face.
        self.painter.add(Shape::closed_line(chamfer(r.expand(0.5), c + 0.2), Stroke::new(1.0_f32, Color32::from_black_alpha(130))));
        self.fill_grad(&chamfer(r, c), r.top(), r.bottom(), top, bot);
        let k = (c * 0.6).min(r.width() / 2.0);
        let y0 = r.top() + 1.0;
        self.painter.line_segment([Pos2::new(r.left() + k, y0), Pos2::new(r.right() - k, y0)], Stroke::new(2.0_f32, Color32::from_black_alpha(110)));
        let y1 = r.bottom() - 0.5;
        self.painter.line_segment([Pos2::new(r.left() + c, y1), Pos2::new(r.right() - c, y1)], Stroke::new(1.0_f32, edge));
    }

    /// Width of the steel bezel around pictures.
    pub(crate) const BEZEL: f32 = 7.0;

    /// A steel bezel around `pic` (the picture goes inside it, drawn after);
    /// returns the outer rectangle.
    pub(crate) fn bezel(&self, pic: Rect) -> Rect {
        let outer = pic.expand(Self::BEZEL);
        let c = 7.0;
        let base = pal::PLATE;
        self.painter.add(Shape::convex_polygon(chamfer(outer.translate(Vec2::new(0.0, 3.0)), c), Color32::from_black_alpha(90), Stroke::NONE));
        self.fill_grad(&chamfer(outer, c), outer.top(), outer.bottom(), shade(base, 0.14), shade(base, -0.22));
        self.painter.add(Shape::line(top_edge(outer.shrink(0.5), c - 0.2), Stroke::new(1.0_f32, shade(base, 0.35))));
        self.painter.add(Shape::closed_line(chamfer(outer, c), Stroke::new(1.0_f32, Color32::from_rgb(0x0e, 0x10, 0x0e))));
        self.painter.rect_filled(pic, 0.0, pal::WELL);
        outer
    }

    /// The lip where a picture sits in its bezel (drawn over the picture).
    pub(crate) fn bezel_lip(&self, pic: Rect) {
        let lip = pic.expand(1.0);
        let base = pal::PLATE;
        self.painter.line_segment([lip.left_top(), lip.right_top()], Stroke::new(2.0_f32, Color32::from_black_alpha(170)));
        self.painter.line_segment([lip.left_top(), lip.left_bottom()], Stroke::new(1.0_f32, Color32::from_black_alpha(130)));
        self.painter.line_segment([lip.left_bottom(), lip.right_bottom()], Stroke::new(1.0_f32, shade(base, 0.3)));
        self.painter.line_segment([lip.right_top(), lip.right_bottom()], Stroke::new(1.0_f32, shade(base, 0.12)));
    }

    /// A picture framed by a steel bezel; `min`/`max` bound the frame, the
    /// picture is shown at its native pixel size or smaller. Returns the outer rect.
    pub(crate) fn bezel_picture(&self, tex: &egui::TextureHandle, min: Pos2, max: Vec2) -> Rect {
        let b = Self::BEZEL;
        let ppp = self.ppp();
        let px = tex.size_vec2();
        let room = max - Vec2::splat(2.0 * b);
        let k = (room.x / (px.x / ppp)).min(room.y / (px.y / ppp)).min(1.0);
        let size = px / ppp * k;
        let snap = |v: f32| (v * ppp).round() / ppp;
        let pic = Rect::from_min_size(Pos2::new(snap(min.x + b), snap(min.y + b)), Vec2::new(snap(size.x), snap(size.y)));
        let outer = self.bezel(pic);
        self.painter.image(tex.id(), pic, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        self.bezel_lip(pic);
        outer
    }

    /// Text in the display face (Russo One); like [`Gfx::text`].
    pub(crate) fn label(&self, pos: Pos2, text: &str, size: f32, color: Color32, anchor: Align2) -> Rect {
        self.text_in(pos, text, crate::theme::display_font(size), size, color, anchor)
    }

    pub(crate) fn label_width(&self, text: &str, size: f32) -> f32 {
        self.painter.layout_no_wrap(text.into(), crate::theme::display_font(size), Color32::WHITE).size().x
    }

    /// A plate button ([`Button`]) with a display-face label `size` high;
    /// rivets when it is big enough to carry them.
    pub(crate) fn plate_button(&mut self, id: impl std::hash::Hash, rect: Rect, label: &str, state: Button, size: f32) -> egui::Response {
        let Button { primary, focused, enabled } = state;
        let resp = self.interact_button(rect, id, enabled);
        let hot = enabled && (resp.hovered() || focused);
        let down = enabled && resp.is_pointer_button_down_on();
        let kind = if primary && enabled { Plate::Brass } else { Plate::Steel };
        let rivets = rect.height() >= 40.0 && rect.width() >= 90.0;
        let face = self.plate(rect, kind, hot, down, rivets, 7.0);
        if focused && enabled && !primary {
            self.painter.add(Shape::closed_line(chamfer(rect, 7.0), Stroke::new(1.5_f32, pal::BRASS)));
        }
        let color = if !enabled {
            pal::DISABLED
        } else if primary {
            pal::INK
        } else if hot {
            pal::SAND_HI
        } else {
            pal::SAND
        };
        self.label(face.center(), label, size, color, Align2::CENTER_CENTER);
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brushed_texture_is_deterministic_and_in_range() {
        let a = brushed_pixels();
        assert_eq!(a, brushed_pixels());
        let mean = a.iter().sum::<f32>() / a.len() as f32;
        assert!((mean - TEX_MEAN).abs() < 0.01, "mean {mean}");
        assert!(a.iter().all(|p| (0.0..=1.0).contains(p)));
    }
}
