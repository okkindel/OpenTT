//! The first-run screen ("Locate your ThinkTanks installation"), its
//! import progress and the import's report (what was imported and what
//! was skipped, with the log folder a click away), also used by Settings →
//! Game data → Re-import. It runs
//! before the game data exists, so it draws with the menu's palette, fonts
//! and logo only (all compiled in).
//! The engine owns the work (folder check, folder dialog, the import
//! thread) and feeds this screen through [`SetupState`]; the screen
//! returns what the player asked for as a [`SetupAction`].

use super::{frame, pal, pressed, Gfx, TOP_H};
use crate::MenuState;
use egui::{Align2, Key, Pos2, Rect, Vec2};

/// Why the screen is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupMode {
    /// No game data: the game cannot start without it.
    FirstRun,
    /// Settings → Game data → Re-import.
    Reimport,
}

/// Where a running import is (the engine's copy of the importer's progress).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SetupProgress {
    /// What the import is doing ("Converting textures").
    pub phase: String,
    pub done: usize,
    pub total: usize,
    /// The file being made.
    pub file: String,
}

/// What the player asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum SetupAction {
    /// Open the system's folder dialog.
    Browse,
    /// Check this folder and import it.
    Import(String),
    /// Stop the running import.
    Cancel,
    /// Leave the screen (re-import only: back to the game).
    Back,
    /// Quit the program (first run).
    Quit,
    /// Open the folder of the import log in the file manager (the report).
    OpenLogFolder,
    /// Leave the report: into the game after an import, back to the
    /// folder choice after a failure.
    Continue,
}

/// What the last import (or folder check) found, shown until the player
/// continues ([`SetupAction::Continue`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SetupReport {
    /// The import succeeded (Continue starts the game).
    pub ok: bool,
    /// One paragraph: the counts, or what failed.
    pub summary: String,
    /// The details, a line each (skipped and missing files, warnings).
    pub lines: Vec<String>,
    /// The full report's file (`logs/import.log`), empty when none was written.
    pub log: String,
}

/// The screen's state; the engine sets the fields, [`SetupState::ui`] draws them.
pub struct SetupState {
    pub mode: SetupMode,
    /// The folder field (typed, pasted or set from the folder dialog).
    pub path: String,
    /// The last problem (bad folder, failed import), shown in red.
    pub error: Option<String>,
    /// A note shown in place of the error (e.g. the last import was cancelled).
    pub note: Option<String>,
    /// The system's folder dialog can be used (else the path is typed).
    pub can_browse: bool,
    /// The folder dialog is open.
    pub browsing: bool,
    /// Where the imported files go (shown to the player).
    pub target: String,
    /// The running import, if any.
    pub progress: Option<SetupProgress>,
    /// Cancel was asked for; the import stops after its current file.
    pub cancelling: bool,
    /// The report of the last import, shown in place of the folder choice.
    pub report: Option<SetupReport>,
    /// The menu state the drawing helpers keep their bits in.
    menu: MenuState,
}

impl SetupState {
    pub fn new(mode: SetupMode) -> SetupState {
        SetupState { mode, path: String::new(), error: None, note: None, can_browse: true, browsing: false, target: String::new(), progress: None, cancelling: false, report: None, menu: MenuState::new() }
    }

    /// Draws the screen and handles its input.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<SetupAction> {
        if crate::theme::ensure(ctx) {
            return None;
        }
        let mut menu = std::mem::take(&mut self.menu);
        let action = if self.progress.is_some() {
            importing(ctx, self, &mut menu)
        } else if self.report.is_some() {
            report(ctx, self, &mut menu)
        } else {
            locate(ctx, self, &mut menu)
        };
        menu.sounds.clear();
        self.menu = menu;
        action
    }
}

/// The background and the top bar with the logo.
fn chrome(g: &mut Gfx, sub: &str) {
    let s = g.screen;
    g.backdrop(s);
    g.vignette(s);
    let top = Rect::from_min_max(s.min, Pos2::new(s.right(), g.stage.top() + TOP_H));
    g.header_band(top);
    g.top_logo(sub, None);
}

