//! The in-game HUD in the new style: native, resolution-independent text and panels
//! over the 3D view, with only the game's real art kept (radar base and dots, team
//! dots, lives, brain and lag icons).
//!
//! The engine fills a [`HudView`] each frame from the state the ported client
//! scripts keep in their GUI objects (`PlayGui`, `messagesHud`, `PlayerListGui`,
//! `MessageHud`, ...; `crates/tt-engine/src/hud.rs`), so what is shown and when
//! (line wrapping, fades, timeouts, toggles) stays the game's; [`draw`] only decides
//! the look.
//!
//! Layout (points of a 720-point-high screen, [`pixels_per_point`]):
//! top left the player list or the solo score and lives; top centre the
//! clock and the brain counter; bottom left the messages above the chat
//! input; bottom right the radar with the lag icon above it; prints in the
//! middle. The top-right corner ([`FREE_CORNER`]) is left empty for the
//! engine's frame-rate counter.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontId, Galley, Painter, Pos2, Rect, Stroke, TextureHandle, Vec2};

use crate::screens::pal;
use crate::screens::steel::{chamfer, shade};
use crate::theme;

/// Screen margin (points).
pub const MARGIN: f32 = 18.0;
/// The top-right area (width, height in points from the corner) no HUD
/// element uses: room for a small frame-rate counter.
pub const FREE_CORNER: Vec2 = Vec2::new(170.0, 40.0);

/// Points per pixel: the HUD is laid out for a 720-point-high screen (at
/// least 960 points wide), so it scales with the window.
/// [`pixels_per_point`] times the "HUD size" option: everything, margins
/// and the free corner included, scales.
pub fn scaled_pixels_per_point(screen_px: [f32; 2], scale: f32) -> f32 {
    pixels_per_point(screen_px) * scale.clamp(0.5, 1.5)
}

pub fn pixels_per_point(screen_px: [f32; 2]) -> f32 {
    (screen_px[1] / 720.0).min(screen_px[0] / 960.0).max(0.5)
}

/// One message line (`messagesHud`: a `GuiTextCtrl` per wrapped line).
#[derive(Debug, Clone, PartialEq)]
pub struct ChatLine {
    /// The line (colour codes already stripped by `messageScroll`).
    pub text: String,
    /// A chat line (`ttChatHudProfile`) rather than a server message.
    pub chat: bool,
    /// The fade (`animateAlpha`), 0..1.
    pub alpha: f32,
    /// Lines above the bottom of the message area (animated as the lines
    /// slide up; 1 = the newest line's place).
    pub slot: f32,
}

/// The open chat input (`MessageHud`).
#[derive(Debug, Clone, PartialEq)]
pub struct ChatInput {
    /// Team message (`teamMessageHud`) or to all.
    pub team: bool,
    pub text: String,
}

/// Solo lives (`SPLivesGui`): up to four icons, or one with "x N".
#[derive(Debug, Clone, PartialEq)]
pub struct Lives {
    pub icons: u32,
    /// The " X n" label (more than four lives).
    pub count: Option<String>,
}

/// One radar blip (`tt_gui::hud::RadarBlip`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blip {
    pub dx: f32,
    pub dy: f32,
    pub color: Color32,
    pub big: bool,
    /// Shown in this frame (blinking blips are off half the time).
    pub on: bool,
}

/// The radar (`GuiRadarCtrl`): blips around the player.
#[derive(Debug, Clone, PartialEq)]
pub struct Radar {
    pub heading_deg: f32,
    pub params: tt_gui::hud::RadarParams,
    pub blips: Vec<Blip>,
}

/// A name over a tank (`GuiShapeNameHud`).
#[derive(Debug, Clone, PartialEq)]
pub struct NameTag {
    /// Projected label point in screen pixels.
    pub pos: Pos2,
    pub name: String,
    pub color: Color32,
    /// The distance fade, 0..1.
    pub opacity: f32,
    /// Distance from the camera (nearer labels win when they overlap).
    pub dist: f32,
}

/// Team icon of a player list row (`joinGameServerList` icons).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeamDot {
    Red,
    Green,
    Blue,
}

impl TeamDot {
    pub fn color(self) -> Color32 {
        match self {
            TeamDot::Red => pal::RED,
            TeamDot::Green => theme::TEAM_GREEN,
            TeamDot::Blue => theme::TEAM_BLUE,
        }
    }
}

