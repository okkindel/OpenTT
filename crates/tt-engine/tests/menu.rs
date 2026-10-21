//! The new menus (tt-menu) driven in-process over the game (asset
//! directory): egui frames with keyboard events, no window, no GPU, no
//! audio output.

use std::path::PathBuf;
use tt_engine::menu::{self, Mode};
use tt_engine::{Config, Engine};
use tt_menu::{BotSetup, BotSkill, CustomGame, MenuAction, Screen, Team};

mod common;
use common::*;

/// Drives the engine like the frontend does: sim ticks plus one menu frame
/// per 32 ms.
struct Driver {
    e: Engine,
    ctx: egui::Context,
    time: f64,
}

impl Driver {
    fn new(e: Engine) -> Driver {
        let ctx = egui::Context::default();
        tt_menu::theme::install_fonts(&ctx);
        Driver { e, ctx, time: 0.0 }
    }

    /// One menu frame with `events`.
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.032;
        let mods = events.iter().find_map(|e| match e {
            egui::Event::Key { modifiers, .. } => Some(*modifiers),
            _ => None,
        });
        let mut raw = menu::raw_input((1280, 720), events, self.time);
        raw.modifiers = mods.unwrap_or_default();
        let _ = menu::run_frame(&mut self.e, &self.ctx, raw);
    }

    /// `ms` of game time, one sim tick and one (input-less) menu frame per tick.
    fn run(&mut self, ms: u32) {
        for _ in 0..ms / tt_engine::TICK_MS {
            self.e.tick();
            self.frame(Vec::new());
        }
    }

    /// Runs until `f` holds (at most `ms`); returns whether it did.
    fn until(&mut self, ms: u32, f: impl Fn(&Engine) -> bool) -> bool {
        for _ in 0..ms / tt_engine::TICK_MS {
            if f(&self.e) {
                return true;
            }
            self.e.tick();
            self.frame(Vec::new());
        }
        f(&self.e)
    }

    fn keys(&mut self, keys: &[egui::Key]) {
        for k in keys {
            self.frame(vec![key(*k, false)]);
            self.frame(Vec::new());
        }
    }

    fn screen(&self) -> Screen {
        self.e.menu.as_ref().unwrap().state.screen()
    }

    fn mode(&self) -> Mode {
        self.e.menu.as_ref().unwrap().data.mode
    }

    fn content(&self) -> String {
        content(&self.e)
    }
}

fn key(k: egui::Key, ctrl: bool) -> egui::Event {
    let modifiers = egui::Modifiers { ctrl, command: ctrl, ..Default::default() };
    egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers }
}

fn log(d: &Driver) -> String {
    d.e.menu.as_ref().unwrap().data.log.join(" | ")
}

