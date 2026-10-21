//! The UI typeface: one TrueType font (two weights) bundled in
//! `assets/fonts`, used for all GUI, HUD and menu text, rasterised at the
//! exact pixel size it is shown at (see [`GlyphAtlas`]).
//!
//! To swap the font everywhere, change [`REGULAR_TTF`] / [`BOLD_TTF`].

use std::collections::HashMap;
use std::sync::OnceLock;

use ab_glyph::{point, Font as _, FontArc, GlyphId, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

use crate::geom::{Color, Rect};

/// Chakra Petch (SIL OFL 1.1, `crates/tt-gui/assets/fonts/chakra-petch/OFL.txt`).
pub const REGULAR_TTF: &[u8] = include_bytes!("../assets/fonts/chakra-petch/ChakraPetch-Medium.ttf");
pub const BOLD_TTF: &[u8] = include_bytes!("../assets/fonts/chakra-petch/ChakraPetch-Bold.ttf");
/// Russo One (SIL OFL 1.1, `crates/tt-gui/assets/fonts/russo-one/OFL.txt`):
/// the display face of the menus (headings, tab and button labels).
pub const HEADING_TTF: &[u8] = include_bytes!("../assets/fonts/russo-one/RussoOne-Regular.ttf");

/// One weight of the UI font.
pub struct Face {
    pub font: FontArc,
    /// The TTF data (for hosts that load it themselves, e.g. egui).
    pub data: &'static [u8],
    /// Cap height ("H") in font units.
    cap_units: f32,
}

impl Face {
    fn load(data: &'static [u8]) -> Face {
        let font = FontArc::try_from_slice(data).expect("bundled UI font");
        let h = font.glyph_id('H');
        let cap_units = font.outline(h).map(|o| o.bounds.max.y.abs().max(o.bounds.min.y.abs())).filter(|v| *v > 0.0).unwrap_or(font.ascent_unscaled() * 0.7);
        Face { font, data, cap_units }
    }

    /// Cap height as a fraction of the em.
    pub fn cap_per_em(&self) -> f32 {
        self.cap_units / self.font.units_per_em().unwrap_or(1000.0)
    }

    /// ab_glyph scale giving cap height `cap` pixels.
    fn scale_for_cap(&self, cap: f32) -> PxScale {
        PxScale::from(cap / self.cap_units * self.font.height_unscaled())
    }
}

pub struct UiFont {
    pub regular: Face,
    pub bold: Face,
}

impl UiFont {
    pub fn face(&self, bold: bool) -> &Face {
        if bold {
            &self.bold
        } else {
            &self.regular
        }
    }
}

/// The UI font (loaded once).
pub fn ui_font() -> &'static UiFont {
    static FONT: OnceLock<UiFont> = OnceLock::new();
    FONT.get_or_init(|| UiFont { regular: Face::load(REGULAR_TTF), bold: Face::load(BOLD_TTF) })
}

/// The display face ([`HEADING_TTF`], loaded once).
pub fn heading_face() -> &'static Face {
    static FACE: OnceLock<Face> = OnceLock::new();
    FACE.get_or_init(|| Face::load(HEADING_TTF))
}

/// Size and weight of a text run, in canvas pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Cap height.
    pub cap: f32,
    pub bold: bool,
}

impl TextStyle {
    fn face(&self) -> &'static Face {
        ui_font().face(self.bold)
    }
}

fn glyph_id(face: &Face, c: char) -> Option<GlyphId> {
    if (c as u32) < 0x20 {
        return None;
    }
    let id = face.font.glyph_id(c);
    if id.0 != 0 {
        return Some(id);
    }
    // Not in the font: a close ASCII stand-in, else "?".
    let alt = match c {
        '\u{a0}' => ' ',
        '‘' | '’' => '\'',
        '“' | '”' => '"',
        '–' | '—' => '-',
        _ => '?',
    };
    Some(face.font.glyph_id(alt))
}

/// Glyphs of `text`: (char, glyph, kerning with the previous glyph, advance), in pixels.
fn advances<'a>(text: &'a str, style: &TextStyle) -> impl Iterator<Item = (char, Option<GlyphId>, f32, f32)> + 'a {
    let face = style.face();
    let sf = face.font.as_scaled(face.scale_for_cap(style.cap));
    let mut prev: Option<GlyphId> = None;
    text.chars().map(move |c| match glyph_id(face, c) {
        Some(id) => {
            let kern = prev.map(|p| sf.kern(p, id)).unwrap_or(0.0);
            prev = Some(id);
            (c, Some(id), kern, sf.h_advance(id))
        }
        None => (c, None, 0.0, 0.0),
    })
}

/// Width of `text` in `style` (pixels).
pub fn text_width(text: &str, style: &TextStyle) -> f32 {
    advances(text, style).map(|a| a.2 + a.3).sum()
}

/// One run of text as a draw command: `origin` is the pen at the baseline
/// (canvas pixels, may be fractional), drawn in `color` over an optional
/// `halo` (a soft dark outline; alpha 0 = none).
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub origin: [f32; 2],
    pub text: String,
    pub style: TextStyle,
    pub color: Color,
    pub halo: Color,
    pub clip: Rect,
}