/// The path in the field: the end of it when it is too long.
fn field_text(g: &Gfx, path: &str, w: f32) -> String {
    if g.text_width(path, 30.0) <= w {
        return path.to_string();
    }
    let chars: Vec<char> = path.chars().collect();
    let mut start = 0;
    while start < chars.len() && g.text_width(&format!("...{}", chars[start..].iter().collect::<String>()), 30.0) > w {
        start += 1;
    }
    format!("...{}", chars[start..].iter().collect::<String>())
}

/// Which copy of ThinkTanks to choose, in three steps (the setup screen
/// and Options → Game data).
pub(super) const STEPS: [&str; 3] = [
    "You need ThinkTanks for Windows, installed.",
    "On Linux or macOS, install it with Wine (wine Setup.exe, or whatever the installer is called).",
    "Choose the folder that contains the \"game\" folder, not the installer.",
];
/// Example folders of an installed copy.
pub(super) const EXAMPLES: [(&str, &str); 2] = [("Windows", "C:\\Program Files (x86)\\BraveTree\\ThinkTanks"), ("Wine", "~/.wine/drive_c/Program Files (x86)/BraveTree/ThinkTanks")];

/// The hint panel of the first-run screen: which copy of ThinkTanks to
/// choose, in short numbered steps, with example folders. A calm steel
/// panel with a brass strip down its left edge.
fn what_to_choose(g: &mut Gfx, r: Rect) {
    let inner = g.steel_panel(r, 18.0);
    let strip = Rect::from_min_max(Pos2::new(r.left() + 6.0, r.top() + 14.0), Pos2::new(r.left() + 10.0, r.bottom() - 14.0));
    g.fill_grad(&[strip.left_top(), strip.right_top(), strip.right_bottom(), strip.left_bottom()], strip.top(), strip.bottom(), super::steel::shade(pal::BRASS, 0.1), super::steel::shade(pal::BRASS, -0.3));
    let inner = Rect::from_min_max(Pos2::new(inner.left() + 6.0, inner.top()), inner.max);
    let mut y = inner.top();
    g.label(Pos2::new(inner.left(), y), "Which folder?", 24.0, pal::BRASS, Align2::LEFT_TOP);
    y += 40.0;
    let num_w = 30.0;
    for (i, s) in STEPS.iter().enumerate() {
        g.label(Pos2::new(inner.left(), y), &format!("{}.", i + 1), 21.0, pal::BRASS, Align2::LEFT_TOP);
        y += g.paragraph(Rect::from_min_max(Pos2::new(inner.left() + num_w, y), Pos2::new(inner.right(), inner.bottom())), s, 21.0, pal::SAND) + 8.0;
    }
    y += 4.0;
    examples(g, inner, y, num_w);
}

/// The example folders ([`EXAMPLES`]) from `y` in `inner`, indented by
/// `indent`: the system, then the path beside it, or below it when it
/// does not fit there. Returns the `y` below them.
pub(super) fn examples(g: &mut Gfx, inner: Rect, mut y: f32, indent: f32) -> f32 {
    let label_w = 86.0;
    for (os, path) in EXAMPLES {
        if y + 22.0 > inner.bottom() {
            break;
        }
        g.text(Pos2::new(inner.left() + indent, y), os, 18.0, pal::SAND_MUTED, Align2::LEFT_TOP);
        let beside = inner.width() - indent - label_w;
        if g.text_width(path, 18.0) > beside && y + 40.0 <= inner.bottom() {
            y += 19.0;
            g.text_fit(Pos2::new(inner.left() + indent, y + 9.0), path, 18.0, pal::SAND_MUTED, inner.width() - indent);
        } else {
            g.text_fit(Pos2::new(inner.left() + indent + label_w, y + 9.0), path, 18.0, pal::SAND_MUTED, beside);
        }
        y += 24.0;
    }
    y
}

