//! New shell menus, built with egui.
//!
//! Flow changes (start a game, resume, quit) are returned from
//! [`MenuState::ui`] as [`MenuAction`]s.
//!
//! Integration (engine side): call [`theme::install_fonts`] once, set
//! `pixels_per_point` from
//! [`theme::pixels_per_point`] each frame, run `MenuState::ui` inside the
//! egui frame, and translate gamepad input to arrow keys / Enter / Escape.

pub mod art;
pub mod backend;
pub mod hud;
pub mod logo;
mod screens;
pub mod theme;

pub use backend::*;
pub use screens::setup::{SetupAction, SetupMode, SetupProgress, SetupReport, SetupState};

/// Screens of the new menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Choose / create / delete the player profile (first screen, like `loadPlayerGui`).
    PlayerSelect,
    /// "Play the tutorial?" after creating a player.
    TutorialPrompt,
    /// The 8 tutorial slides of `MarqueeGui`, then target practice.
    Tutorial,
    MainMenu,
    /// Tank choice with stats and a 3D preview.
    Garage,
    /// Solo campaign between levels: next level, briefing, lives, score (`soloPlayGui`).
    Solo,
    /// Solo top-10 (also shown after a game over, with the new entry highlighted).
    HighScores,
    /// Mission loading progress (`loadingGui`).
    Loading,
    /// Local skirmish on any map with bots.
    CustomGame,
    /// Online play: join a server (LAN list, by address) or host one.
    Online,
    Settings,
    Help,
    Credits,
    QuitConfirm,
    /// In-game menu (Escape during play; replaces `quitDlg`).
    Pause,
    /// Round results over the game (replaces `EndGameGui`).
    Results,
    /// The server rotation's maps over the game: pick one to call a vote.
    VoteMap,
}

/// A flow change for the engine to perform.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    /// Start the tutorial (the game's "Intro" mission).
    StartTutorial,
    /// Solo: start a new run (`launchSPServer()`) or, during a run, the next
    /// level (`soloPlayGui.finishScore(); onCyclePauseEnd()`).
    StartSolo,
    /// Solo level select: a new run starting at this campaign position (3
    /// lives, score 0, the difficulty the run would have there).
    StartSoloAt(u32),
    /// Quick Play (the game's quick-play path).
    QuickPlay,
    /// Start a local custom game.
    StartCustom(CustomGame),
    /// Host a listen server and play on it.
    Host(HostGame),
    /// Join the server at `address` (`host[:port]` or `IP:a.b.c.d:port`).
    /// `team`: the team asked for in a team game (`Auto`: the server picks).
    Join { address: String, password: String, team: Team },
    /// Close the in-game menu and continue playing.
    Resume,
    /// Call a vote for this map of the rotation (its id), then play on.
    CallVote(String),
    /// Open the rotation's map list to call a vote (as F10 does).
    VoteMenu,
    /// Leave the running game and return to the menus.
    LeaveGame,
    /// Quit the program.
    Quit,
    /// Settings → Game data → Re-import: the engine shuts the game down,
    /// shows the import screen and boots again on the new data.
    ReimportGameData,
    /// Options → Mods → Restart: the engine shuts the game down and boots
    /// it again (the enabled mods apply).
    Restart,
}

/// The two halves of the Online screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnlineTab {
    #[default]
    Join,
    Host,
}

/// A text field of the Online screen with the keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Address,
    ServerName,
    ServerPassword,
}

/// The master servers dialog (Join → Internet): one address a row (the
/// pref stores them comma-separated), the row typed into, and whether
/// Save found an entry that is not `host[:port]`.
#[derive(Debug, Clone, Default)]
pub struct MastersEdit {
    pub rows: Vec<String>,
    pub focus: usize,
    pub invalid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Display,
    Graphics,
    Audio,
    Controls,
    /// Where the game data came from, and importing it again.
    GameData,
    /// The installed mods: enable, order, open their folder.
    Mods,
}

