//! The optional frame-rate counter: "60 fps · 16.7 ms" averaged over half a
//! second, small, in the UI font with a soft shadow, in the top-right corner
//! the HUD keeps free ([`tt_menu::hud::FREE_CORNER`]), over the game and the
//! menus. Drawn in target pixels, so it stays crisp at any canvas scale; the
//! text buffer and draw list are reused (no allocations of its own per
//! frame).

use std::fmt::Write;
use std::time::Instant;
use tt_gui::uifont::{text_width, TextRun, TextStyle};
use tt_gui::{Color, Rect};

/// Averaging window (ms).
const WINDOW_MS: f64 = 500.0;

pub struct FpsCounter {
    last: Option<Instant>,
    /// Frames and time since the shown values were computed.
    frames: u32,
    acc_ms: f64,
    fps: f64,
    ms: f64,
    run: TextRun,
}

impl Default for FpsCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl FpsCounter {
    pub fn new() -> FpsCounter {
        let run = TextRun {
            origin: [0.0, 0.0],
            text: String::with_capacity(32),
            style: TextStyle { cap: 10.0, bold: true },
            color: Color::rgba(240, 240, 225, 235),
            halo: Color::rgba(0, 0, 0, 150),
            clip: Rect::new(0, 0, 1, 1),
        };
        FpsCounter { last: None, frames: 0, acc_ms: 0.0, fps: 0.0, ms: 0.0, run }
    }

    /// Counts a drawn frame (call once per frame, shown or not, so the first
    /// value after turning it on is already averaged).
    pub fn frame(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last {
            self.acc_ms += (now - last).as_secs_f64() * 1000.0;
            self.frames += 1;
            if self.acc_ms >= WINDOW_MS {
                self.ms = self.acc_ms / self.frames as f64;
                self.fps = 1000.0 / self.ms.max(1e-3);
                self.acc_ms = 0.0;
                self.frames = 0;
            }
        }
        self.last = Some(now);
    }

    /// Paints the counter over `target`.
    /// `hud_scale`: the HUD size option (the counter fills the HUD's corner).
    pub fn draw(&mut self, gui: &mut crate::gui_gpu::GuiRenderer, target: crate::gfx::Target, hud_scale: f32) {
        let crate::gfx::Target { device, queue, view, size } = target;
        let (fps, ms) = (self.fps, self.ms);
        let run = &mut self.run;
        run.text.clear();
        if ms > 0.0 {
            let _ = write!(run.text, "{fps:.0} fps \u{b7} {ms:.1} ms");
        } else {
            run.text.push_str("-- fps");
        }
        // In the HUD's free corner, scaled like the HUD (13 px caps at 1080
        // lines).
        let ppp = tt_menu::hud::scaled_pixels_per_point([size.0 as f32, size.1 as f32], hud_scale);
        let corner = tt_menu::hud::FREE_CORNER * ppp;
        let cap = (ppp * 8.7).max(8.0).round();
        run.style.cap = cap;
        let w = text_width(&run.text, &run.style);
        let margin = (tt_menu::hud::MARGIN * ppp).round();
        let x = (size.0 as f32 - margin - w).max(size.0 as f32 - corner.x);
        run.origin = [x.round(), ((corner.y - cap) * 0.5 + cap).round()];
        run.clip = Rect::new(0, 0, size.0 as i32, size.1 as i32);
        let frame = crate::gui_gpu::GuiFrame { texts: std::slice::from_ref(&self.run), size, pixel_fills: Vec::new() };
        gui.render(device, queue, view, size, &frame);
    }
}
