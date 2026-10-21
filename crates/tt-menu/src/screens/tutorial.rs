//! The tutorial: a slide show, then target practice.

use egui::{Align2, Color32, Key, Pos2, Rect, Vec2};

use super::{pal, pressed, Gfx};
use crate::backend::*;
use crate::{MenuAction, MenuState, Screen};

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

pub(super) fn tutorial(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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