fn locate(ctx: &egui::Context, st: &mut SetupState, menu: &mut MenuState) -> Option<SetupAction> {
    let mut action = None;
    let mut edited = false;
    if !st.browsing {
        ctx.input(|i| {
            for e in &i.events {
                match e {
                    egui::Event::Text(t) | egui::Event::Paste(t) => {
                        st.path.extend(t.chars().filter(|c| !c.is_control()));
                        edited = true;
                    }
                    egui::Event::Key { key: Key::Backspace, pressed: true, modifiers, .. } => {
                        if modifiers.command {
                            st.path.clear();
                        } else {
                            st.path.pop();
                        }
                        edited = true;
                    }
                    egui::Event::Key { key: Key::Enter, pressed: true, .. } => action = Some(SetupAction::Import(st.path.trim().to_string())),
                    _ => {}
                }
            }
        });
    }
    if edited {
        st.error = None;
        st.note = None;
    }
    if action.is_none() && pressed(ctx, Key::Escape) && st.mode == SetupMode::Reimport {
        action = Some(SetupAction::Back);
    }
    let sub = match st.mode {
        SetupMode::FirstRun => "First start: game data needed",
        SetupMode::Reimport => "Game data: import again",
    };
    let clicked = frame(ctx, menu, egui::Order::Background, false, |g, _| {
        chrome(g, sub);
        let mut out = None;
        let x = g.stage.left() + 40.0;
        let w = g.stage.width() - 80.0;
        let mut y = g.r(0.0, 120.0, 0.0, 0.0).top();
        g.label(Pos2::new(x, y), "Locate your ThinkTanks installation", 34.0, pal::SAND, Align2::LEFT_TOP);
        y += 64.0;
        let intro = "The game needs the models, maps, sounds and music of ThinkTanks. They are converted once into the game's own format.";
        y += g.paragraph(Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 120.0)), intro, 24.0, pal::SAND) + 4.0;
        let local = if st.target.is_empty() {
            "Your files stay on this computer: they are only read, and the converted copy is kept locally. Nothing is uploaded.".to_string()
        } else {
            format!("Your files stay on this computer: they are only read, and the converted copy is kept in {}. Nothing is uploaded.", st.target)
        };
        y += g.paragraph(Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 90.0)), &local, 22.0, pal::SAND_MUTED) + 20.0;
        // Two columns: the folder (field, Browse, messages) and what to choose.
        let hint_w = 540.0;
        let hint = Rect::from_min_max(Pos2::new(x + w - hint_w, y), Pos2::new(x + w, g.action(0.0, 10.0).top() - 18.0));
        what_to_choose(g, hint);
        let w = w - hint_w - 24.0;
        g.text(Pos2::new(x, y), "ThinkTanks folder", 22.0, pal::SAND_MUTED, Align2::LEFT_TOP);
        y += 30.0;
        let browse_w = 170.0;
        let field = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w - browse_w - 12.0, 54.0));
        let shown = field_text(g, &st.path, field.width() - 40.0);
        g.text_field(field, &shown);
        let browse = Rect::from_min_size(Pos2::new(field.right() + 12.0, y), Vec2::new(browse_w, 54.0));
        let label = if st.browsing { "Choosing..." } else { "Browse..." };
        if g.button("setup-browse", browse, label, false, false, st.can_browse && !st.browsing).clicked() {
            out = Some(SetupAction::Browse);
        }
        y += 70.0;
        if let Some(e) = &st.error {
            g.paragraph(Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 100.0)), e, 22.0, pal::RED);
        } else if let Some(n) = &st.note {
            g.paragraph(Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 100.0)), n, 22.0, pal::SAND_MUTED);
        } else if !st.can_browse {
            g.paragraph(Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 100.0)), "Type or paste the folder's path (no folder dialog is available here).", 22.0, pal::SAND_MUTED);
        }
        let (left, left_action) = match st.mode {
            SetupMode::FirstRun => ("Quit", SetupAction::Quit),
            SetupMode::Reimport => ("Back", SetupAction::Back),
        };
        if g.button("setup-left", g.action_left(0.0, 200.0), left, false, false, true).clicked() {
            out = Some(left_action);
        }
        let ready = !st.path.trim().is_empty() && !st.browsing;
        if g.button("setup-import", g.action(0.0, 220.0), "Import", true, false, ready).clicked() {
            out = Some(SetupAction::Import(st.path.trim().to_string()));
        }
        out
    });
    clicked.or(action)
}