/// A row of the player or team list.
#[derive(Debug, Clone, PartialEq)]
pub struct ListRow {
    pub team: TeamDot,
    pub name: String,
    pub score: String,
    /// K, D, A (the K/D/A columns).
    pub kda: Option<[String; 3]>,
    pub ping: Option<String>,
    /// The local player.
    pub me: bool,
}

/// The player list (`PlayerListGui`, toggled with Tab).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerList {
    pub teams: Vec<ListRow>,
    pub players: Vec<ListRow>,
    pub kda: bool,
    pub ping: bool,
}

/// Everything the HUD shows in a frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HudView {
    /// Real time in ms (caret blink).
    pub time_ms: u64,
    pub chat: Vec<ChatLine>,
    pub chat_input: Option<ChatInput>,
    /// Player names to colour in message lines (team colours).
    pub name_colors: Vec<(String, Color32)>,
    pub center_print: Option<String>,
    pub bottom_print: Option<String>,
    /// `HudClock` text.
    pub clock: Option<String>,
    /// Solo score digits (`SPScoreGui`).
    pub score: Option<String>,
    pub lives: Option<Lives>,
    /// Brain counter text ("03 / 10", `SPBrainCounter`).
    pub brains: Option<String>,
    pub lag: bool,
    pub radar: Option<Radar>,
    pub names: Vec<NameTag>,
    pub player_list: Option<PlayerList>,
    pub vote: Option<VotePrompt>,
}

/// The running map vote: its question and the count line under it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VotePrompt {
    /// "Vote: switch to Medulla Mesa?"
    pub question: String,
    /// "F12 yes / F11 no — yes 2 / no 1 / 6 players — 23 s"
    pub detail: String,
}

/// The HUD's artwork, loaded on first use from the asset directory and
/// resampled to the exact pixel size it is drawn at.
pub struct HudArt {
    root: PathBuf,
    src: HashMap<&'static str, Option<Arc<image::RgbaImage>>>,
    sized: HashMap<(&'static str, u32), Option<TextureHandle>>,
}

pub const RADAR_BASE: &str = "ui/hud/radar_base.png";
pub const LIVES: &str = "ui/icons/lives.png";
pub const BRAIN: &str = "ui/icons/brainicon.png";
pub const LAG: &str = "ui/hud/lagicon.png";

fn dot_art(t: TeamDot) -> &'static str {
    match t {
        TeamDot::Red => "ui/icons/reddot.png",
        TeamDot::Green => "ui/icons/greendot.png",
        TeamDot::Blue => "ui/icons/bluedot.png",
    }
}

impl HudArt {
    pub fn new(root: &Path) -> HudArt {
        HudArt { root: root.to_path_buf(), src: HashMap::new(), sized: HashMap::new() }
    }

    fn source(&mut self, name: &'static str) -> Option<Arc<image::RgbaImage>> {
        let root = &self.root;
        self.src.entry(name).or_insert_with(|| crate::art::load_rgba(root, name).map(Arc::new)).clone()
    }

    /// `name` as a texture `px` pixels high (aspect kept): Lanczos-resampled
    /// when smaller than the image, so it is never minified by the GPU.
    pub fn texture(&mut self, ctx: &egui::Context, name: &'static str, px: u32) -> Option<TextureHandle> {
        let px = px.clamp(1, 4096);
        if let Some(t) = self.sized.get(&(name, px)) {
            return t.clone();
        }
        let tex = self.source(name).map(|img| {
            let (w, h) = (img.width().max(1), img.height().max(1));
            let (img, filter) = if px < h {
                let nw = ((w as f32 * px as f32 / h as f32).round() as u32).max(1);
                (image::imageops::resize(img.as_ref(), nw, px, image::imageops::FilterType::Lanczos3), egui::TextureOptions::LINEAR)
            } else {
                (img.as_ref().clone(), egui::TextureOptions::LINEAR)
            };
            let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
            ctx.load_texture(format!("tt-hud:{name}:{px}"), color, filter)
        });
        // Keep a few sizes per image (window resizes).
        if self.sized.len() > 64 {
            self.sized.clear();
        }
        self.sized.insert((name, px), tex.clone());
        tex
    }
}

/// Drawing helpers on the HUD layer.
struct Hud<'a> {
    ctx: &'a egui::Context,
    p: Painter,
    ppp: f32,
    screen: Rect,
    art: &'a mut HudArt,
}