#[test]
fn player_select_quick_play_pause_leave() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut d = Driver::new(engine_wall_clock(&root, "flow", &[]));
        assert!(d.e.boot());
        assert!(d.e.menu.is_some());
        d.run(500);
        assert_eq!(d.content(), "loadPlayerGui");
        assert_eq!(d.screen(), Screen::PlayerSelect);
        assert_eq!(d.mode(), Mode::Shell);
        assert!(!d.e.host.shell.prefs.is_parental());

        // No players: the name dialog is open; type the name, Enter.
        d.frame(Vec::new());
        d.frame(vec![egui::Event::Text("Tester".into())]);
        d.frame(vec![key(egui::Key::Enter, false)]);
        d.frame(Vec::new());
        println!("after new player: {} / {:?}", log(&d), d.screen());
        assert_eq!(d.e.var("players.count"), "1");
        assert_eq!(d.e.var("players.NAME0"), "Tester");
        assert_eq!(d.e.var("player.name"), "Tester");
        assert!(d.e.host.fs.data_dir().join("players.ron").exists(), "players.ron saved");
        // The script state follows: the tank selection is the shell content.
        assert_eq!(d.content(), "tankSelectionGui");
        // The tutorial prompt; Down + Enter skips it.
        assert_eq!(d.screen(), Screen::TutorialPrompt);
        d.keys(&[egui::Key::ArrowDown, egui::Key::Enter]);
        assert_eq!(d.screen(), Screen::MainMenu);

        // Quick Play (second item).
        d.keys(&[egui::Key::ArrowDown, egui::Key::Enter]);
        assert!(log(&d).contains("QuickPlay"), "{}", log(&d));
        assert_eq!(d.screen(), Screen::Loading);
        assert!(d.until(20_000, |e| content(e) == "loadingGui"), "loading: {}", content(&d.e));
        assert_eq!(d.screen(), Screen::Loading);
        assert!(d.until(60_000, |e| content(e) == "PlayGui"), "game: {}\n{}", content(&d.e), d.e.report(20));
        d.run(200);
        assert_eq!(d.mode(), Mode::InGame);
        assert!(!d.e.menu.as_ref().unwrap().visible(&d.e.host), "menu hidden in game");
        assert!(d.e.control_object().is_some());
        d.run(2000);

        d.e.key("escape", true, None);
        d.e.key("escape", false, None);
        d.run(100);
        assert!(d.e.menu.as_ref().unwrap().pause.is_some(), "pause menu: {}\n{} req={}", log(&d), d.e.host.canvas.describe(), d.e.host.shell.pause_request);
        assert!(d.e.menu.as_ref().unwrap().visible(&d.e.host));
        assert!(d.e.time_scale() < 0.001, "paused: sim.time_scale = {}", d.e.var("sim.time_scale"));
        // The game stays the canvas content behind the menu.
        assert_eq!(d.content(), "PlayGui");

        // Resume (Enter on the first item), then pause again and leave.
        d.keys(&[egui::Key::Enter]);
        assert!(d.e.menu.as_ref().unwrap().pause.is_none());
        assert!((d.e.time_scale() - 1.0).abs() < 1e-6);
        d.run(1000);
        d.e.key("escape", true, None);
        d.e.key("escape", false, None);
        d.run(100);
        assert!(d.e.menu.as_ref().unwrap().pause.is_some(), "pause again: {}", log(&d));
        d.keys(&[egui::Key::ArrowDown, egui::Key::ArrowDown, egui::Key::ArrowDown, egui::Key::Enter]);
        assert!(log(&d).contains("LeaveGame"), "{}", log(&d));
        assert!(d.until(5_000, |e| content(e) == "tankSelectionGui"), "back to the shell: {}", content(&d.e));
        d.run(500);
        assert_eq!(d.mode(), Mode::Shell);
        assert_eq!(d.screen(), Screen::MainMenu);
        assert!(d.e.control_object().is_none());
        assert!((d.e.time_scale() - 1.0).abs() < 1e-6);
        println!("menu log: {}", log(&d));

        // Prefs and players go to the data dir on exit.
        d.e.shutdown();
        assert!(d.e.host.fs.data_dir().join("settings.ron").exists());
        let _ = std::fs::remove_dir_all(d.e.host.fs.data_dir());
    });
}