fn importing(ctx: &egui::Context, st: &mut SetupState, menu: &mut MenuState) -> Option<SetupAction> {
    let p = st.progress.clone().unwrap_or_default();
    let mut action = (pressed(ctx, Key::Escape) && !st.cancelling).then_some(SetupAction::Cancel);
    let clicked = frame(ctx, menu, egui::Order::Background, false, |g, _| {
        chrome(g, "Importing the game data");
        let x = g.stage.left() + 40.0;
        let w = g.stage.width() - 80.0;
        let mut y = g.r(0.0, 120.0, 0.0, 0.0).top();
        g.label(Pos2::new(x, y), "Importing ThinkTanks", 34.0, pal::SAND, Align2::LEFT_TOP);
        y += 64.0;
        let from = if st.path.is_empty() { String::new() } else { format!("From {}", st.path) };
        g.text_fit(Pos2::new(x, y + 12.0), &from, 22.0, pal::SAND_MUTED, w);
        y += 70.0;
        let phase = if st.cancelling { "Cancelling..." } else { p.phase.as_str() };
        g.text(Pos2::new(x, y), phase, 30.0, pal::BRASS, Align2::LEFT_TOP);
        let pct = if p.total > 0 { (p.done as f32 / p.total as f32).clamp(0.0, 1.0) } else { 0.0 };
        g.text(Pos2::new(x + w, y), &format!("{}%", (pct * 100.0).floor()), 30.0, pal::SAND, Align2::RIGHT_TOP);
        y += 46.0;
        let bar = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 30.0));
        g.bar(bar, pct);
        y += 44.0;
        g.text(Pos2::new(x, y), &format!("{} of {} files", p.done, p.total), 22.0, pal::SAND, Align2::LEFT_TOP);
        g.text_fit(Pos2::new(x + 260.0, y + 11.0), &p.file, 22.0, pal::SAND_MUTED, w - 260.0);
        y += 60.0;
        let tip = format!("{} This takes a minute or two. Cancel leaves the game data as it was.", st.note.as_deref().unwrap_or_default());
        g.paragraph(Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 80.0)), tip.trim_start(), 22.0, pal::SAND_MUTED);
        let mut out = None;
        if g.button("setup-cancel", g.action(0.0, 220.0), if st.cancelling { "Cancelling" } else { "Cancel" }, false, false, !st.cancelling).clicked() {
            out = Some(SetupAction::Cancel);
        }
        out
    });
    ctx.request_repaint();
    if clicked.is_some() {
        action = clicked;
    }
    action
}

/// Height of a line of the report's list.
const REPORT_LINE: f32 = 28.0;