/// Dark plates behind text for legibility over bright terrain.
const PLATE: Color32 = Color32::from_rgba_premultiplied(19, 21, 18, 165);
const SHADOW: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 190);
/// Corner cut of the HUD plates.
const CHAMFER: f32 = 4.0;

const CHAT: Color32 = Color32::from_rgb(0x6c, 0xe6, 0xee);

fn alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

impl Hud<'_> {
    fn snap(&self, v: f32) -> f32 {
        (v * self.ppp).round() / self.ppp
    }

    fn snap_pos(&self, p: Pos2) -> Pos2 {
        Pos2::new(self.snap(p.x), self.snap(p.y))
    }

    fn galley(&self, text: &str, font: FontId, color: Color32) -> Arc<Galley> {
        self.p.layout_no_wrap(text.to_string(), font, color)
    }

    /// A galley with a 1-pixel drop shadow; `pos` is its top left.
    fn put(&self, pos: Pos2, g: &Arc<Galley>, a: f32) {
        let pos = self.snap_pos(pos);
        let d = 1.0;
        self.p.galley_with_override_text_color(pos + Vec2::splat(d), g.clone(), alpha(SHADOW, a));
        if a >= 0.999 {
            self.p.galley(pos, g.clone(), Color32::WHITE);
        } else {
            // Faded: the galley's own colours scaled.
            let mut job = (*g.job).clone();
            for s in &mut job.sections {
                s.format.color = alpha(s.format.color, a);
            }
            let g2 = self.p.layout_job(job);
            self.p.galley(pos, g2, Color32::WHITE);
        }
    }

    /// Text anchored at `pos`; returns its rectangle.
    fn text(&self, pos: Pos2, text: &str, font: FontId, color: Color32, anchor: Align2) -> Rect {
        let g = self.galley(text, font, color);
        let r = anchor.anchor_size(pos, g.size());
        self.put(r.min, &g, 1.0);
        r
    }

    /// A translucent dark steel plate with chamfered corners.
    fn plate(&self, r: Rect, a: f32) {
        self.p.add(egui::Shape::convex_polygon(chamfer(r, CHAMFER), alpha(PLATE, a), Stroke::NONE));
    }

    /// A steel panel (the menus' look, translucent): chamfered, a steel rim
    /// and an `accent` strip along the top.
    fn panel(&self, r: Rect, accent: Color32) {
        let c = CHAMFER + 2.0;
        self.p.add(egui::Shape::convex_polygon(chamfer(r, c), Color32::from_rgba_unmultiplied(0x23, 0x26, 0x22, 205), Stroke::NONE));
        self.p.add(egui::Shape::closed_line(chamfer(r, c), Stroke::new(1.0_f32, alpha(pal::LINE, 0.8))));
        self.p.rect_filled(Rect::from_min_max(Pos2::new(r.left() + c, r.top()), Pos2::new(r.right() - c, r.top() + 2.0)), 0.0, accent);
    }

    /// A strip of colour along the top or bottom of a [`Hud::plate`], inside its chamfers.
    fn strip(&self, r: Rect, top: bool, color: Color32) {
        let y = if top { r.top() } else { r.bottom() - 2.0 };
        self.p.rect_filled(Rect::from_min_max(Pos2::new(r.left() + CHAMFER, y), Pos2::new(r.right() - CHAMFER, y + 2.0)), 0.0, color);
    }

    /// `name` as an image `h` points high with its left top at `pos`; returns its rect.
    fn image(&mut self, name: &'static str, pos: Pos2, h: f32, tint: Color32) -> Option<Rect> {
        let px = (h * self.ppp).round() as u32;
        let tex = self.art.texture(self.ctx, name, px)?;
        let [w, th] = tex.size();
        let size = Vec2::new(w as f32 * h / th.max(1) as f32, h);
        let r = Rect::from_min_size(self.snap_pos(pos), size);
        self.p.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
        Some(r)
    }
}

fn game(size: f32) -> FontId {
    theme::game_font(size)
}

/// The display face (Russo One): numbers and headings.
fn display(size: f32) -> FontId {
    theme::display_font(size)
}

fn bold(size: f32) -> FontId {
    // `theme::heading` is an em size: match the cap height of `game_font`.
    FontId::new(size * 0.62 / tt_gui::uifont::ui_font().bold.cap_per_em(), theme::heading_family())
}