#[test]
fn backend_reads_the_game() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        use tt_menu::MenuBackend;
        let mut e = engine_wall_clock(&root, "backend", &[]);
        assert!(e.boot());
        let Engine { host, menu, .. } = &mut e;
        let m = menu.as_mut().unwrap();
        let mut b = menu::Backend::new(host, &mut m.data);
        assert!(b.players().is_empty());
        b.create_player("Alice").unwrap();
        b.create_player("Bob").unwrap();
        assert!(b.create_player("alice").is_err(), "duplicate");
        let p = b.players();
        assert_eq!(p.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Alice", "Bob"]);
        assert_eq!(b.current_player().as_deref(), Some("Bob"));
        b.select_player("Alice").unwrap();
        assert_eq!(b.current_player().as_deref(), Some("Alice"));
        b.delete_player("Bob").unwrap();
        assert_eq!(b.players().len(), 1);
        assert!(b.delete_player("Alice").is_err(), "the last player stays");

        let tanks = b.tanks();
        assert_eq!(tanks.len(), 4);
        assert_eq!(tanks[0].name, "Light");
        // Baby Boss is unlocked for everyone in the new menu.
        assert!(!tanks[3].locked && !tanks[1].locked);
        // Armor bar = damage_amount / armor_scale: Baby Boss (10/1.8) < Light < Medium < Heavy.
        assert!(tanks[3].armor < tanks[0].armor && tanks[0].armor < tanks[1].armor && tanks[1].armor < tanks[2].armor);
        b.select_tank(3);
        assert_eq!(b.selected_tank(), 3);
        b.select_tank(2);
        assert_eq!(b.selected_tank(), 2);
        drop(b);
        let Engine { host, menu, .. } = &mut e;
        tt_engine::shell::set_pref(host, "player.unlock_baby_boss", "0");
        let m = menu.as_mut().unwrap();
        let mut b = menu::Backend::new(host, &mut m.data);
        assert!(b.tanks()[3].locked);
        b.select_tank(3);
        assert_eq!(b.selected_tank(), 2, "Baby Boss is locked");

        let s = b.solo_status();
        println!("solo: {s:?}");
        assert_eq!(s.level_number, 1);
        assert!(!s.mission.is_empty() && !s.title.is_empty() && !s.briefing.is_empty());
        assert_eq!(s.lives, 3);
        assert!(!s.in_run);
        let hs = b.high_scores();
        println!("high scores: {hs:?}");
        assert_eq!(hs.len(), 10);
        assert!(hs[0].score >= hs[9].score && hs[0].score > 0);
        assert_eq!(b.last_run_rank(), None);

        assert_eq!(b.missions().len(), 18);
        assert_eq!(b.missions()[15].world, "Moon");
        assert_eq!(b.missions()[17].name, "Power-up Test");
        assert_eq!(b.missions()[17].world, "Special");
        b.mission_preview("missions/tt2_3.ron", [400, 225]);
        assert_eq!(b.missions()[7].name, "Greymatter Gorge");
        assert_eq!(b.bot_names().len(), 6);
        assert!(!b.credits().is_empty());
        let keys = b.key_bindings();
        println!("bindings: {keys:?}");
        assert!(keys.iter().any(|k| k.action == "Forward" && k.keys.contains(&"W".to_string())));
        assert!(keys.iter().any(|k| k.action == "Fire" && k.keys.contains(&"Mouse 1".to_string())));
        assert!(keys.iter().any(|k| k.action == "Team chat" && k.keys == ["Y"]));

        let mut s = b.settings();
        s.music_volume = 0.25;
        s.sound_volume = 0.5;
        s.resolution = (1024, 768);
        s.window_mode = tt_menu::WindowMode::Windowed;
        b.apply_settings(&s);
        let s2 = b.settings();
        assert_eq!(s2.music_volume, 0.25);
        assert_eq!(s2.sound_volume, 0.5);
        assert_eq!(s2.resolution, (1024, 768));
        assert_eq!(s2.window_mode, tt_menu::WindowMode::Windowed);
        drop(b);
        assert_eq!(e.var("audio.music_volume"), "0.25");
        assert!((e.host.audio.channel_volume(4) - 0.25).abs() < 1e-6);
        assert_eq!((e.host.video.width, e.host.video.height), (1024, 768));
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

#[test]
fn custom_game_on_tt2_3() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut d = Driver::new(engine_wall_clock(&root, "custom", &[]));
        assert!(d.e.boot());
        d.run(300);
        {
            use tt_menu::MenuBackend;
            let Engine { host, menu, .. } = &mut d.e;
            let m = menu.as_mut().unwrap();
            menu::Backend::new(host, &mut m.data).create_player("Skirmisher").unwrap();
        }
        let bot = |name: &str, skill, tank, team| BotSetup { name: name.into(), skill, tank, team };
        let g = CustomGame {
            mission: "missions/tt2_3.ron".into(),
            mode: 1,
            bots: vec![bot("Hum", BotSkill::Bronze, 0, Team::Green), bot("Tom", BotSkill::Silver, 1, Team::Blue), bot("Mal", BotSkill::Gold, 2, Team::Blue)],
            player_tank: 2,
            player_team: Team::Green,
            minutes: 5,
            replace_bots: false,
        };
        apply(&mut d.e, MenuAction::StartCustom(g));
        assert_eq!(d.screen(), Screen::Loading);
        assert!(d.until(60_000, |e| content(e) == "PlayGui"), "game: {}\n{}", content(&d.e), d.e.report(20));
        d.run(3000);
        let e = &d.e;
        println!("{}", d.e.report(20));
        assert_eq!(e.server().setup.round_secs, 300.0);
        assert_eq!(e.server().session.mode_label, "Team Battlemode");
        assert!(!e.server().setup.single_player());
        assert!(e.server().session.mission_file.clone().to_ascii_lowercase().contains("tt2_3"), "mission {}", e.server().session.mission_file.clone());
        assert_eq!(e.var("player.tank"), "2");
        // The "Replace bots" box, off.
        assert_eq!(e.var("server.replace_bots"), "0");
        let g = d.e.host.game.as_ref().expect("simulation");
        assert_eq!(g.world.tanks.iter().filter(|t| !t.deleted && !t.is_corpse).count(), 4, "player + 3 bots");
        let player = d.e.control_object().expect("player tank");
        let db = g.entities.get(player).map(|x| x.def.clone());
        assert_eq!(db.as_deref(), Some("HeavyTank"));
        assert_eq!(d.mode(), Mode::InGame);
        let _ = std::fs::remove_dir_all(d.e.host.fs.data_dir());
    });
}

