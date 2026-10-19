//! The window's video mode as the game sets it (`createCanvas`,
//! `setScreenMode`, `toggleFullScreen`): [`crate::host::Video`] and the
//! prefs that remember it.

use crate::host::EngineHost;
use tt_gameplay::client::value::Val;

/// `createCanvas(title)`: the window from the prefs, the canvas, then the
/// video card profile (`setVideo`).
pub fn create_canvas(h: &mut EngineHost, title: &str) {
    h.video.title = title.to_string();
    let res: Vec<i32> = h.shell.prefs.get("video.resolution").split_whitespace().map(tt_game::fmt::atoi).collect();
    if let [w, hh, rest @ ..] = res.as_slice() {
        if *w > 0 && *hh > 0 {
            h.video.width = *w as u32;
            h.video.height = *hh as u32;
            h.video.bpp = rest.first().copied().unwrap_or(32).max(16) as u32;
        }
    }
    h.video.fullscreen = !h.video.force_windowed && tt_game::fmt::atoi(&h.shell.prefs.get("video.full_screen")) != 0;
    h.video.changed = true;
    h.canvas.exists = true;
    let v = &h.video;
    let msg = format!("Video mode {}x{}x{} {} (OpenGL)", v.width, v.height, v.bpp, if v.fullscreen { "fullscreen" } else { "windowed" });
    h.log.info(&msg);
    crate::gameplay::client::game::set_video_profile(h);
}

/// `setScreenMode(w, h, bpp, fullScreen)` (`fullscreen` None: unchanged).
pub fn set_mode(h: &mut EngineHost, w: i32, hh: i32, bpp: i32, fullscreen: Option<bool>) {
    if w > 0 && hh > 0 {
        h.video.width = w as u32;
        h.video.height = hh as u32;
    }
    if bpp >= 16 {
        h.video.bpp = bpp as u32;
    }
    if let Some(f) = fullscreen {
        h.video.fullscreen = f && !h.video.force_windowed;
    }
    h.video.changed = true;
    let v = &h.video;
    let res = format!("{} {} {}", v.width, v.height, v.bpp);
    let full = if v.fullscreen { "1" } else { "0" };
    crate::shell::set_pref_val(&mut h.shell.prefs, "video.resolution", Val::s(res));
    crate::shell::set_pref_val(&mut h.shell.prefs, "video.full_screen", Val::s(full));
}

/// `toggleFullScreen()`.
pub fn toggle_fullscreen(h: &mut EngineHost) {
    let fs = !h.video.fullscreen;
    let (w, hh, bpp) = (h.video.width as i32, h.video.height as i32, h.video.bpp as i32);
    set_mode(h, w, hh, bpp, Some(fs));
}