/// Draws the HUD on the background layer of `ctx` (run inside an egui
/// frame whose `pixels_per_point` is [`pixels_per_point`]).
pub fn draw(ctx: &egui::Context, view: &HudView, art: &mut HudArt) {
    let layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("tt-hud"));
    let screen = ctx.screen_rect();
    let mut h = Hud { ctx, p: ctx.layer_painter(layer), ppp: ctx.pixels_per_point(), screen, art };
    names(&mut h, &view.names);
    radar(&mut h, view);
    top_left(&mut h, view);
    top_centre(&mut h, view);
    prints(&mut h, view);
    chat(&mut h, view);
}

/// Places name tags, nearest first (`rects` in that order, each centred on
/// its tank's label point): a tag that would overlap a nearer one moves up
/// to sit just above it, at most [`MAX_LIFT`] tag heights; one that still
/// overlaps is left out (`None`), like modern games declutter labels.
pub fn place_tags(rects: &[Rect]) -> Vec<Option<Rect>> {
    let mut placed: Vec<Rect> = Vec::new();
    let mut out = Vec::with_capacity(rects.len());
    for &base in rects {
        let mut r = base;
        let mut spot = None;
        for _ in 0..4 {
            match placed.iter().filter(|p| p.expand(1.0).intersects(r)).map(|p| p.top()).reduce(f32::min) {
                None => {
                    spot = Some(r);
                    break;
                }
                Some(top) => r = r.translate(Vec2::new(0.0, top - 2.0 - r.bottom())),
            }
            if base.top() - r.top() > MAX_LIFT * base.height() {
                break;
            }
        }
        if let Some(r) = spot {
            placed.push(r);
        }
        out.push(spot);
    }
    out
}

/// How far (in tag heights) a tag may move up out of the way.
pub const MAX_LIFT: f32 = 2.5;

/// Name tags over the tanks, decluttered by [`place_tags`]; a moved tag
/// keeps a thin leader line to its tank.
fn names(h: &mut Hud, tags: &[NameTag]) {
    let mut order: Vec<&NameTag> = tags.iter().filter(|t| t.opacity > 0.01).collect();
    order.sort_by(|a, b| a.dist.total_cmp(&b.dist));
    let font = game(16.0);
    let pad = Vec2::new(5.0, 2.0);
    let galleys: Vec<Arc<Galley>> = order.iter().map(|t| h.galley(&t.name, font.clone(), t.color)).collect();
    let anchors: Vec<Pos2> = order.iter().map(|t| Pos2::new(t.pos.x / h.ppp, t.pos.y / h.ppp)).collect();
    let rects: Vec<Rect> = galleys.iter().zip(&anchors).map(|(g, a)| Rect::from_center_size(*a, g.size() + pad * 2.0)).collect();
    let spots = place_tags(&rects);
    // Farthest first, so nearer tags are drawn on top.
    for i in (0..order.len()).rev() {
        let Some(r) = spots[i] else { continue };
        if !h.screen.intersects(r) {
            continue;
        }
        let t = order[i];
        let a = t.opacity;
        if rects[i].center().y - r.center().y > 3.0 {
            let from = Pos2::new(r.center().x, r.bottom());
            h.p.line_segment([from, Pos2::new(anchors[i].x, rects[i].center().y)], Stroke::new(1.0_f32, alpha(t.color, 0.55 * a)));
        }
        h.plate(r, 0.85 * a);
        h.strip(r, false, alpha(t.color, 0.9 * a));
        h.put(r.min + pad, &galleys[i], a);
    }
}