#[test]
fn solo_level_end_next_level_game_over() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut d = Driver::new(engine_wall_clock(&root, "solo", &[]));
        assert!(d.e.boot());
        d.run(300);
        // The name dialog (no players yet), skip the tutorial prompt, Solo Campaign (first item), Launch.
        d.frame(vec![egui::Event::Text("Soloist".into())]);
        d.frame(vec![key(egui::Key::Enter, false)]);
        d.frame(Vec::new());
        assert_eq!(d.screen(), Screen::TutorialPrompt);
        d.keys(&[egui::Key::ArrowDown, egui::Key::Enter]);
        assert_eq!(d.screen(), Screen::MainMenu);
        d.keys(&[egui::Key::Enter]);
        assert_eq!(d.screen(), Screen::Solo);
        d.keys(&[egui::Key::Enter]);
        assert!(log(&d).contains("StartSolo"), "{}", log(&d));
        assert!(d.until(60_000, |e| content(e) == "PlayGui"), "level 1: {}", content(&d.e));
        d.run(1000);
        assert!(d.e.server().setup.single_player());
        let first = d.e.server().session.mission_file.clone();

        // The level ends.
        end_game(&mut d.e);
        assert!(d.until(5_000, |e| content(e) == "soloPlayGui"), "between levels: {}", content(&d.e));
        d.run(500);
        assert_eq!(d.mode(), Mode::Shell);
        assert_eq!(d.screen(), Screen::Solo);
        {
            use tt_menu::MenuBackend;
            let Engine { host, menu, .. } = &mut d.e;
            let m = menu.as_mut().unwrap();
            let s = menu::Backend::new(host, &mut m.data).solo_status();
            println!("between levels: {s:?}");
            assert!(s.in_run);
            assert_eq!(s.level_number, 2);
        }
        // Next level.
        d.keys(&[egui::Key::Enter]);
        assert!(d.until(60_000, |e| content(e) == "PlayGui"), "level 2: {}", content(&d.e));
        d.run(1000);
        assert_ne!(d.e.server().session.mission_file.clone(), first);

        // Game over: no lives left when respawning.
        tt_engine::gameplay::server_host::with_local_record(&mut d.e.host, |r| r.lives = Some(0));
        spawn(&mut d.e);
        assert!(d.until(5_000, |e| content(e) == "soloGameOverGui"), "game over: {}", content(&d.e));
        d.run(200);
        assert_eq!(d.screen(), Screen::HighScores);
        assert!(d.e.host.fs.data_dir().join("hiscores.ron").exists());
        d.keys(&[egui::Key::Enter]);
        assert!(d.until(5_000, |e| content(e) == "tankSelectionGui"), "after game over: {}", content(&d.e));
        d.run(300);
        assert_eq!(d.screen(), Screen::MainMenu);
        assert!(d.e.control_object().is_none() && d.e.host.net.server_conn.is_none());
        println!("menu log: {}", log(&d));
        let _ = std::fs::remove_dir_all(d.e.host.fs.data_dir());
    });
}

#[test]
fn tutorial_message_box_and_leave() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut d = Driver::new(engine_wall_clock(&root, "tutorial", &[]));
        assert!(d.e.boot());
        d.run(300);
        d.frame(vec![egui::Event::Text("Rookie".into())]);
        d.frame(vec![key(egui::Key::Enter, false)]);
        d.frame(Vec::new());
        assert_eq!(d.screen(), Screen::TutorialPrompt);
        // "Play tutorial" is the first choice: the 8 slides (MarqueeGui), Enter
        // steps through them and starts target practice on the last one.
        d.keys(&[egui::Key::Enter]);
        assert_eq!(d.screen(), Screen::Tutorial);
        d.keys(&[egui::Key::Enter; 7]);
        assert_eq!(d.screen(), Screen::Tutorial);
        assert!(!log(&d).contains("StartTutorial"), "{}", log(&d));
        d.keys(&[egui::Key::Enter]);
        assert!(log(&d).contains("StartTutorial"), "{}", log(&d));
        assert!(d.until(60_000, |e| content(e) == "PlayGui"), "target practice: {}", content(&d.e));
        d.run(300);
        // The scripts' welcome box (ttMessageWindow) is shown by the menu over the game.
        let msg = |d: &mut Driver| {
            use tt_menu::MenuBackend;
            let Engine { host, menu, .. } = &mut d.e;
            let m = menu.as_mut().unwrap();
            menu::Backend::new(host, &mut m.data).script_message()
        };
        let m = msg(&mut d).expect("welcome message");
        assert!(m.text.contains("target practice") && !m.yes_no, "{m:?}");
        assert!(d.e.menu.as_ref().unwrap().visible(&d.e.host));
        d.keys(&[egui::Key::Enter]);
        d.run(500);
        assert!(msg(&mut d).is_none(), "answered: {}", log(&d));
        assert!(!d.e.menu.as_ref().unwrap().visible(&d.e.host));
        // Leave: back to the player selection.
        d.e.key("escape", true, None);
        d.e.key("escape", false, None);
        d.run(100);
        assert!(d.e.menu.as_ref().unwrap().pause.is_some(), "{}", log(&d));
        d.keys(&[egui::Key::ArrowDown, egui::Key::ArrowDown, egui::Key::ArrowDown, egui::Key::Enter]);
        assert!(d.until(5_000, |e| content(e) == "loadPlayerGui"), "after the tutorial: {}", content(&d.e));
        d.run(300);
        assert_eq!(d.screen(), Screen::PlayerSelect);
        println!("menu log: {}", log(&d));
        let _ = std::fs::remove_dir_all(d.e.host.fs.data_dir());
    });
}