/// Menu state machine. Create with [`MenuState::new`] for the shell or
/// [`MenuState::pause`] for the in-game menu. The public fields are set
/// directly only by the previews. ([`Default`] is the bare state the
/// constructors start from: no screen stack, which shows the player screen.)
#[derive(Default)]
pub struct MenuState {
    stack: Vec<Screen>,
    /// Keyboard/gamepad focus index in the current screen's main list.
    pub focus: usize,
    pub(crate) player_sel: usize,
    pub(crate) new_player: Option<String>,
    pub(crate) confirm_delete: Option<String>,
    pub(crate) message: Option<String>,
    pub(crate) settings: Option<Settings>,
    pub(crate) settings_tab: SettingsTab,
    /// Options → Mods: the list (loaded when the tab opens) and the selected mod's id.
    pub(crate) mods: Option<ModsInfo>,
    pub(crate) mods_sel: Option<String>,
    /// The backend has mods (asked once).
    pub(crate) mods_supported: Option<bool>,
    pub custom: Option<CustomGame>,
    pub(crate) player_init: bool,
    /// Interface sounds requested this frame (played through the backend).
    pub(crate) sounds: Vec<UiSound>,
    /// The button under the pointer this frame / the last frame (`soundButtonOver`).
    pub(crate) hover_now: Option<egui::Id>,
    pub(crate) hover_last: Option<egui::Id>,
    /// The solo tally animation.
    pub(crate) tally: screens::TallyAnim,
    /// The tutorial slide show.
    pub(crate) slides: screens::Slides,
    /// Solo level select: the chosen campaign position (None: the
    /// highest unlocked one when the screen opens).
    pub(crate) solo_level: Option<u32>,
    // ---- online
    pub online_tab: OnlineTab,
    /// Selected server (by address, so it survives refreshes).
    pub join_sel: Option<String>,
    /// Sort column of the server list and ascending.
    pub(crate) sort: (usize, bool),
    /// The "Join by address" text.
    pub(crate) address: String,
    pub(crate) field: Option<Field>,
    /// The password prompt: (address, typed password).
    pub password: Option<(String, String)>,
    pub host: Option<HostGame>,
    /// A LAN query was started on entering the screen.
    pub(crate) queried: bool,
    /// The join list's source (LAN / Internet).
    pub(crate) server_source: ServerSource,
    /// The team asked for when joining a team game.
    pub(crate) join_team: Team,
    /// The master servers dialog, while open.
    pub masters: Option<MastersEdit>,
    /// The server details show the player list instead of the info.
    pub show_players: bool,
    /// The Host screen shows the bots instead of the map rotation.
    pub host_bots: bool,
    /// Escape was used by the screen (leaving a text field).
    pub(crate) esc_eaten: bool,
}

impl MenuState {
    /// Shell menus, starting at player selection.
    pub fn new() -> MenuState {
        Self::at(Screen::PlayerSelect)
    }

    /// The in-game menu.
    pub fn pause() -> MenuState {
        Self::at(Screen::Pause)
    }

    /// Start at a given screen (previews, returning from a game to the main menu).
    pub fn at(screen: Screen) -> MenuState {
        MenuState { stack: vec![screen], sort: (0, true), ..Default::default() }
    }

    /// The join list's source (previews).
    pub fn set_server_source(&mut self, source: ServerSource) {
        self.server_source = source;
    }

    pub fn screen(&self) -> Screen {
        self.stack.last().copied().unwrap_or(Screen::PlayerSelect)
    }

    /// Go to a screen (pushes it; Escape / Back returns).
    pub fn open(&mut self, s: Screen) {
        self.stack.push(s);
        self.focus = 0;
        self.message = None;
    }

    /// Replace the whole stack with one screen.
    pub fn reset(&mut self, s: Screen) {
        self.stack = vec![s];
        self.focus = 0;
        self.message = None;
    }

    /// Back one screen; `false` at the root.
    pub fn back(&mut self) -> bool {
        if self.stack.len() > 1 {
            self.stack.pop();
            self.focus = 0;
            self.message = None;
            self.settings = None;
            true
        } else {
            false
        }
    }

    /// Open the "new player" dialog (on the player screen) with a prefilled name.
    pub fn show_new_player_dialog(&mut self, name: &str) {
        self.new_player = Some(name.to_string());
    }

    /// Put tutorial slide `n` (1..8) up at time 0, faded in after 1 ms (previews).
    pub fn show_tutorial_slide(&mut self, n: u32) {
        self.slides = screens::Slides { started: true, cur: n.clamp(1, 8), shown_at: 0.0, fade_in_ms: 1.0, pending: None };
    }

    /// Select the settings tab (previews).
    pub fn set_settings_tab(&mut self, tab: SettingsTab) {
        self.settings_tab = tab;
    }

    /// Draw the current screen and handle input. Returns a flow change for
    /// the engine, if any.
    pub fn ui(&mut self, ctx: &egui::Context, backend: &mut dyn MenuBackend) -> Option<MenuAction> {
        // Fonts not installed by the host yet: install them;
        // they take effect on the next frame.
        if theme::ensure(ctx) {
            return None;
        }
        let action = screens::ui(self, ctx, backend);
        // `soundButtonOver` when the pointer enters a button, `soundButtonDown` on a press.
        let hover = self.hover_now.take();
        if hover.is_some() && hover != self.hover_last {
            self.sounds.insert(0, UiSound::ButtonOver);
        }
        self.hover_last = hover;
        for s in std::mem::take(&mut self.sounds) {
            backend.play_ui_sound(s);
        }
        action
    }
}

pub fn strip_markup(s: &str) -> String {
    strip_tags(s).chars().filter(|&c| c == '\n' || c == '\t' || c >= ' ').collect()
}