fn radar(h: &mut Hud, view: &HudView) {
    let size = 176.0;
    let s = h.screen;
    let rect = Rect::from_min_size(Pos2::new(s.right() - MARGIN - size, s.bottom() - MARGIN - size), Vec2::splat(size));
    if view.lag {
        h.image(LAG, Pos2::new(rect.right() - 40.0, rect.top() - 44.0), 40.0, Color32::WHITE);
    }
    let Some(r) = &view.radar else { return };
    // The control's 128-pixel space scaled to `size`.
    let k = size / 128.0;
    let ext = tt_gui::geom::Point::new(128, 128);
    for b in r.blips.iter().filter(|b| b.on) {
        let (x, y) = tt_gui::hud::radar_point(&r.params, ext, r.heading_deg, b.dx, b.dy);
        // Drawn as vector shapes (crisp at any size): a small filled dot,
        // or a thin ring for the big blips, each with a dark rim.
        let d = r.params.dot_size.x as f32 * k;
        let c = rect.min + Vec2::new(x * k, y * k);
        let rim = Color32::from_black_alpha(150);
        if b.big {
            let (rad, w): (f32, f32) = (d * 0.42, 1.5);
            h.p.circle_stroke(c, rad, Stroke::new(w + 1.5, rim));
            h.p.circle_stroke(c, rad, Stroke::new(w, b.color));
        } else {
            let rad = (d * 0.2).max(1.8);
            h.p.circle_filled(c, rad + 0.8, rim);
            h.p.circle_filled(c, rad, b.color);
        }
    }
    h.image(RADAR_BASE, rect.min, size, Color32::WHITE);
}

fn top_left(h: &mut Hud, view: &HudView) {
    let s = h.screen;
    let mut y = s.top() + MARGIN;
    let x = s.left() + MARGIN;
    if let Some(list) = &view.player_list {
        y = player_list(h, list, Pos2::new(x, y)) + 10.0;
    }
    if let Some(score) = &view.score {
        let label = h.galley("Score", game(12.0), pal::SAND_MUTED);
        let digits = h.galley(score, display(28.0), pal::SAND);
        let w = label.size().x.max(digits.size().x) + 24.0;
        let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 62.0));
        h.panel(r, pal::RUST);
        h.put(Pos2::new(r.left() + 12.0, r.top() + 9.0), &label, 1.0);
        h.put(Pos2::new(r.left() + 12.0, r.top() + 26.0), &digits, 1.0);
        y = r.bottom() + 8.0;
    }
    if let Some(l) = &view.lives {
        let icon = 34.0;
        let n = l.icons.max(1) as f32;
        let count = l.count.as_ref().map(|c| h.galley(c.trim(), display(22.0), pal::SAND));
        let cw = count.as_ref().map_or(0.0, |g| g.size().x + 8.0);
        let w = if l.icons == 0 { 0.0 } else { n * (icon + 4.0) - 4.0 + cw };
        if w > 0.0 {
            let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w + 16.0, icon + 12.0));
            h.plate(r, 1.0);
            let mut ix = r.left() + 8.0;
            for _ in 0..l.icons {
                h.image(LIVES, Pos2::new(ix, r.top() + 6.0), icon, Color32::WHITE);
                ix += icon + 4.0;
            }
            if let Some(g) = count {
                h.put(Pos2::new(ix + 4.0, r.center().y - g.size().y / 2.0), &g, 1.0);
            }
        }
    }
}