/// The frontend's offscreen path draws the menu with egui-wgpu, and the
/// garage's tank preview into an egui texture. PNGs go to
/// `$TT_ENGINE_MENU_PREVIEW_DIR` (default `<tmp>/tt-engine-menu-preview`).
/// Skipped without a GPU adapter.
#[test]
fn offscreen_menu_frames() {
    let root = assets();
    big_stack(move || {
        let Ok(off) = tt_engine::gfx::Offscreen::new((1280, 720)) else { return eprintln!("skipped: no GPU adapter") };
        let out = std::env::var_os("TT_ENGINE_MENU_PREVIEW_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("tt-engine-menu-preview"));
        std::fs::create_dir_all(&out).unwrap();
        let mut e = engine_wall_clock(&root, "offscreen", &[]);
        assert!(e.boot());
        {
            use tt_menu::MenuBackend;
            let Engine { host, menu, .. } = &mut e;
            let m = menu.as_mut().unwrap();
            menu::Backend::new(host, &mut m.data).create_player("Painter").unwrap();
            m.state.reset(Screen::MainMenu);
        }
        let mut fe = tt_engine::frontend::Frontend::new(&off.device, &off.queue, off.format, root.clone(), false);
        let mut frames = |e: &mut Engine, n: u32| {
            for _ in 0..n {
                fe.step(e, &off.device, &off.queue, 1000.0 / 60.0);
                fe.draw(e, &off.device, &off.queue, &off.view, off.size, 1000.0 / 60.0);
            }
            off.wait();
        };
        frames(&mut e, 10);
        off.save(&out.join("main_menu.png")).unwrap();
        let img = image::open(out.join("main_menu.png")).unwrap().to_rgba8();
        assert!(img.pixels().filter(|p| p.0[0] > 40 || p.0[1] > 40 || p.0[2] > 40).count() > 10_000, "the menu is drawn");
        e.menu.as_mut().unwrap().state.open(Screen::Garage);
        frames(&mut e, 10);
        off.save(&out.join("garage.png")).unwrap();
        assert!(e.menu.as_ref().unwrap().data.preview_texture.is_some(), "tank preview rendered");
        println!("previews in {}", out.display());
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}

/// Mouse clicks through the window's input path ([`menu::PointerInput`] +
/// [`menu::window_raw_input`], window pixels in, menu points out) at two
/// window sizes: clicking the main menu's "Quick Play" entry starts it.
#[test]
fn mouse_click_on_main_menu_item() {
    let root = assets();
    big_stack(move || {
        for size in [(1280u32, 720u32), (1920, 1080), (1000, 800)] {
            let mut d = Driver::new(engine_wall_clock(&root, &format!("click{}", size.0), &[]));
            assert!(d.e.boot());
            {
                use tt_menu::MenuBackend;
                let Engine { host, menu, .. } = &mut d.e;
                let m = menu.as_mut().unwrap();
                menu::Backend::new(host, &mut m.data).create_player("Clicker").unwrap();
                m.state.reset(Screen::MainMenu);
            }
            let mut pointer = menu::PointerInput::default();
            let frame = |d: &mut Driver, pointer: &mut menu::PointerInput| {
                d.time += 0.016;
                let ppp = tt_menu::theme::pixels_per_point([size.0 as f32, size.1 as f32]);
                d.ctx.set_pixels_per_point(ppp);
                let raw = menu::window_raw_input(None, pointer, size, d.time);
                let _ = menu::run_frame(&mut d.e, &d.ctx, raw);
            };
            for _ in 0..4 {
                frame(&mut d, &mut pointer);
            }
            // Where the menu laid out "Quick Play" (item 1), in points.
            let id = egui::Id::new(("tt-menu", ("mode", 1usize)));
            let rect = d.ctx.read_response(id).map(|r| r.rect).expect("main menu item 1 laid out");
            let ppp = d.ctx.pixels_per_point();
            let px = (rect.center().x * ppp) as f64;
            let py = (rect.center().y * ppp) as f64;
            pointer.moved(px, py, size);
            frame(&mut d, &mut pointer);
            pointer.button(0, true);
            frame(&mut d, &mut pointer);
            pointer.button(0, false);
            frame(&mut d, &mut pointer);
            frame(&mut d, &mut pointer);
            assert!(log(&d).contains("QuickPlay"), "{size:?}: click at {px},{py} px ({rect:?} pt, ppp {ppp}): {}", log(&d));
            assert_eq!(d.screen(), Screen::Loading);
            let _ = std::fs::remove_dir_all(d.e.host.fs.data_dir());
        }
    });
}

/// The shell state of the engine (tt-shell).
/// The local player's respawn request, run by the server now.
fn spawn(e: &mut Engine) {
    let c = tt_engine::gameplay::server_host::local_client(&e.host).unwrap();
    tt_engine::gameplay::server_host::client_cmd(&mut e.host, c, &tt_gameplay::ClientCmd::SpawnPlayer);
}

/// The round ends now (`endGame()`).
fn end_game(e: &mut Engine) {
    tt_engine::gameplay::server_host::with_server(&mut e.host, |s, b| s.end_round(b));
}

fn shell(e: &mut Engine) -> &mut tt_engine::shell::Shell {
    &mut e.host.shell
}

/// The shell is native: its screens are the canvas's own (no script GUI).
fn assert_no_shell_scripts(e: &Engine) {
    assert!(e.host.shell.screens);
}

#[test]
fn players_saved_by_tt_shell_without_shell_scripts() {
    let root = assets();
    big_stack(move || {
        use tt_menu::MenuBackend;
        let mut d = Driver::new(engine_wall_clock(&root, "shellplayers", &[]));
        assert!(d.e.boot());
        d.run(300);
        assert_no_shell_scripts(&d.e);
        assert_eq!(d.content(), "loadPlayerGui");
        backend_do(&mut d.e, |b| {
            b.create_player("Tester").unwrap();
            b.create_player("Second").unwrap();
            b.select_player("Tester").unwrap();
        });
        let data = d.e.host.fs.data_dir().to_path_buf();
        let saved = tt_shell::save::read_dir(&data).players;
        assert_eq!(saved.names(), ["Tester", "Second"]);
        assert_eq!(saved.last_selection(), 0);
        // The interpreter mirror (server scripts read it) and the name pref.
        assert_eq!(d.e.var("players.count"), "2");
        assert_eq!(d.e.var("player.name"), "Tester");
        assert_eq!(d.content(), "tankSelectionGui");
        backend_do(&mut d.e, |b| b.delete_player("Second").unwrap());
        assert_eq!(
            tt_shell::save::read_dir(&data).players.names(),
            ["Tester"]
        );
        // Prefs are written by tt-shell on exit (onExit's exports).
        d.e.shutdown();
        let prefs = tt_shell::Prefs::with_settings(&tt_shell::save::read_dir(&data).settings.expect("settings.ron"));
        assert_eq!(prefs.player_name(), "Tester");
        assert!(prefs.unlock_baby_boss());
        assert!(!data.join(tt_shell::save::LEGACY_CLIENT_PREFS).exists(), "no old-format save is written");
        // A second start reads them back.
        let cfg = Config {
            assets: root.clone(),
            data: data.clone(),
            args: Vec::new(),
            audio_output: false,
            echo_log: false,
            windowed: true,
        };
        let mut e2 = Engine::new(&cfg).unwrap();
        assert!(e2.boot());
        assert_eq!(shell(&mut e2).players.names(), ["Tester"]);
        assert_eq!(e2.var("player.name"), "Tester");
        let _ = std::fs::remove_dir_all(&data);
    });
}

#[test]
fn solo_campaign_tally_next_level_and_hiscore() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        use tt_menu::MenuBackend;
        let mut d = Driver::new(engine_wall_clock(&root, "shellsolo", &[]));
        assert!(d.e.boot());
        // A player before the first frame (no new-player dialog).
        backend_do(&mut d.e, |b| {
            b.create_player("Soloist").unwrap();
            // No tutorial prompt in this test.
            assert!(b.wants_tutorial_prompt());
        });
        d.e.menu.as_mut().unwrap().state.reset(Screen::MainMenu);
        d.run(300);
        apply(&mut d.e, MenuAction::StartSolo);
        assert!(
            d.until(60_000, |e| content(e) == "PlayGui"),
            "level 1: {}",
            content(&d.e)
        );
        d.run(500);
        assert_no_shell_scripts(&d.e);
        // In play the game takes the mouse: cursor off, lockMouse(1), no menu.
        {
            let menu_vis = d.e.menu.as_ref().unwrap().visible(&d.e.host);
            assert!(!d.e.host.canvas.cursor_on && d.e.host.mouse_locked && !menu_vis, "mouse not captured in play");
        }
        assert!(
            d.e.server().session.mission_file.clone().contains("tt1_1"),
            "{}",
            d.e.server().session.mission_file.clone()
        );
        {
            let run = shell(&mut d.e).run.as_ref().expect("solo run");
            assert_eq!(
                (run.mission_seq, run.mission_seq_total, run.lives),
                (1, 1, 3)
            );
        }
        assert_eq!(d.e.server().campaign.mission_seq_total, Some(1));
        assert_eq!(d.e.server().campaign.kind(), Some(tt_gameplay::server::LevelKind::Battle));

        // The level ends with kills on the connection (as the gameplay scripts count them).
        tt_engine::gameplay::server_host::with_local_record(&mut d.e.host, |r| {
            r.spscore = Some(9500.0);
            r.bronze_kills = Some(4);
            r.silver_kills = Some(2);
            r.deaths = Some(0);
        });
        end_game(&mut d.e);
        assert!(
            d.until(5_000, |e| content(e) == "soloPlayGui"),
            "between levels: {}",
            content(&d.e)
        );
        d.run(200);
        assert_eq!(d.screen(), Screen::Solo);
        let t = shell(&mut d.e).last_tally.clone().expect("tally");
        println!("tally: {t:?}");
        assert!(
            matches!(
                t.rows[0],
                tt_shell::TallyRow::Kills {
                    kind: tt_shell::solo::KillKind::Bronze,
                    count: 4,
                    ..
                }
            ),
            "{:?}",
            t.rows
        );
        assert!(t
            .rows
            .iter()
            .any(|r| matches!(r, tt_shell::TallyRow::Survival { .. })));
        assert!(
            t.score > 9500.0
                && t.score
                    == t.start_score
                        + t.rows
                            .iter()
                            .map(|r| match r {
                                tt_shell::TallyRow::Kills { pts, .. }
                                | tt_shell::TallyRow::Survival { pts }
                                | tt_shell::TallyRow::Boss { pts }
                                | tt_shell::TallyRow::Completion { pts } => *pts,
                            })
                            .sum::<f64>()
        );
        // The tally went back onto the connection: score, extra life, counters consumed.
        // Engine fields now (the gameplay state's record): read like a script does.
        let rec = |e: &mut Engine| tt_engine::gameplay::server_host::local_record(&e.host).unwrap_or_default();
        assert_eq!(rec(&mut d.e).spscore, Some(t.score));
        assert_eq!(rec(&mut d.e).lives, Some(t.lives));
        assert_eq!(t.extra_lives, (t.score / 10000.0).floor() as i32);
        assert_eq!(rec(&mut d.e).bronze_kills, Some(0));
        let s = backend_do(&mut d.e, |b| b.solo_status());
        assert!(
            s.in_run && s.level_number == 2 && s.mission == "tt1_2" && s.score == t.score as i64,
            "{s:?}"
        );
        assert!(
            s.tally
                .iter()
                .any(|(l, v)| l == "Bronze tanks" && v.starts_with("4 x")),
            "{:?}",
            s.tally
        );
        // The animated tally: the advanceScore steps end on the final score.
        let anim = s.tally_anim.clone().expect("tally steps");
        assert_eq!(anim.start_score, 9500);
        assert_eq!(anim.steps.last().map(|x| (x.score, x.delay_ms)), Some((t.score as i64, -1)));
        assert_eq!(anim.steps.iter().filter(|x| x.event == tt_menu::TallyEvent::Count(tt_menu::TallyKind::Bronze)).count(), 4);
        assert!(anim.steps.iter().any(|x| x.event == tt_menu::TallyEvent::Survival));
        assert_eq!(anim.steps.iter().filter(|x| x.event == tt_menu::TallyEvent::ExtraLifeStart).count() as i32, t.extra_lives);

        // Next level.
        apply(&mut d.e, MenuAction::StartSolo);
        assert!(
            d.until(60_000, |e| content(e) == "PlayGui"),
            "level 2: {}",
            content(&d.e)
        );
        d.run(500);
        assert!(d.e.server().session.mission_file.clone().contains("tt1_2"));
        assert_eq!(d.e.server().campaign.mission_seq_total, Some(2));
        assert_eq!(rec(&mut d.e).deaths, Some(0));

        // Game over with 400000 points: third in the default top 10.
        tt_engine::gameplay::server_host::with_local_record(&mut d.e.host, |r| {
            r.spscore = Some(400000.0);
            r.lives = Some(0);
        });
        spawn(&mut d.e);
        assert!(
            d.until(5_000, |e| content(e) == "soloGameOverGui"),
            "game over: {}",
            content(&d.e)
        );
        d.run(200);
        assert_eq!(d.screen(), Screen::HighScores);
        assert_eq!(backend_do(&mut d.e, |b| b.last_run_rank()), Some(2));
        let data = d.e.host.fs.data_dir().to_path_buf();
        let hs = tt_shell::save::read_dir(&data).hiscores;
        let e2 = hs.entry(2).expect("valid table");
        assert_eq!(
            (e2.name.as_str(), e2.score, e2.level),
            ("Soloist", 400000, 2)
        );
        assert_eq!(hs.entry(3).unwrap().name, "Shark");
        let players = tt_shell::save::read_dir(&data).players;
        assert_eq!(players.solo_hiscore(0), "400000");
        let top = backend_do(&mut d.e, |b| b.high_scores());
        assert_eq!(top[2].name, "Soloist");
        // OK: disconnect, main menu.
        d.keys(&[egui::Key::Enter]);
        assert!(
            d.until(5_000, |e| content(e) == "tankSelectionGui"),
            "after game over: {} {:?} {}",
            content(&d.e),
            d.screen(),
            log(&d)
        );
        assert!(d.e.host.net.server_conn.is_none());
        let _ = std::fs::remove_dir_all(&data);
    });
}