/// The import's report: the summary, the scrollable list of what was
/// skipped or missing, "Open log folder" and Continue (or Back).
fn report(ctx: &egui::Context, st: &mut SetupState, menu: &mut MenuState) -> Option<SetupAction> {
    let r = st.report.clone().unwrap_or_default();
    let mut action = None;
    if pressed(ctx, Key::Enter) || pressed(ctx, Key::Escape) {
        action = Some(SetupAction::Continue);
    }
    // Keys scroll the list as the wheel does (`Gfx::scroll_offset` clamps).
    let keys = [(Key::ArrowDown, 1.0), (Key::ArrowUp, -1.0), (Key::PageDown, 10.0), (Key::PageUp, -10.0)];
    let step: f32 = keys.iter().filter(|(k, _)| pressed(ctx, *k)).map(|(_, n)| n * REPORT_LINE).sum();
    if step != 0.0 {
        let key = egui::Id::new(("tt-menu-scroll-px", "setup-report"));
        ctx.data_mut(|d| d.insert_temp(key, (d.get_temp::<f32>(key).unwrap_or(0.0) + step).max(0.0)));
    }
    let clicked = frame(ctx, menu, egui::Order::Background, false, |g, _| {
        chrome(g, if r.ok { "Game data imported" } else { "Game data: not imported" });
        let x = g.stage.left() + 40.0;
        let w = g.stage.width() - 80.0;
        let mut y = g.r(0.0, 120.0, 0.0, 0.0).top();
        g.label(Pos2::new(x, y), if r.ok { "Import complete" } else { "Import failed" }, 34.0, pal::SAND, Align2::LEFT_TOP);
        y += 60.0;
        y += g.paragraph(Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 110.0)), &r.summary, 22.0, if r.ok { pal::SAND } else { pal::RED }) + 10.0;
        if !r.log.is_empty() {
            g.text_fit(Pos2::new(x, y + 11.0), &format!("Full report: {}", r.log), 20.0, pal::SAND_MUTED, w);
            y += 34.0;
        }
        let bottom = g.action(0.0, 10.0).top() - 18.0;
        let list = g.well(Rect::from_min_max(Pos2::new(x, y), Pos2::new(x + w, bottom)));
        let inner = list.shrink2(Vec2::new(14.0, 8.0));
        let lines: Vec<&str> = if r.lines.is_empty() { vec!["Nothing was skipped: every file of the game was found."] } else { r.lines.iter().map(String::as_str).collect() };
        let content_h = lines.len() as f32 * REPORT_LINE;
        let off = g.scroll_offset("setup-report", inner, content_h);
        g.clipped(inner, |g| {
            for (i, l) in lines.iter().enumerate() {
                let ly = inner.top() + i as f32 * REPORT_LINE - off;
                if ly + REPORT_LINE < inner.top() || ly > inner.bottom() {
                    continue;
                }
                let head = !l.starts_with(' ');
                let colour = if head && !r.lines.is_empty() { pal::BRASS } else { pal::SAND };
                let indent = if head { 0.0 } else { 22.0 };
                g.text_fit(Pos2::new(inner.left() + indent, ly + REPORT_LINE / 2.0), l.trim_start(), 20.0, colour, inner.width() - 12.0 - indent);
            }
        });
        let mut out = None;
        if g.button("setup-log", g.action_left(0.0, 260.0), "Open log folder", false, false, !r.log.is_empty()).clicked() {
            out = Some(SetupAction::OpenLogFolder);
        }
        if g.button("setup-continue", g.action(0.0, 220.0), if r.ok { "Continue" } else { "Back" }, true, false, true).clicked() {
            out = Some(SetupAction::Continue);
        }
        out
    });
    clicked.or(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(st: &mut SetupState, events: Vec<egui::Event>) -> Option<SetupAction> {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = None;
        for ev in [Vec::new(), events] {
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))), events: ev, ..Default::default() };
            let _ = ctx.run(raw, |ctx| out = st.ui(ctx));
        }
        out
    }

    #[test]
    fn typing_and_enter_import_the_path() {
        let mut st = SetupState::new(SetupMode::FirstRun);
        st.error = Some("old".into());
        let a = run(&mut st, vec![egui::Event::Text("/games/tt".into()), egui::Event::Key { key: Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        assert_eq!(a, None);
        assert_eq!(st.path, "/games/t");
        assert_eq!(st.error, None);
        let a = run(&mut st, vec![egui::Event::Paste("t ".into()), egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        assert_eq!(a, Some(SetupAction::Import("/games/tt".into())));
    }

    #[test]
    fn escape_leaves_only_a_reimport_and_cancels_an_import() {
        let esc = || vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }];
        let mut first = SetupState::new(SetupMode::FirstRun);
        assert_eq!(run(&mut first, esc()), None);
        let mut again = SetupState::new(SetupMode::Reimport);
        assert_eq!(run(&mut again, esc()), Some(SetupAction::Back));
        again.progress = Some(SetupProgress { phase: "Converting textures".into(), done: 10, total: 100, file: "x.png".into() });
        assert_eq!(run(&mut again, esc()), Some(SetupAction::Cancel));
    }

    #[test]
    fn the_report_continues_on_enter() {
        let mut st = SetupState::new(SetupMode::FirstRun);
        st.report = Some(SetupReport { ok: true, summary: "300 files imported, 9 skipped".into(), lines: (0..50).map(|i| format!("  file{i}")).collect(), log: "/tmp/logs/import.log".into() });
        assert_eq!(run(&mut st, Vec::new()), None);
        let enter = vec![egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }];
        assert_eq!(run(&mut st, enter), Some(SetupAction::Continue));
    }
}