/// Draws the player list with its left top at `at`; returns its bottom.
fn player_list(h: &mut Hud, list: &PlayerList, at: Pos2) -> f32 {
    let row_h = 21.0;
    let font = game(15.0);
    let head = display(11.0);
    let name_w = 150.0;
    // Columns after the name: (label, width).
    let mut cols: Vec<(&str, f32)> = vec![("Score", 52.0)];
    if list.kda {
        cols.extend([("K", 30.0), ("D", 30.0), ("A", 30.0)]);
    }
    if list.ping {
        cols.push(("Ping", 46.0));
    }
    let w = 12.0 + 18.0 + name_w + cols.iter().map(|c| c.1).sum::<f32>() + 10.0;
    let teams_h = if list.teams.is_empty() { 0.0 } else { list.teams.len() as f32 * row_h + 8.0 };
    let hgt = 10.0 + 18.0 + teams_h + list.players.len().max(1) as f32 * row_h + 8.0;
    let r = Rect::from_min_size(at, Vec2::new(w, hgt));
    h.panel(r, pal::OLIVE);
    let mut y = r.top() + 10.0;
    // Header.
    let mut cx = r.left() + 12.0 + 18.0;
    h.text(Pos2::new(cx, y + 7.0), "Player", head.clone(), pal::SAND_MUTED, Align2::LEFT_CENTER);
    cx += name_w;
    for (label, cw) in &cols {
        h.text(Pos2::new(cx + cw / 2.0, y + 7.0), label, head.clone(), pal::SAND_MUTED, Align2::CENTER_CENTER);
        cx += cw;
    }
    y += 18.0;
    let row = |h: &mut Hud, y: f32, rw: &ListRow, team_row: bool| {
        let rr = Rect::from_min_size(Pos2::new(r.left() + 2.0, y), Vec2::new(r.width() - 4.0, row_h));
        if rw.me {
            h.p.rect_filled(rr, 0.0, alpha(shade(pal::OLIVE, -0.18), 0.85));
            h.p.rect_filled(Rect::from_min_size(rr.min, Vec2::new(3.0, rr.height())), 0.0, pal::BRASS);
        }
        let cy = rr.center().y;
        let dot = 10.0;
        if h.image(dot_art(rw.team), Pos2::new(r.left() + 12.0, cy - dot / 2.0), dot, Color32::WHITE).is_none() {
            h.p.circle_filled(Pos2::new(r.left() + 17.0, cy), 4.0, rw.team.color());
        }
        let colour = if rw.me { pal::SAND_HI } else if team_row { rw.team.color() } else { pal::SAND };
        let mut name = rw.name.trim().to_string();
        let mut g = h.galley(&name, font.clone(), colour);
        while g.size().x > name_w - 8.0 && name.chars().count() > 1 {
            name.pop();
            g = h.galley(&format!("{}...", name.trim_end()), font.clone(), colour);
        }
        h.put(Pos2::new(r.left() + 30.0, cy - g.size().y / 2.0), &g, 1.0);
        let mut cx = r.left() + 30.0 + name_w;
        let mut vals: Vec<&str> = vec![rw.score.as_str()];
        if list.kda {
            match &rw.kda {
                Some(k) => vals.extend(k.iter().map(String::as_str)),
                None => vals.extend(["", "", ""]),
            }
        }
        if list.ping {
            vals.push(rw.ping.as_deref().unwrap_or(""));
        }
        for ((_, cw), v) in cols.iter().zip(vals) {
            let c = if team_row || rw.me { colour } else { pal::SAND };
            h.text(Pos2::new(cx + cw / 2.0, cy), v, font.clone(), c, Align2::CENTER_CENTER);
            cx += cw;
        }
    };
    for t in &list.teams {
        row(h, y, t, true);
        y += row_h;
    }
    if !list.teams.is_empty() {
        h.p.rect_filled(Rect::from_min_size(Pos2::new(r.left() + 10.0, y + 3.0), Vec2::new(r.width() - 20.0, 1.0)), 0.0, alpha(pal::LINE, 0.8));
        y += 8.0;
    }
    for p in &list.players {
        row(h, y, p, false);
        y += row_h;
    }
    r.bottom()
}

fn top_centre(h: &mut Hud, view: &HudView) {
    let s = h.screen;
    let mut y = s.top() + MARGIN;
    let cx = s.center().x;
    if let Some(clock) = &view.clock {
        let g = h.galley(clock, display(34.0), pal::SAND);
        let r = Rect::from_center_size(Pos2::new(cx, y + g.size().y / 2.0 + 6.0), g.size() + Vec2::new(28.0, 12.0));
        h.panel(r, pal::RUST);
        h.put(r.min + Vec2::new(14.0, 6.0), &g, 1.0);
        y = r.bottom() + 8.0;
    }
    if let Some(b) = &view.brains {
        let icon = 30.0;
        let g = h.galley(b, display(24.0), pal::SAND);
        let w = icon * 43.0 / 33.0 + 8.0 + g.size().x;
        let r = Rect::from_min_size(Pos2::new(cx - (w + 20.0) / 2.0, y), Vec2::new(w + 20.0, icon + 10.0));
        h.plate(r, 1.0);
        let ir = h.image(BRAIN, Pos2::new(r.left() + 10.0, r.top() + 5.0), icon, Color32::WHITE);
        let tx = ir.map_or(r.left() + 10.0 + icon, |ir| ir.right()) + 8.0;
        h.put(Pos2::new(tx, r.center().y - g.size().y / 2.0), &g, 1.0);
        y = r.bottom() + 8.0;
    }
    if let Some(v) = &view.vote {
        let q = h.galley(&v.question, bold(20.0), pal::SAND_HI);
        let d = h.galley(&v.detail, game(15.0), pal::SAND);
        let w = q.size().x.max(d.size().x) + 32.0;
        let r = Rect::from_min_size(Pos2::new(cx - w / 2.0, y), Vec2::new(w, q.size().y + d.size().y + 22.0));
        h.panel(r, pal::BRASS);
        h.put(Pos2::new(cx - q.size().x / 2.0, r.top() + 8.0), &q, 1.0);
        h.put(Pos2::new(cx - d.size().x / 2.0, r.top() + 12.0 + q.size().y), &d, 1.0);
    }
}