#[test]
fn custom_game_on_every_map_and_mp_stats() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut d = Driver::new(engine_wall_clock(&root, "shellmaps", &[]));
        assert!(d.e.boot());
        {
            use tt_menu::MenuBackend;
            backend_do(&mut d.e, |b| {
                b.create_player("Mapper").unwrap();
                b.wants_tutorial_prompt()
            });
        }
        d.e.menu.as_mut().unwrap().state.reset(Screen::MainMenu);
        let missions = tt_engine::shell::missions(&mut d.e.host).to_vec();
        assert_eq!(missions.len(), 18, "the game's maps: {missions:?}");
        for (i, m) in missions.iter().enumerate() {
            let mode = (i % 4) as u32;
            let g = CustomGame {
                mission: tt_assets::layout::mission(&m.id),
                mode,
                bots: vec![BotSetup {
                    name: "Hum".into(),
                    skill: BotSkill::Gold,
                    tank: 0,
                    team: Team::Auto,
                }],
                // Baby Boss (index 3) is available to everyone.
                player_tank: if i == 0 { 3 } else { i % 3 },
                player_team: Team::Green,
                minutes: 5,
                replace_bots: true,
            };
            apply(&mut d.e, MenuAction::StartCustom(g));
            assert!(
                d.until(60_000, |e| content(e) == "PlayGui"),
                "{}: {}",
                m.id,
                content(&d.e)
            );
            d.run(300);
            assert!(
                d.e.server().session.mission_file.clone().contains(&m.id),
                "{}: {}",
                m.id,
                d.e.server().session.mission_file.clone()
            );
            assert_eq!(d.mode(), Mode::InGame);
            let player = d.e.control_object().expect("player tank");
            if i == 0 {
                let db = d.e.host.game.as_ref().and_then(|g| g.entities.get(player)).map(|x| x.def.clone());
                assert_eq!(db.as_deref(), Some("BabyBoss"));
                // A counted free-for-all round: the stats go to players.ron.
                end_game(&mut d.e);
                assert!(
                    d.until(5_000, |e| content(e) == "EndGameGui"),
                    "round end: {}",
                    content(&d.e)
                );
                // The menu's round results stand in for EndGameGui.
                d.run(100);
                assert!(d.e.menu.as_ref().unwrap().visible(&d.e.host));
                let r = backend_do(&mut d.e, |b| tt_menu::MenuBackend::round_results(b)).expect("round results");
                assert!(r.kda && r.players.iter().any(|p| p.me && p.name == "Mapper"), "{r:?}");
                assert_eq!(r.players.len(), 2, "{r:?}");
                let saved =
                    tt_shell::save::read_dir(d.e.host.fs.data_dir()).players;
                assert_eq!(saved.combat_exp(0), "1");
                assert_eq!(d.e.var("players.count"), "1");
            }
            apply(&mut d.e, MenuAction::LeaveGame);
            assert!(
                d.until(5_000, |e| content(e) == "tankSelectionGui"),
                "{} leave: {}",
                m.id,
                content(&d.e)
            );
            d.run(100);
            assert_eq!(d.screen(), Screen::MainMenu);
        }
        assert_no_shell_scripts(&d.e);
        // Quick play picks a random multiplayer map (any world).
        apply(&mut d.e, MenuAction::QuickPlay);
        assert!(
            d.until(60_000, |e| content(e) == "PlayGui"),
            "quick play: {}",
            content(&d.e)
        );
        assert!(
            ["tt1_", "tt2_", "tt3_"].iter().any(|w| d.e.server().session.mission_file.clone().contains(w)),
            "{}",
            d.e.server().session.mission_file.clone()
        );
        let _ = std::fs::remove_dir_all(d.e.host.fs.data_dir());
    });
}
