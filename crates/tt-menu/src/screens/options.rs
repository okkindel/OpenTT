//! The Options tab: display, graphics, sound, controls, game data and mods.

use egui::{Align2, Color32, Key, Pos2, Rect, Stroke, Vec2};

use super::steel::{self, Button};
use super::widgets::CELL;
use super::{pal, pressed, setup, Gfx};
use crate::backend::*;
use crate::{sorted_modes, MenuAction, MenuState, Screen, SettingsTab};

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

pub(super) fn settings(g: &mut Gfx, st: &mut MenuState, b: &mut dyn MenuBackend) -> Option<MenuAction> {
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
            if g.plate_button("reimport", re, "Re-import...", Button::enabled(info.is_some()), 20.0).clicked() {
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
