//! The screens, in the "field garage" look of [`steel`]: riveted steel
//! plates, brass and olive drab like the OpenTT logo, Russo One for headings
//! and labels, the UI font (`tt_gui::uifont`) for body text. Layout
//! happens on a 1280x720 stage centred on the screen (scaled by
//! `pixels_per_point`).

use egui::{Align2, Key, Pos2, Rect, Vec2};

use crate::backend::*;
use crate::{strip_markup, MenuAction, MenuState, Screen};

mod custom;
mod garage;
mod gfx;
mod help;
mod host;
mod in_game;
mod online;
mod options;
mod play;
pub(crate) mod players;
mod selectors;
pub(crate) mod setup;
mod solo;
pub(crate) mod steel;
mod tutorial;
mod widgets;

use gfx::Gfx;
use steel::Button;
pub(crate) use solo::TallyAnim;
pub(crate) use tutorial::Slides;

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

/// The keys a text field acts on, pressed this frame.
#[derive(Default)]
struct Typed {
    enter: bool,
    escape: bool,
    tab: bool,
}

/// Typing into `text` this frame: printable characters (up to `max` of
/// them) and Backspace; returns the Enter, Escape and Tab presses.
fn edit_text(ctx: &egui::Context, text: &mut String, max: usize) -> Typed {
    let mut k = Typed::default();
    ctx.input(|i| {
        for e in &i.events {
            match e {
                egui::Event::Text(t) => {
                    for c in t.chars().filter(|c| !c.is_control()) {
                        if text.chars().count() < max {
                            text.push(c);
                        }
                    }
                }
                egui::Event::Key { key: Key::Backspace, pressed: true, .. } => {
                    text.pop();
                }
                egui::Event::Key { key: Key::Enter, pressed: true, .. } => k.enter = true,
                egui::Event::Key { key: Key::Escape, pressed: true, .. } => k.escape = true,
                egui::Event::Key { key: Key::Tab, pressed: true, .. } => k.tab = true,
                _ => {}
            }
        }
    });
    k
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
        Screen::Loading | Screen::QuitConfirm | Screen::Pause | Screen::Results | Screen::VoteMap => return None,
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
    if g.plate_button("top-quit", q, "Quit", Button::PLAIN, 24.0).clicked() {
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
    let modal = message.is_some() || st.new_player.is_some() || st.confirm_delete.is_some() || st.password.is_some() || st.masters.is_some() || screen == Screen::QuitConfirm;
    let shown = if screen == Screen::QuitConfirm { st.stack.iter().rev().nth(1).copied().unwrap_or(Screen::MainMenu) } else { screen };
    // The Play tab's mission picture, blurred behind the content.
    let backdrop = if shown == Screen::MainMenu && st.focus == 0 {
        let mission = b.solo_status().mission;
        b.art(ctx, &Art::Mission(mission))
    } else {
        None
    };
    let action = frame(ctx, st, egui::Order::Background, modal, |g, st| {
        if !matches!(screen, Screen::Pause | Screen::Results | Screen::VoteMap) {
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
        return frame(ctx, st, egui::Order::Foreground, false, play::quit_confirm);
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
        return players::new_player_dialog(ctx, st, b);
    }
    if st.confirm_delete.is_some() {
        players::delete_dialog(ctx, st, b);
        return None;
    }
    if st.password.is_some() {
        return online::password_dialog(ctx, st);
    }
    if st.masters.is_some() {
        online::masters_dialog(ctx, st, b);
        return None;
    }
    if std::mem::take(&mut st.esc_eaten) {
        return action;
    }
    if action.is_none() && st.screen() == screen && pressed(ctx, Key::Escape) {
        match screen {
            Screen::Pause | Screen::VoteMap => return Some(MenuAction::Resume),
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
        Screen::PlayerSelect => players::player_select(g, st, b),
        Screen::TutorialPrompt | Screen::Tutorial if !b.tutorial_available() => {
            // No tutorial art in the game data: no tutorial.
            st.reset(Screen::MainMenu);
            None
        }
        Screen::TutorialPrompt => players::tutorial_prompt(g, st),
        Screen::Tutorial => tutorial::tutorial(g, st, b),
        Screen::MainMenu => play::main_menu(g, st, b),
        Screen::Garage => garage::garage(g, st, b),
        Screen::Solo => solo::solo(g, st, b),
        Screen::HighScores => solo::high_scores(g, st, b),
        Screen::Loading => play::loading(g, b),
        Screen::CustomGame => custom::custom_game(g, st, b),
        Screen::Online => online::online(g, st, b),
        Screen::Settings => options::settings(g, st, b),
        Screen::Help => help::help(g, st, b),
        Screen::Credits => help::credits(g, st, b),
        Screen::QuitConfirm => None,
        Screen::Pause => in_game::pause(g, st, b),
        Screen::Results => in_game::results(g, b),
        Screen::VoteMap => in_game::vote_map(g, st, b),
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
