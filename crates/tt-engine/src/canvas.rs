//! The canvas: the screen on it (the content) and the dialogs over it, by
//! layer. The game's screens are the 3D view (`PlayGui`), the end-of-round
//! screen (`EndGameGui`) and the places the menu shows (the loading screen,
//! the solo screen, the main menu, ...); the dialogs are the chat input
//! (`MessageHud`) and the player list (`PlayerListGui`). Nothing is drawn
//! from them: the menu ([`crate::menu`]) follows the content, the native HUD
//! ([`crate::hud`]) the dialogs, and the 3D view fills the window while
//! `PlayGui` is the content.
//!
//! Adding a screen to the canvas wakes it, removing it puts it to sleep
//! (`PlayGui`'s and `EndGameGui`'s `onWake`/`onSleep`, [`wake`] /
//! [`sleep`]). The changes the game's client code makes ([`set_content`],
//! [`push_dialog`], [`pop_dialog`]) then check the cursor: on while a screen
//! without `noCursor` is up, else off with the mouse locked to the game. The
//! shell's own screen changes ([`show`]) do not (they set the content
//! directly, as the shell's stand-ins did).

use crate::host::EngineHost;

/// A screen or dialog of the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ctrl {
    PlayGui,
    EndGameGui,
    MessageHud,
    PlayerListGui,
    LoadPlayerGui,
    TankSelectionGui,
    SoloPlayGui,
    SoloGameOverGui,
    LoadingGui,
    ConnErrorGui,
    JoinGameGui,
    MarqueeGui,
}

impl Ctrl {
    pub const ALL: [Ctrl; 12] = [
        Ctrl::PlayGui,
        Ctrl::EndGameGui,
        Ctrl::MessageHud,
        Ctrl::PlayerListGui,
        Ctrl::LoadPlayerGui,
        Ctrl::TankSelectionGui,
        Ctrl::SoloPlayGui,
        Ctrl::SoloGameOverGui,
        Ctrl::LoadingGui,
        Ctrl::ConnErrorGui,
        Ctrl::JoinGameGui,
        Ctrl::MarqueeGui,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Ctrl::PlayGui => "PlayGui",
            Ctrl::EndGameGui => "EndGameGui",
            Ctrl::MessageHud => "MessageHud",
            Ctrl::PlayerListGui => "PlayerListGui",
            Ctrl::LoadPlayerGui => "loadPlayerGui",
            Ctrl::TankSelectionGui => "tankSelectionGui",
            Ctrl::SoloPlayGui => "soloPlayGui",
            Ctrl::SoloGameOverGui => "soloGameOverGui",
            Ctrl::LoadingGui => "loadingGui",
            Ctrl::ConnErrorGui => "connErrorGui",
            Ctrl::JoinGameGui => "joinGameGui",
            Ctrl::MarqueeGui => "MarqueeGui",
        }
    }

    /// By name (case-insensitive).
    pub fn from_name(name: &str) -> Option<Ctrl> {
        Ctrl::ALL.iter().copied().find(|c| c.name().eq_ignore_ascii_case(name))
    }

    /// `noCursor = "1"`: the cursor stays off while it is up.
    pub fn no_cursor(self) -> bool {
        matches!(self, Ctrl::PlayGui | Ctrl::MessageHud | Ctrl::PlayerListGui)
    }
}

#[derive(Debug, Default)]
pub struct Canvas {
    /// The canvas exists (`createCanvas`; none on a dedicated server).
    pub exists: bool,
    /// What is on the canvas, in order: the content first, then the
    /// dialogs by layer; (control, layer).
    members: Vec<(Ctrl, i32)>,
    pub cursor_on: bool,
    pub cursor: String,
    /// Log of stack operations ("setContent loadingGui", ...), for reports.
    pub history: Vec<String>,
}

impl Canvas {
    pub fn new() -> Canvas {
        Canvas { cursor_on: true, ..Default::default() }
    }

    /// The content (first on the canvas).
    pub fn content(&self) -> Option<Ctrl> {
        self.members.first().map(|m| m.0)
    }

    /// The content's name ("" with nothing on the canvas).
    pub fn content_name(&self) -> &'static str {
        self.content().map(Ctrl::name).unwrap_or("")
    }

    /// On the canvas (awake).
    pub fn is_awake(&self, c: Ctrl) -> bool {
        self.members.iter().any(|m| m.0 == c)
    }

    pub fn describe(&self) -> String {
        if self.members.is_empty() {
            return "(empty)".into();
        }
        let mut s = String::new();
        for (i, (c, layer)) in self.members.iter().enumerate() {
            if i == 0 {
                s += &format!("content {}", c.name());
            } else {
                s += &format!(", dialog {} (layer {layer})", c.name());
            }
        }
        s
    }

}