pub fn strip_markup(s: &str) -> String {
    crate::strip_tags(s).chars().filter(|c| !c.is_control() || *c == '\n').collect()
}

fn prints(h: &mut Hud, view: &HudView) {
    let s = h.screen;
    let max_w = (s.width() * 0.7).min(820.0);
    let print = |h: &mut Hud, text: &str, y: f32, size: f32| {
        let text = strip_markup(text);
        if text.trim().is_empty() {
            return;
        }
        let mut job = LayoutJob::simple(text.trim().to_string(), bold(size), pal::SAND, max_w);
        job.halign = egui::Align::Center;
        let g = h.p.layout_job(job);
        let r = Rect::from_center_size(Pos2::new(s.center().x, y), g.size() + Vec2::new(36.0, 14.0));
        h.plate(r, 1.0);
        h.strip(r, true, alpha(pal::BRASS, 0.8));
        // A centred galley's rows are laid out around x = 0.
        h.put(Pos2::new(s.center().x, r.top() + 7.0), &g, 1.0);
    };
    if let Some(t) = &view.center_print {
        print(h, t, s.top() + s.height() * 0.36, 26.0);
    }
    if let Some(t) = &view.bottom_print {
        print(h, t, s.top() + s.height() * 0.74, 21.0);
    }
}