/// A textured quad of the [`GlyphAtlas`] in target pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphQuad {
    pub dst: Rect,
    pub src: Rect,
    pub color: Color,
}

const ATLAS: u32 = 1024;
/// Horizontal sub-pixel positions per pixel.
const SUBPIXEL: f32 = 4.0;

/// (bold, glyph, cap height * 4, halo radius * 8, sub-pixel x)
type Key = (bool, u16, u16, u8, u8);

/// Glyph coverage, `left`/`top` relative to the pen's pixel at the baseline.
struct Mask {
    left: i32,
    top: i32,
    w: i32,
    h: i32,
    cov: Vec<u8>,
}

fn rasterize(face: &Face, id: GlyphId, cap_px: f32, sub_x: f32) -> Option<Mask> {
    let g = id.with_scale_and_position(face.scale_for_cap(cap_px), point(sub_x, 0.0));
    let og = face.font.outline_glyph(g)?;
    let b = og.px_bounds();
    let (w, h) = (b.width() as i32, b.height() as i32);
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut cov = vec![0u8; (w * h) as usize];
    og.draw(|x, y, c| {
        if (x as i32) < w && (y as i32) < h {
            cov[(y as i32 * w + x as i32) as usize] = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    });
    Some(Mask { left: b.min.x as i32, top: b.min.y as i32, w, h, cov })
}

/// The coverage grown by `r` pixels (a soft-edged disc dilation), for halos.
fn dilate(m: &Mask, r: f32) -> Mask {
    let ri = r.ceil() as i32 + 1;
    let (w, h) = (m.w + 2 * ri, m.h + 2 * ri);
    let mut cov = vec![0u8; (w * h) as usize];
    let taps: Vec<(i32, i32, f32)> =
        (-ri..=ri).flat_map(|dy| (-ri..=ri).map(move |dx| (dx, dy))).filter_map(|(dx, dy)| {
            let k = (r + 0.5 - ((dx * dx + dy * dy) as f32).sqrt()).clamp(0.0, 1.0);
            (k > 0.0).then_some((dx, dy, k))
        }).collect();
    for y in 0..h {
        for x in 0..w {
            let mut v = 0.0f32;
            for &(dx, dy, k) in &taps {
                let (sx, sy) = (x - ri + dx, y - ri + dy);
                if sx >= 0 && sy >= 0 && sx < m.w && sy < m.h {
                    v = v.max(m.cov[(sy * m.w + sx) as usize] as f32 * k);
                }
            }
            cov[(y * w + x) as usize] = v.round() as u8;
        }
    }
    Mask { left: m.left - ri, top: m.top - ri, w, h, cov }
}

/// Glyphs rasterised at target resolution, packed into one RGBA texture
/// (white, coverage in alpha) that a backend uploads when
/// [`GlyphAtlas::version`] changes.
#[derive(Debug, Clone)]
pub struct GlyphAtlas {
    image: RgbaImage,
    slots: HashMap<Key, Option<(Rect, i32, i32)>>,
    shelf: (i32, i32, i32),
    version: u64,
}

impl Default for GlyphAtlas {
    fn default() -> Self {
        GlyphAtlas::new()
    }
}

impl GlyphAtlas {
    pub fn new() -> GlyphAtlas {
        GlyphAtlas { image: RgbaImage::from_pixel(ATLAS, ATLAS, Rgba([255, 255, 255, 0])), slots: HashMap::new(), shelf: (1, 1, 0), version: 1 }
    }

    pub fn image(&self) -> &RgbaImage {
        &self.image
    }

    /// Bumped whenever the image changes.
    pub fn version(&self) -> u64 {
        self.version
    }

    fn clear(&mut self) {
        self.image.pixels_mut().for_each(|p| p.0[3] = 0);
        self.slots.clear();
        self.shelf = (1, 1, 0);
        self.version += 1;
    }

    fn slot(&mut self, key: Key) -> Option<(Rect, i32, i32)> {
        if let Some(s) = self.slots.get(&key) {
            return *s;
        }
        let (bold, id, cap4, halo8, sub) = key;
        let face = ui_font().face(bold);
        let mask = rasterize(face, GlyphId(id), cap4 as f32 / 4.0, sub as f32 / SUBPIXEL).map(|m| if halo8 > 0 { dilate(&m, halo8 as f32 / 8.0) } else { m });
        let slot = match mask {
            Some(m) if m.w + 2 < ATLAS as i32 && m.h + 2 < ATLAS as i32 => {
                let (mut x, mut y, mut sh) = self.shelf;
                if x + m.w + 1 > ATLAS as i32 {
                    (x, y, sh) = (1, y + sh + 1, 0);
                }
                if y + m.h + 1 > ATLAS as i32 {
                    self.clear();
                    (x, y, sh) = self.shelf;
                }
                for j in 0..m.h {
                    for i in 0..m.w {
                        self.image.get_pixel_mut((x + i) as u32, (y + j) as u32).0[3] = m.cov[(j * m.w + i) as usize];
                    }
                }
                self.shelf = (x + m.w + 1, y, sh.max(m.h));
                self.version += 1;
                Some((Rect::new(x, y, m.w, m.h), m.left, m.top))
            }
            _ => None,
        };
        self.slots.insert(key, slot);
        slot
    }

    /// Quads of `run` on a target where canvas pixel `p` lands at
    /// `off + p * scale`: the halo first, then the glyphs. Glyphs keep the
    /// canvas layout (scaled); they are rasterised at the target size with
    /// the baseline and the cap height on whole pixels.
    pub fn layout(&mut self, run: &TextRun, scale: f32, off: (f32, f32), out: &mut Vec<GlyphQuad>) {
        let st = &run.style;
        let cap_px = (st.cap * scale).round().max(3.0);
        let cap4 = (cap_px * 4.0).round().min(u16::MAX as f32) as u16;
        let base_y = (off.1 + run.origin[1] * scale).round() as i32;
        let mut pens: Vec<(u16, i32, u8)> = Vec::new();
        let mut pen = 0.0f32;
        for (c, id, kern, adv) in advances(&run.text, st) {
            pen += kern;
            if let Some(id) = id.filter(|_| !c.is_whitespace()) {
                let x = off.0 + (run.origin[0] + pen) * scale;
                let mut ix = x.floor();
                let mut sub = ((x - ix) * SUBPIXEL).round();
                if sub >= SUBPIXEL {
                    ix += 1.0;
                    sub = 0.0;
                }
                pens.push((id.0, ix as i32, sub as u8));
            }
            pen += adv;
        }
        let emit = |atlas: &mut GlyphAtlas, halo8: u8, color: Color, out: &mut Vec<GlyphQuad>| {
            for &(id, ix, sub) in &pens {
                if let Some((src, left, top)) = atlas.slot((st.bold, id, cap4, halo8, sub)) {
                    out.push(GlyphQuad { dst: Rect::new(ix + left, base_y + top, src.w, src.h), src, color });
                }
            }
        };
        if run.halo.0[3] > 0 {
            let r = (cap_px * 0.12).clamp(1.0, 3.0);
            emit(self, (r * 8.0).round() as u8, run.halo, out);
        }
        if run.color.0[3] > 0 {
            emit(self, 0, run.color, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_font_loads() {
        let f = ui_font();
        for face in [&f.regular, &f.bold] {
            let c = face.cap_per_em();
            assert!((0.5..0.85).contains(&c), "cap/em {c}");
            for ch in "AZaz09!?.,:;()[]-+/'\"%&#@äöüéß".chars() {
                assert_ne!(face.font.glyph_id(ch).0, 0, "{ch}");
            }
        }
    }

    #[test]
    fn metrics() {
        let st = TextStyle { cap: 10.6, bold: false };
        assert_eq!(text_width("", &st), 0.0);
        let w = text_width("Hello World 0123", &st);
        // The game's Isocpeur 20 is 103 px wide for this string.
        assert!((80.0..140.0).contains(&w), "{w}");
        // Colour codes take no room.
        assert_eq!(text_width("\x01ab\x02", &st), text_width("ab", &st));
    }

    #[test]
    fn cap_height_matches() {
        // An "H" at cap 10 px: ink rows -10..-1 above the baseline, with crisp edges.
        let f = ui_font();
        let m = rasterize(&f.regular, f.regular.font.glyph_id('H'), 10.0, 0.0).unwrap();
        assert_eq!(m.top, -10, "top");
        assert_eq!(m.top + m.h, 0, "bottom");
        let row = |y: i32| (0..m.w).map(|i| m.cov[((y - m.top) * m.w + i) as usize] as u32).max().unwrap();
        assert!(row(-10) > 200 && row(-1) > 200);
    }

    #[test]
    fn atlas_layout_places_glyphs() {
        let mut a = GlyphAtlas::new();
        let run = TextRun {
            origin: [10.0, 20.0],
            text: "Hi there".into(),
            style: TextStyle { cap: 10.6, bold: false },
            color: Color::WHITE,
            halo: Color::rgba(0, 0, 0, 100),
            clip: Rect::new(0, 0, 100, 100),
        };
        let mut out = Vec::new();
        a.layout(&run, 2.0, (0.0, 0.0), &mut out);
        // 7 glyphs, each with a halo first.
        assert_eq!(out.len(), 14);
        assert!(out[..7].iter().all(|q| q.color.0[3] == 100));
        let v = a.version();
        out.clear();
        a.layout(&run, 2.0, (0.0, 0.0), &mut out);
        assert_eq!(a.version(), v, "cached");
        // "H" starts near x = 20 and its top is 21 px above the baseline at y = 40.
        let g = out[7];
        assert!((19..=23).contains(&g.dst.x), "{:?}", g.dst);
        assert_eq!(g.dst.y, 40 - 21, "{:?}", g.dst);
    }
}