/// A control coming onto the canvas: its `onWake`.
fn wake(h: &mut EngineHost, c: Ctrl) {
    match c {
        Ctrl::PlayGui => crate::gameplay::client::hud::play_gui_on_wake(h),
        Ctrl::EndGameGui => h.client.end_game.loading = "0".into(),
        _ => {}
    }
}

/// A control leaving the canvas: its `onSleep`.
fn sleep(h: &mut EngineHost, c: Ctrl) {
    if c == Ctrl::PlayGui {
        h.input.pop_map(crate::input::MOVE_MAP);
    }
}

fn remove(h: &mut EngineHost, c: Ctrl) {
    if !h.canvas.is_awake(c) {
        return;
    }
    sleep(h, c);
    h.canvas.members.retain(|m| m.0 != c);
}

fn add(h: &mut EngineHost, c: Ctrl, layer: i32, index: usize) {
    let was_awake = h.canvas.is_awake(c);
    let members = &mut h.canvas.members;
    members.retain(|m| m.0 != c);
    let i = index.min(members.len());
    members.insert(i, (c, layer));
    if !was_awake {
        wake(h, c);
    }
}

/// `Canvas.setContent(c)` without the cursor check: replaces the content
/// and every layer-0 dialog.
fn set_content_raw(h: &mut EngineHost, c: Ctrl) {
    if !h.canvas.exists {
        return;
    }
    let now = h.sim_time();
    h.canvas.history.push(format!("{now}ms setContent {}", c.name()));
    let members = h.canvas.members.clone();
    for (m, layer) in members {
        if m != c && layer == 0 {
            remove(h, m);
        }
    }
    if h.canvas.members.first().map(|m| m.0) == Some(c) {
        h.canvas.members[0].1 = 0;
    } else {
        add(h, c, 0, 0);
    }
}

/// The shell puts screen `c` on the canvas (a stand-in's `show()`).
pub fn show(h: &mut EngineHost, c: Ctrl) {
    set_content_raw(h, c);
}

/// `Canvas.setContent(c)` from the game's client code (cursor checked).
pub fn set_content(h: &mut EngineHost, c: Ctrl) {
    set_content_raw(h, c);
    check_cursor(h);
}

/// `Canvas.pushDialog(c, layer)`: on top of every dialog of the same or a
/// lower layer (cursor checked).
pub fn push_dialog(h: &mut EngineHost, c: Ctrl, layer: i32) {
    if !h.canvas.exists {
        return;
    }
    let now = h.sim_time();
    h.canvas.history.push(format!("{now}ms pushDialog {} {layer}", c.name()));
    let on_canvas = h.canvas.is_awake(c);
    if on_canvas {
        h.canvas.members.retain(|m| m.0 != c);
    }
    let members = h.canvas.members.clone();
    // Index 0 is the content; after the last control with layer <= ours.
    let mut index = members.len();
    for (i, (_, l)) in members.iter().enumerate().skip(1) {
        if *l > layer {
            index = i;
            break;
        }
    }
    if on_canvas {
        let i = index.min(h.canvas.members.len());
        h.canvas.members.insert(i, (c, layer));
    } else {
        add(h, c, layer, index.max(usize::from(!members.is_empty())));
    }
    check_cursor(h);
}

/// `Canvas.popDialog(c)` (cursor checked).
pub fn pop_dialog(h: &mut EngineHost, c: Ctrl) {
    if !h.canvas.exists {
        return;
    }
    let now = h.sim_time();
    h.canvas.history.push(format!("{now}ms popDialog {}", c.name()));
    if h.canvas.is_awake(c) {
        remove(h, c);
    }
    check_cursor(h);
}

/// The cursor on while anything on the canvas lacks `noCursor`, else
/// off; switching it locks or frees the mouse (`cursorOn` /
/// `cursorOff`).
fn check_cursor(h: &mut EngineHost) {
    let should_be_on = h.canvas.members.iter().any(|(c, _)| !c.no_cursor());
    if h.canvas.cursor_on != should_be_on {
        if should_be_on {
            cursor_on(h);
        } else {
            cursor_off(h);
        }
    }
}

/// `cursorOn()`.
pub fn cursor_on(h: &mut EngineHost) {
    if h.cl.cursor_controlled {
        h.mouse_locked = false;
    }
    h.canvas.cursor_on = true;
    h.canvas.cursor = "DefaultCursor".into();
}

/// `cursorOff()`.
pub fn cursor_off(h: &mut EngineHost) {
    if h.cl.cursor_controlled {
        h.mouse_locked = true;
    }
    h.canvas.cursor_on = false;
}