/// A message line as a layout job: the names of `name_colors` in their colours.
fn line_job(text: &str, base: Color32, names: &[(String, Color32)], font: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |c| TextFormat { font_id: font.clone(), color: c, ..Default::default() };
    let mut i = 0;
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
    let mut plain = String::new();
    while i < text.len() {
        let hit = names
            .iter()
            .filter(|(n, _)| !n.is_empty() && text[i..].starts_with(n.as_str()) && !word(text[..i].chars().next_back()) && !word(text[i + n.len()..].chars().next()))
            .max_by_key(|(n, _)| n.len());
        match hit {
            Some((n, c)) => {
                if !plain.is_empty() {
                    job.append(&std::mem::take(&mut plain), 0.0, fmt(base));
                }
                job.append(n, 0.0, fmt(*c));
                i += n.len();
            }
            None => {
                let ch = text[i..].chars().next().unwrap();
                plain.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    if !plain.is_empty() {
        job.append(&plain, 0.0, fmt(base));
    }
    job
}

fn chat(h: &mut Hud, view: &HudView) {
    let s = h.screen;
    let x = s.left() + MARGIN;
    let input_h = 30.0;
    let bottom = s.bottom() - MARGIN - input_h - 8.0;
    let lh = 24.0;
    let font = game(17.0);
    // Messages: clipped to their area so lines slide in from its bottom.
    let area = Rect::from_min_max(Pos2::new(x, bottom - lh * 9.0), Pos2::new(x + (s.width() * 0.5).min(620.0), bottom));
    let clip = h.p.clone().with_clip_rect(area);
    let saved = std::mem::replace(&mut h.p, clip);
    for l in &view.chat {
        if l.alpha <= 0.01 {
            continue;
        }
        let base = if l.chat { CHAT } else { pal::SAND };
        let job = line_job(&strip_markup(&l.text), base, &view.name_colors, &font);
        let g = h.p.layout_job(job);
        let top = bottom - l.slot * lh;
        let r = Rect::from_min_size(Pos2::new(x, top), Vec2::new(g.size().x + 16.0, lh - 2.0));
        h.plate(r, 0.85 * l.alpha);
        if l.chat {
            h.p.rect_filled(Rect::from_min_max(r.min + Vec2::new(0.0, CHAMFER), Pos2::new(r.left() + 2.0, r.bottom() - CHAMFER)), 0.0, alpha(CHAT, l.alpha));
        }
        h.put(Pos2::new(r.left() + 8.0, r.center().y - g.size().y / 2.0), &g, l.alpha);
    }
    h.p = saved;
    if let Some(input) = &view.chat_input {
        let r = Rect::from_min_size(Pos2::new(x, s.bottom() - MARGIN - input_h), Vec2::new((s.width() * 0.45).min(520.0), input_h));
        h.p.add(egui::Shape::convex_polygon(chamfer(r, CHAMFER), Color32::from_rgba_unmultiplied(0x17, 0x19, 0x16, 220), Stroke::NONE));
        h.p.add(egui::Shape::closed_line(chamfer(r, CHAMFER), Stroke::new(1.0_f32, pal::LINE)));
        let (label, c) = if input.team { ("Team", theme::TEAM_BLUE) } else { ("All", pal::BRASS) };
        let lg = h.galley(label, bold(13.0), pal::INK);
        let chip = Rect::from_min_size(r.min + Vec2::new(5.0, 5.0), Vec2::new(lg.size().x + 14.0, r.height() - 10.0));
        h.p.rect_filled(chip, 2.0, c);
        h.p.galley(h.snap_pos(chip.center() - lg.size() / 2.0), lg, pal::INK);
        let tx = chip.right() + 8.0;
        let clip = Rect::from_min_max(Pos2::new(tx, r.top()), Pos2::new(r.right() - 6.0, r.bottom()));
        let g = h.galley(&input.text, font.clone(), pal::SAND);
        // Keep the end (the caret) in view.
        let off = (g.size().x - clip.width() + 6.0).max(0.0);
        let p = h.p.clone().with_clip_rect(clip);
        let pos = h.snap_pos(Pos2::new(tx - off, r.center().y - g.size().y / 2.0));
        p.galley(pos, g.clone(), pal::SAND);
        if (view.time_ms / 500).is_multiple_of(2) {
            let cx = h.snap(pos.x + g.size().x + 1.0);
            p.rect_filled(Rect::from_min_size(Pos2::new(cx, r.top() + 7.0), Vec2::new(2.0, r.height() - 14.0)), 0.0, pal::BRASS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_is_stripped() {
        assert_eq!(strip_markup("<just:center>Bait scored a goal!"), "Bait scored a goal!");
        assert_eq!(strip_markup("a <font:Arial:12>b"), "a b");
        assert_eq!(strip_markup("x < y"), "x < y");
    }

    #[test]
    fn names_are_coloured_at_word_boundaries() {
        let names = vec![("Hum".to_string(), Color32::RED), ("Gum".to_string(), Color32::GREEN)];
        let job = line_job("Hum has been eliminated by Gum!", Color32::WHITE, &names, &game(15.0));
        let parts: Vec<(&str, Color32)> = job.sections.iter().map(|s| (&job.text[s.byte_range.clone()], s.format.color)).collect();
        assert_eq!(parts, vec![("Hum", Color32::RED), (" has been eliminated by ", Color32::WHITE), ("Gum", Color32::GREEN), ("!", Color32::WHITE)]);
        // Not inside a word.
        let job = line_job("Humvee", Color32::WHITE, &names, &game(15.0));
        assert_eq!(job.sections.len(), 1);
    }

    #[test]
    fn overlapping_tags_stack_up_or_hide() {
        let tag = |x: f32, y: f32| Rect::from_center_size(Pos2::new(x, y), Vec2::new(60.0, 20.0));
        // Nearest first: the second overlaps the first and moves just above
        // it; the third (same place) above that; a fourth would have to move
        // too far and is left out; a tag elsewhere stays put.
        let spots = place_tags(&[tag(100.0, 100.0), tag(110.0, 104.0), tag(100.0, 100.0), tag(105.0, 98.0), tag(400.0, 100.0)]);
        assert_eq!(spots[0], Some(tag(100.0, 100.0)));
        let s1 = spots[1].unwrap();
        assert_eq!((s1.left(), s1.bottom()), (80.0, 88.0));
        let s2 = spots[2].unwrap();
        assert_eq!(s2.bottom(), s1.top() - 2.0);
        assert_eq!(spots[3], None);
        assert_eq!(spots[4], Some(tag(400.0, 100.0)));
        // No two placed tags overlap.
        let placed: Vec<Rect> = spots.iter().flatten().copied().collect();
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn hud_scale_follows_the_window_height() {
        assert_eq!(pixels_per_point([1280.0, 720.0]), 1.0);
        assert_eq!(pixels_per_point([1920.0, 1080.0]), 1.5);
        assert_eq!(pixels_per_point([3840.0, 2160.0]), 3.0);
        // Narrow windows: at least 960 points wide.
        assert!((pixels_per_point([960.0, 1200.0]) - 1.0).abs() < 1e-6);
    }
}