/// `s` without its `<...>` tags (an unclosed `<` and what follows stay).
pub(crate) fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        match rest[i..].find('>') {
            Some(j) => rest = &rest[i + j + 1..],
            None => {
                rest = &rest[i..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Display modes sorted (largest first) and de-duplicated; the desktop mode
/// is included even if the backend did not list it.
pub fn sorted_modes(modes: &[DisplayMode], desktop: DisplayMode) -> Vec<DisplayMode> {
    let mut v: Vec<DisplayMode> = modes.iter().copied().chain(std::iter::once(desktop)).filter(|m| m.width >= 640 && m.height >= 480).collect();
    v.sort_by_key(|m| std::cmp::Reverse((m.width * m.height, m.width)));
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_and_modes() {
        assert_eq!(strip_markup("<font:Arial Bold:22>Title\n<just:center>x\x01y"), "Title\nxy");
        let d = |w, h| DisplayMode { width: w, height: h };
        let m = sorted_modes(&[d(1280, 720), d(1920, 1080), d(1280, 720), d(320, 200)], d(2560, 1440));
        assert_eq!(m, vec![d(2560, 1440), d(1920, 1080), d(1280, 720)]);
    }

    fn run(state: &mut MenuState, backend: &mut MockBackend, events: Vec<egui::Event>) -> Option<MenuAction> {
        let ctx = egui::Context::default();
        let mut action = None;
        for ev in [Vec::new(), events] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
                events: ev,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                if let Some(a) = state.ui(ctx, backend) {
                    action = Some(a);
                }
            });
        }
        action
    }

    /// Frames of pointer events: move, press, release at `pos` (one frame each).
    fn click(state: &mut MenuState, backend: &mut MockBackend, ctx: &egui::Context, pos: egui::Pos2) -> Option<MenuAction> {
        let mut action = None;
        let ev = |e: Vec<egui::Event>| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
            events: e,
            ..Default::default()
        };
        let btn = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for e in [vec![egui::Event::PointerMoved(pos)], vec![btn(true)], vec![btn(false)], vec![]] {
            let _ = ctx.run(ev(e), |ctx| {
                if let Some(a) = state.ui(ctx, backend) {
                    action = Some(a);
                }
            });
        }
        action
    }

    #[test]
    fn mouse_clicks() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        // Player list: click the second row (Bob), then Accept.
        let mut s = MenuState::new();
        let _ = ctx.run(Default::default(), |ctx| {
            s.ui(ctx, &mut b);
        });
        click(&mut s, &mut b, &ctx, egui::pos2(300.0, 22.0 + 176.0 + 52.0 + 26.0));
        assert_eq!(s.player_sel, 1, "row click");
        click(&mut s, &mut b, &ctx, egui::pos2(1130.0, 671.0));
        assert_eq!(s.screen(), Screen::MainMenu);
        assert_eq!(b.log, vec!["select Bob"]);
        // Main menu: clicking "Quick Play" starts it; "Custom Game" opens its screen.
        let a = click(&mut s, &mut b, &ctx, egui::pos2(200.0, 22.0 + 120.0 + 104.0 + 46.0));
        assert_eq!(a, Some(MenuAction::QuickPlay));
        click(&mut s, &mut b, &ctx, egui::pos2(200.0, 22.0 + 120.0 + 2.0 * 104.0 + 46.0));
        assert_eq!(s.screen(), Screen::CustomGame);
        // Map list row 3 (rows of 35 below a 40 header in a well at y 134).
        click(&mut s, &mut b, &ctx, egui::pos2(150.0, 22.0 + 136.0 + 40.0 + 3.0 * 35.0 + 17.0));
        // Launch.
        let a = click(&mut s, &mut b, &ctx, egui::pos2(1130.0, 671.0));
        match a {
            Some(MenuAction::StartCustom(g)) => assert_eq!(g.mission, "missions/tt1_4.ron"),
            other => panic!("expected StartCustom, got {other:?}"),
        }
        // New-player dialog: open it with the mouse, type, click Create.
        let mut s = MenuState::new();
        click(&mut s, &mut b, &ctx, egui::pos2(150.0, 671.0));
        assert!(s.new_player.is_some());
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
            events: vec![egui::Event::Text("Zed".into())],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            s.ui(ctx, &mut b);
        });
        click(&mut s, &mut b, &ctx, egui::pos2(872.0, 479.0));
        assert!(b.log.contains(&"create Zed".to_string()), "{:?}", b.log);
        assert_eq!(s.screen(), Screen::TutorialPrompt);
        let mut s = MenuState::at(Screen::MainMenu);
        // The top-bar Quit button opens the quit dialog.
        click(&mut s, &mut b, &ctx, egui::pos2(1280.0 - 40.0 - 60.0, 54.0));
        assert_eq!(s.screen(), Screen::QuitConfirm);
    }

    fn key(k: egui::Key) -> egui::Event {
        egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
    }

    #[test]
    fn keyboard_flow() {
        let mut b = MockBackend::new();
        let mut s = MenuState::new();
        // Accept the selected player with Enter -> main menu.
        run(&mut s, &mut b, vec![key(egui::Key::Enter)]);
        assert_eq!(s.screen(), Screen::MainMenu);
        assert_eq!(b.log, vec!["select Alice"]);
        // Down to "Quick Play" and Enter.
        let a = run(&mut s, &mut b, vec![key(egui::Key::ArrowDown), key(egui::Key::Enter)]);
        assert_eq!(a, Some(MenuAction::QuickPlay));
        // Escape at the main menu asks to quit.
        run(&mut s, &mut b, vec![key(egui::Key::Escape)]);
        assert_eq!(s.screen(), Screen::QuitConfirm);
        let a = run(&mut s, &mut b, vec![key(egui::Key::Enter)]);
        assert_eq!(a, Some(MenuAction::Quit));
        // In-game menu: Escape resumes.
        let mut p = MenuState::pause();
        assert_eq!(run(&mut p, &mut b, vec![key(egui::Key::Escape)]), Some(MenuAction::Resume));
    }

    /// One frame at time `t` (seconds).
    fn frame_at(ctx: &egui::Context, state: &mut MenuState, backend: &mut MockBackend, t: f64, events: Vec<egui::Event>) -> Option<MenuAction> {
        // Frames without a time (`click`) advance egui's clock by itself;
        // never step it back (egui asserts on that in debug builds).
        let t = t.max(ctx.input(|i| i.time));
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
            // Never behind the clicks' frames (egui's history wants monotonic time).
            time: Some(t.max(ctx.input(|i| i.time))),
            events,
            ..Default::default()
        };
        let mut action = None;
        let _ = ctx.run(input, |ctx| action = state.ui(ctx, backend));
        action
    }

    #[test]
    fn solo_tally_animation_timing_and_sounds() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        let mut s = MenuState::at(Screen::Solo);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new()); // fonts
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        assert!(b.sounds.is_empty());
        // presentScore: the first advanceScore after 1 s ("Bonus Points:", SPBonus),
        // then 1.5 s until the bronze row (SPCount1), 0.75 s to the first count (SPCount2).
        frame_at(&ctx, &mut s, &mut b, 0.999, Vec::new());
        assert!(b.sounds.is_empty());
        frame_at(&ctx, &mut s, &mut b, 1.0, Vec::new());
        assert_eq!(b.sounds, vec![UiSound::Bonus]);
        frame_at(&ctx, &mut s, &mut b, 2.5, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 3.25, Vec::new());
        assert_eq!(b.sounds, vec![UiSound::Bonus, UiSound::Count1, UiSound::Count2]);
        // Everything up to the end: one sound per step that has one.
        frame_at(&ctx, &mut s, &mut b, 60.0, Vec::new());
        let want: Vec<UiSound> = SoloTally::example().steps.iter().filter_map(|x| x.event.sound()).collect();
        assert_eq!(b.sounds, want);
        assert_eq!(b.sounds.iter().filter(|&&x| x == UiSound::Extra).count(), 1);
        assert!(b.sounds.contains(&UiSound::Survival));
        // Coming back to the screen shows the final state without replaying.
        s.open(Screen::HighScores);
        frame_at(&ctx, &mut s, &mut b, 61.0, Vec::new());
        s.back();
        frame_at(&ctx, &mut s, &mut b, 62.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 70.0, Vec::new());
        assert_eq!(b.sounds.len(), want.len());
    }

    #[test]
    fn leaving_solo_finishes_the_tally_silently() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        let mut s = MenuState::at(Screen::Solo);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 1.2, Vec::new());
        assert_eq!(b.sounds, vec![UiSound::Bonus]);
        s.open(Screen::HighScores);
        frame_at(&ctx, &mut s, &mut b, 1.3, Vec::new());
        s.back();
        frame_at(&ctx, &mut s, &mut b, 30.0, Vec::new());
        assert_eq!(b.sounds, vec![UiSound::Bonus]);
    }

    #[test]
    fn tutorial_slides_then_target_practice() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        let mut s = MenuState::at(Screen::TutorialPrompt);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 0.0, vec![key(egui::Key::Enter)]);
        assert_eq!(s.screen(), Screen::Tutorial);
        frame_at(&ctx, &mut s, &mut b, 0.1, Vec::new());
        assert_eq!(s.slides.cur, 1);
        // MarqueeTime: the next slide after 5 s (fade out 2 x 150 ms, switch at 1.5 fades).
        frame_at(&ctx, &mut s, &mut b, 5.2, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 5.7, Vec::new());
        assert_eq!(s.slides.cur, 2);
        // Enter steps through the rest; on the last slide it starts target practice.
        let mut t = 6.0;
        let mut action = None;
        for _ in 0..6 {
            action = frame_at(&ctx, &mut s, &mut b, t, vec![key(egui::Key::Enter)]);
            t += 0.01;
            frame_at(&ctx, &mut s, &mut b, t, Vec::new());
            t += 0.01;
        }
        assert_eq!(action, None);
        assert_eq!(s.slides.pending.map_or(s.slides.cur, |p| p.0), 8);
        let a = frame_at(&ctx, &mut s, &mut b, t, vec![key(egui::Key::Enter)]);
        assert_eq!(a, Some(MenuAction::StartTutorial));
        // Escape on the slides goes back.
        let mut s = MenuState::at(Screen::MainMenu);
        s.open(Screen::Tutorial);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 0.1, vec![key(egui::Key::Escape)]);
        assert_eq!(s.screen(), Screen::MainMenu);
    }

    /// Game data without the tutorial's art (an edition of ThinkTanks
    /// without the slides): no prompt after a new player, no Tutorial
    /// button, and the slides' screens fall back to the main menu.
    #[test]
    fn no_tutorial_without_its_art() {
        let ctx = egui::Context::default();
        let pos = egui::pos2(420.0 + 40.0 + 100.0, 671.0);
        let has_tutorial_button = |b: &mut MockBackend, ctx: &egui::Context| {
            let mut s = MenuState::at(Screen::PlayerSelect);
            frame_at(ctx, &mut s, b, 0.0, Vec::new());
            let before = b.log.len();
            click(&mut s, b, ctx, pos);
            s.screen() == Screen::Tutorial || b.log.len() > before
        };
        for available in [true, false] {
            let mut b = MockBackend::new();
            b.tutorial = available;
            // After creating a player.
            b.tutorial_prompt = true;
            let mut s = MenuState::at(Screen::PlayerSelect);
            assert!(crate::screens::players::finish_player(&mut s, &mut b).is_none());
            assert_eq!(s.screen(), if available { Screen::TutorialPrompt } else { Screen::MainMenu });
            // The slides themselves.
            let mut s = MenuState::at(Screen::MainMenu);
            s.open(Screen::Tutorial);
            frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
            frame_at(&ctx, &mut s, &mut b, 0.1, Vec::new());
            assert_eq!(s.screen(), if available { Screen::Tutorial } else { Screen::MainMenu });
            assert_eq!(has_tutorial_button(&mut b, &ctx), available, "tutorial art: {available}");
        }
    }

    #[test]
    fn button_sounds() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        let mut s = MenuState::at(Screen::MainMenu);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        // Pointer onto the Play button: AudioButtonOver once, AudioButtonDown on the press.
        let pos = egui::pos2(1130.0, 671.0);
        frame_at(&ctx, &mut s, &mut b, 0.1, vec![egui::Event::PointerMoved(pos)]);
        frame_at(&ctx, &mut s, &mut b, 0.2, vec![egui::Event::PointerMoved(pos + egui::vec2(2.0, 0.0))]);
        assert_eq!(b.sounds, vec![UiSound::ButtonOver]);
        let btn = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame_at(&ctx, &mut s, &mut b, 0.3, vec![btn(true)]);
        assert_eq!(b.sounds, vec![UiSound::ButtonOver, UiSound::ButtonDown]);
        // The click opened Solo; off the buttons and onto its Launch button: another AudioButtonOver.
        frame_at(&ctx, &mut s, &mut b, 0.4, vec![btn(false), egui::Event::PointerMoved(egui::pos2(640.0, 400.0))]);
        frame_at(&ctx, &mut s, &mut b, 0.5, vec![egui::Event::PointerMoved(pos)]);
        frame_at(&ctx, &mut s, &mut b, 0.6, Vec::new());
        assert_eq!(b.sounds.iter().filter(|&&x| x == UiSound::ButtonOver).count(), 2, "{:?} {:?}", b.sounds, s.screen());
    }

    fn type_text(ctx: &egui::Context, s: &mut MenuState, b: &mut MockBackend, text: &str, enter: bool) -> Option<MenuAction> {
        let mut ev = vec![egui::Event::Text(text.into())];
        if enter {
            ev.push(key(egui::Key::Enter));
        }
        let mut a = frame_at(ctx, s, b, 1.0, ev);
        if a.is_none() {
            a = frame_at(ctx, s, b, 1.0, Vec::new());
        }
        a
    }

    /// The Host screen's rotation: clicked maps join it in order, a click
    /// on one of them takes it out (not the last), the first starts.
    #[test]
    fn host_rotation_picker() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        let mut s = MenuState::at(Screen::Online);
        s.online_tab = OnlineTab::Host;
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        assert_eq!(s.screen(), Screen::Online);
        let row = |k: usize| egui::pos2(700.0, 22.0 + 146.0 + 16.0 + 44.0 + 2.0 + 40.0 + k as f32 * 35.0 + 17.0);
        // A click a second after the last (two quick ones are a double click).
        let mut t = 1.0;
        let mut click = |s: &mut MenuState, b: &mut MockBackend, _: &egui::Context, pos: egui::Pos2| {
            t += 1.0;
            let btn = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            let mut a = None;
            for (dt, e) in [(0.0, egui::Event::PointerMoved(pos)), (0.05, btn(true)), (0.1, btn(false))] {
                a = a.or(frame_at(&ctx, s, b, t + dt, vec![e]));
            }
            a.or(frame_at(&ctx, s, b, t + 0.15, Vec::new()))
        };
        let rotation = |s: &MenuState| s.host.as_ref().unwrap().rotation.clone();
        let file = |k: usize| tt_assets::layout::mission(MAPS[k].0);
        assert_eq!(rotation(&s), vec![file(0)], "the first map alone");
        click(&mut s, &mut b, &ctx, row(2));
        click(&mut s, &mut b, &ctx, row(1));
        assert_eq!(rotation(&s), vec![file(0), file(2), file(1)]);
        click(&mut s, &mut b, &ctx, row(0));
        assert_eq!(rotation(&s), vec![file(2), file(1)], "out: the next one starts");
        assert_eq!(s.host.as_ref().unwrap().game.mission, file(2));
        click(&mut s, &mut b, &ctx, row(2));
        click(&mut s, &mut b, &ctx, row(1));
        assert_eq!(rotation(&s), vec![file(1)], "the last one stays");
        // "Random order" below the list: off at first, it needs two maps.
        let random = |s: &MenuState| s.host.as_ref().unwrap().rotation_random;
        let random_box = egui::pos2(1080.0, 201.0);
        assert!(!random(&s));
        click(&mut s, &mut b, &ctx, random_box);
        assert!(!random(&s), "one map: no order to choose");
        click(&mut s, &mut b, &ctx, row(3));
        click(&mut s, &mut b, &ctx, random_box);
        assert!(random(&s));
        // Down to one map: the order goes off with it.
        click(&mut s, &mut b, &ctx, row(3));
        assert!(!random(&s), "one map: no random order left on");
        click(&mut s, &mut b, &ctx, row(3));
        click(&mut s, &mut b, &ctx, random_box);
        assert!(random(&s));
        let a = click(&mut s, &mut b, &ctx, egui::pos2(1130.0, 671.0));
        match a {
            Some(MenuAction::Host(g)) => {
                assert_eq!(g.rotation, vec![file(1), file(3)]);
                assert_eq!(g.game.mission, file(1));
                assert!(g.rotation_random, "the random order goes with the game");
            }
            other => panic!("expected Host, got {other:?}"),
        }
    }

    /// The in-game vote menu: a digit or Enter calls the vote, Escape
    /// closes it.
    #[test]
    fn vote_menu_keys() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        b.vote_maps = vec![("tt1_1".into(), "Medulla Mesa".into()), ("tt2_3".into(), "Snowblind".into())];
        let mut s = MenuState::at(Screen::VoteMap);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        assert_eq!(frame_at(&ctx, &mut s, &mut b, 0.1, vec![key(egui::Key::Num2)]), Some(MenuAction::CallVote("tt2_3".into())));
        frame_at(&ctx, &mut s, &mut b, 0.2, vec![key(egui::Key::ArrowDown)]);
        frame_at(&ctx, &mut s, &mut b, 0.3, vec![key(egui::Key::ArrowDown)]);
        assert_eq!(frame_at(&ctx, &mut s, &mut b, 0.4, vec![key(egui::Key::Enter)]), Some(MenuAction::CallVote("tt1_1".into())));
        assert_eq!(frame_at(&ctx, &mut s, &mut b, 0.5, vec![key(egui::Key::Escape)]), Some(MenuAction::Resume));
        assert_eq!(frame_at(&ctx, &mut s, &mut b, 0.6, vec![key(egui::Key::Num9)]), None, "no ninth map");
    }

    #[test]
    fn pause_menu_votes_only_in_multiplayer() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        // A local game: the second item is Options.
        let mut s = MenuState::pause();
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 0.1, vec![key(egui::Key::ArrowDown)]);
        frame_at(&ctx, &mut s, &mut b, 0.2, vec![key(egui::Key::Enter)]);
        assert_eq!(s.screen(), Screen::Settings);
        // A multiplayer game: "Vote for a map" comes second.
        b.vote_maps = vec![("tt1_1".into(), "Medulla Mesa".into())];
        let mut s = MenuState::pause();
        frame_at(&ctx, &mut s, &mut b, 0.3, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 0.4, vec![key(egui::Key::ArrowDown)]);
        assert_eq!(frame_at(&ctx, &mut s, &mut b, 0.5, vec![key(egui::Key::Enter)]), Some(MenuAction::VoteMenu));
    }

    #[test]
    fn update_notice_opens_the_releases_page() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        let mut s = MenuState::at(Screen::MainMenu);
        let button = egui::pos2(520.0, 669.0);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        click(&mut s, &mut b, &ctx, button);
        assert!(!b.log.iter().any(|l| l == "open releases"));
        b.update = Some("v0.2.0".into());
        click(&mut s, &mut b, &ctx, button);
        assert_eq!((s.screen(), b.log.iter().filter(|l| *l == "open releases").count()), (Screen::MainMenu, 1));
    }

    /// The main menu's Discord button (bottom left) opens the invite.
    #[test]
    fn discord_button_opens_the_invite() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        let mut s = MenuState::at(Screen::MainMenu);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        click(&mut s, &mut b, &ctx, egui::pos2(140.0, 669.0));
        assert_eq!(b.log.last().map(String::as_str), Some(format!("open {}", screens::DISCORD_INVITE).as_str()));
        assert_eq!(s.screen(), Screen::MainMenu);
    }

    #[test]
    fn online_join_password_address_and_host() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        let mut s = MenuState::at(Screen::MainMenu);
        s.set_server_source(ServerSource::Lan);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        // The On-line entry opens the screen, which starts a LAN query.
        click(&mut s, &mut b, &ctx, egui::pos2(200.0, 22.0 + 120.0 + 3.0 * 104.0 + 46.0));
        click(&mut s, &mut b, &ctx, egui::pos2(1130.0, 671.0));
        assert_eq!(s.screen(), Screen::Online);
        frame_at(&ctx, &mut s, &mut b, 0.1, Vec::new());
        assert_eq!(b.log.iter().filter(|l| *l == "refresh").count(), 1);
        // Sorted by name: Brain Drain LAN (open) is selected; Join joins it.
        let row = |i: usize| egui::pos2(200.0, 270.0 + i as f32 * 46.0 + 23.0);
        assert_eq!(click(&mut s, &mut b, &ctx, egui::pos2(1130.0, 671.0)), Some(MenuAction::Join { address: "IP:192.168.1.20:28000".into(), password: String::new(), team: Team::Auto }));
        // A password server: the prompt, then Join with what was typed.
        click(&mut s, &mut b, &ctx, row(1));
        assert_eq!(s.join_sel.as_deref(), Some("IP:192.168.1.31:28000"));
        assert_eq!(click(&mut s, &mut b, &ctx, egui::pos2(1130.0, 671.0)), None);
        assert!(s.password.is_some());
        assert_eq!(type_text(&ctx, &mut s, &mut b, "abc", true), Some(MenuAction::Join { address: "IP:192.168.1.31:28000".into(), password: "abc".into(), team: Team::Auto }));
        assert!(s.password.is_none());
        // The incompatible server cannot be joined.
        click(&mut s, &mut b, &ctx, row(2));
        assert_eq!(click(&mut s, &mut b, &ctx, egui::pos2(1130.0, 671.0)), None);
        // Sorting: Ping header twice -> descending, the slowest first.
        click(&mut s, &mut b, &ctx, egui::pos2(730.0, 250.0));
        click(&mut s, &mut b, &ctx, egui::pos2(730.0, 250.0));
        assert_eq!(s.sort, (4, false));
        // Join by address: click the field, type, Enter.
        click(&mut s, &mut b, &ctx, egui::pos2(280.0, 590.0));
        assert_eq!(s.field, Some(Field::Address));
        assert_eq!(type_text(&ctx, &mut s, &mut b, "10.1.2.3:28001", true), Some(MenuAction::Join { address: "10.1.2.3:28001".into(), password: String::new(), team: Team::Auto }));
        // Escape in a field only leaves the field.
        click(&mut s, &mut b, &ctx, egui::pos2(280.0, 590.0));
        frame_at(&ctx, &mut s, &mut b, 2.0, vec![key(egui::Key::Escape)]);
        assert_eq!((s.screen(), s.field), (Screen::Online, None));
        // A password request from the game (CHR_PASSWORD) opens the prompt.
        b.password_request = Some("IP:10.1.2.3:28001".into());
        frame_at(&ctx, &mut s, &mut b, 2.1, Vec::new());
        assert_eq!(s.password.as_ref().map(|p| p.0.as_str()), Some("IP:10.1.2.3:28001"));
        frame_at(&ctx, &mut s, &mut b, 2.2, vec![key(egui::Key::Escape)]);
        assert!(s.password.is_none());
        // Host: the sub-tab, a password, then Host.
        click(&mut s, &mut b, &ctx, egui::pos2(190.0, 152.0));
        assert_eq!(s.online_tab, OnlineTab::Host);
        click(&mut s, &mut b, &ctx, egui::pos2(165.0, 322.0));
        assert_eq!(s.field, Some(Field::ServerPassword));
        type_text(&ctx, &mut s, &mut b, "pw", false);
        match click(&mut s, &mut b, &ctx, egui::pos2(1130.0, 671.0)) {
            Some(MenuAction::Host(h)) => {
                assert_eq!((h.name.as_str(), h.password.as_str(), h.max_players, h.port), ("Alice's game", "pw", 16, 28000));
                assert_eq!(h.game.mission, "missions/tt1_1.ron");
            }
            other => panic!("{other:?}"),
        }
    }

    /// The server details' player list, the team choice of a team game,
    /// the Internet source with its master address, the host's internet options.
    #[test]
    fn online_players_teams_and_internet() {
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        let mut s = MenuState::at(Screen::MainMenu);
        assert_eq!(s.server_source, ServerSource::Internet, "Internet by default");
        s.set_server_source(ServerSource::Lan);
        s.open(Screen::Online);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        assert_eq!(b.log, vec!["source Lan", "refresh"]);
        let row = |i: usize| egui::pos2(200.0, 270.0 + i as f32 * 46.0 + 23.0);
        let join = egui::pos2(1130.0, 671.0);
        // The team server: its players (from the info reply) and the team choice.
        click(&mut s, &mut b, &ctx, row(1));
        assert!(b.servers[1].team_game() && !b.servers[0].team_game());
        assert_eq!(b.servers[1].player_list[1], ServerPlayer { name: "Krevitz".into(), team: Team::Blue, score: 11 });
        click(&mut s, &mut b, &ctx, egui::pos2(1160.0, 200.0));
        assert!(s.show_players);
        click(&mut s, &mut b, &ctx, egui::pos2(1032.0, 200.0));
        assert!(!s.show_players);
        click(&mut s, &mut b, &ctx, egui::pos2(655.0, 671.0));
        assert_eq!(s.join_team, Team::Blue);
        assert_eq!(click(&mut s, &mut b, &ctx, join), None, "password prompt");
        assert_eq!(type_text(&ctx, &mut s, &mut b, "pw", true), Some(MenuAction::Join { address: "IP:192.168.1.31:28000".into(), password: "pw".into(), team: Team::Blue }));
        // A free-for-all server: no team asked for.
        click(&mut s, &mut b, &ctx, row(0));
        assert_eq!(click(&mut s, &mut b, &ctx, join), Some(MenuAction::Join { address: "IP:192.168.1.20:28000".into(), password: String::new(), team: Team::Auto }));
        // Internet without a master server: a hint, no query, Refresh disabled.
        b.log.clear();
        click(&mut s, &mut b, &ctx, egui::pos2(116.0, 200.0));
        assert_eq!((s.server_source, b.source), (ServerSource::Internet, ServerSource::Internet));
        click(&mut s, &mut b, &ctx, egui::pos2(900.0, 671.0));
        assert_eq!(b.log, vec!["source Internet"]);
        // The master servers dialog (the plate in the tab row, top right):
        // one row a master, "+ Add", × removes, Save checks, Escape cancels.
        click(&mut s, &mut b, &ctx, egui::pos2(1000.0, 148.0));
        assert_eq!(s.masters.as_ref().map(|m| m.rows.clone()), Some(vec![String::new()]));
        type_text(&ctx, &mut s, &mut b, "master.example.org:28100", false);
        click(&mut s, &mut b, &ctx, egui::pos2(388.0, 261.0));
        assert_eq!(s.masters.as_ref().map(|m| (m.rows.len(), m.focus)), Some((2, 1)));
        type_text(&ctx, &mut s, &mut b, "bad host", true);
        assert!(s.masters.as_ref().is_some_and(|m| m.invalid), "not saved");
        assert_eq!(b.master, "");
        click(&mut s, &mut b, &ctx, egui::pos2(948.0, 263.0));
        assert_eq!(s.masters.as_ref().map(|m| m.rows.clone()), Some(vec!["master.example.org:28100".to_string()]));
        click(&mut s, &mut b, &ctx, egui::pos2(902.0, 607.0));
        assert!(s.masters.is_none());
        assert_eq!(b.master, "master.example.org:28100");
        click(&mut s, &mut b, &ctx, egui::pos2(1000.0, 148.0));
        click(&mut s, &mut b, &ctx, egui::pos2(388.0, 261.0));
        type_text(&ctx, &mut s, &mut b, "tt.example.net", false);
        frame_at(&ctx, &mut s, &mut b, 2.0, vec![key(egui::Key::Escape)]);
        assert!(s.masters.is_none());
        assert_eq!(b.master, "master.example.org:28100", "cancelled");
        assert_eq!(b.log[1..], ["master master.example.org:28100", "refresh internet master.example.org:28100"]);
        b.internet_servers = vec![b.servers[0].clone()];
        frame_at(&ctx, &mut s, &mut b, 3.0, Vec::new());
        assert_eq!(click(&mut s, &mut b, &ctx, join), Some(MenuAction::Join { address: "IP:192.168.1.20:28000".into(), password: String::new(), team: Team::Auto }));
        // Back to LAN.
        click(&mut s, &mut b, &ctx, egui::pos2(236.0, 200.0));
        assert_eq!(b.source, ServerSource::Lan);
        // Host: the router port mapping and the master listing, at the
        // bottom of the server plate.
        click(&mut s, &mut b, &ctx, egui::pos2(190.0, 152.0));
        assert_eq!(s.online_tab, OnlineTab::Host);
        click(&mut s, &mut b, &ctx, egui::pos2(95.0, 544.0));
        click(&mut s, &mut b, &ctx, egui::pos2(95.0, 576.0));
        match click(&mut s, &mut b, &ctx, join) {
            Some(MenuAction::Host(h)) => assert_eq!((h.port_mapping, h.public), (false, false)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn custom_game_replace_bots() {
        // A custom game is local: no Replace bots toggle there.
        let ctx = egui::Context::default();
        let mut b = MockBackend::new();
        let mut s = MenuState::at(Screen::CustomGame);
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        let before = s.custom.as_ref().unwrap().replace_bots;
        click(&mut s, &mut b, &ctx, egui::pos2(1200.0, 22.0 + 115.0));
        assert_eq!(s.custom.as_ref().unwrap().replace_bots, before);
        // Hosting: the tick box by the Bots tab.
        let mut s = MenuState::at(Screen::Online);
        s.online_tab = OnlineTab::Host;
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        frame_at(&ctx, &mut s, &mut b, 0.0, Vec::new());
        let before = s.host.as_ref().unwrap().game.replace_bots;
        click(&mut s, &mut b, &ctx, egui::pos2(700.0, 200.0));
        assert!(s.host_bots, "the Bots tab");
        click(&mut s, &mut b, &ctx, egui::pos2(960.0, 201.0));
        assert_eq!(s.host.as_ref().unwrap().game.replace_bots, !before);
    }

}
