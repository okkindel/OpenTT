//! Offscreen previews of the menu screens (egui + egui-wgpu on a headless
//! GPU). PNGs go to `$TT_MENU_PREVIEW_DIR` (default `<tmp>/tt-menu-preview`).
//! Artwork is loaded from the asset directory. Skipped without a GPU adapter.

use std::path::PathBuf;

use tt_menu::{theme, MenuState, MockBackend, OnlineTab, Screen, SettingsTab, SetupMode, SetupProgress, SetupReport, SetupState};

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None)).ok()?;
    Some(Gpu { device, queue })
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Run a few egui passes of `state` and render the last one to an image.
/// The first two passes are at time 0 (fonts, animation start), the others at `t` seconds.
fn render(gpu: &Gpu, renderer: &mut egui_wgpu::Renderer, state: &mut MenuState, backend: &mut MockBackend, size: [u32; 2], t: f64) -> image::RgbaImage {
    let ppp = theme::pixels_per_point([size[0] as f32, size[1] as f32]);
    render_with(gpu, renderer, size, ppp, t, |ctx| {
        let _ = state.ui(ctx, backend);
    })
}

/// Run a few egui passes of `ui` at `ppp` points per pixel and render the last one.
fn render_with(gpu: &Gpu, renderer: &mut egui_wgpu::Renderer, size: [u32; 2], ppp: f32, t: f64, mut ui: impl FnMut(&egui::Context)) -> image::RgbaImage {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut output = None;
    for time in [0.0, 0.0, t, t, t] {
        let input = egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32 / ppp, size[1] as f32 / ppp))),
            max_texture_side: Some(gpu.device.limits().max_texture_dimension_2d as usize),
            ..Default::default()
        };
        ctx.set_pixels_per_point(ppp);
        let out = ctx.run(input, &mut ui);
        for (id, delta) in &out.textures_delta.set {
            renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        output = Some(out);
    }
    let out = output.unwrap();
    let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point: out.pixels_per_point };
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("menu-preview"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    let extra = renderer.update_buffers(&gpu.device, &gpu.queue, &mut enc, &jobs, &screen);
    {
        let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
    }
    let (w, h) = (size[0], size[1]);
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit(extra.into_iter().chain([enc.finish()]));
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device.poll(wgpu::Maintain::Wait);
    let data = buf.slice(..).get_mapped_range();
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let s = (y * row) as usize;
        px.extend_from_slice(&data[s..s + (w * 4) as usize]);
    }
    for id in &out.textures_delta.free {
        renderer.free_texture(id);
    }
    image::RgbaImage::from_raw(w, h, px).unwrap()
}

#[test]
fn menu_previews() {
    let Some(gpu) = gpu() else {
        eprintln!("no GPU adapter; skipping previews");
        return;
    };
    let out = std::env::var_os("TT_MENU_PREVIEW_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("tt-menu-preview"));
    std::fs::create_dir_all(&out).unwrap();
    let root = tt_assets::layout::default_dir();
    let mut renderer = egui_wgpu::Renderer::new(&gpu.device, FORMAT, None, 1, false);
    let slide = |n: u32| {
        let mut s = MenuState::at(Screen::Tutorial);
        s.show_tutorial_slide(n);
        s
    };
    let mut shots: Vec<(&str, MenuState, [u32; 2])> = vec![
        ("01_player_select", MenuState::new(), [1280, 720]),
        ("02_new_player", { let mut s = MenuState::new(); s.show_new_player_dialog("Neo"); s }, [1280, 720]),
        ("03_main_menu", MenuState::at(Screen::MainMenu), [1280, 720]),
        ("04_main_menu_1080p", MenuState::at(Screen::MainMenu), [1920, 1080]),
        ("05_settings_display", MenuState::at(Screen::Settings), [1280, 720]),
        ("05b_settings_display_uncapped", MenuState::at(Screen::Settings), [1280, 720]),
        ("05c_settings_display_uncapped_1080p", MenuState::at(Screen::Settings), [1920, 1080]),
        ("03b_main_menu_long_name", MenuState::at(Screen::MainMenu), [1280, 720]),
        ("06_settings_audio", { let mut s = MenuState::at(Screen::Settings); s.set_settings_tab(SettingsTab::Audio); s }, [1280, 720]),
        ("07_settings_controls", { let mut s = MenuState::at(Screen::Settings); s.set_settings_tab(SettingsTab::Controls); s }, [1280, 720]),
        ("07b_settings_graphics", { let mut s = MenuState::at(Screen::Settings); s.set_settings_tab(SettingsTab::Graphics); s }, [1280, 720]),
        ("07c_settings_game_data", { let mut s = MenuState::at(Screen::Settings); s.set_settings_tab(SettingsTab::GameData); s }, [1280, 720]),
        ("07e_settings_game_data_1080p", { let mut s = MenuState::at(Screen::Settings); s.set_settings_tab(SettingsTab::GameData); s }, [1920, 1080]),
        ("07d_settings_mods", { let mut s = MenuState::at(Screen::Settings); s.set_settings_tab(SettingsTab::Mods); s }, [1280, 720]),
        ("08_custom_game", MenuState::at(Screen::CustomGame), [1280, 720]),
        ("09_solo", MenuState::at(Screen::Solo), [1280, 720]),
        ("10_garage", MenuState::at(Screen::Garage), [1280, 720]),
        ("11_pause", MenuState::pause(), [1280, 720]),
        ("12_quit", MenuState::at(Screen::QuitConfirm), [1280, 720]),
        ("13_high_scores", MenuState::at(Screen::HighScores), [1280, 720]),
        ("14_loading", MenuState::at(Screen::Loading), [1280, 720]),
        ("15_script_message", MenuState::at(Screen::MainMenu), [1280, 720]),
        ("16_tutorial_prompt", MenuState::at(Screen::TutorialPrompt), [1280, 720]),
        ("17_custom_game_1600x900", MenuState::at(Screen::CustomGame), [1600, 900]),
        ("18_help", MenuState::at(Screen::Help), [1280, 720]),
        ("18b_credits", MenuState::at(Screen::Credits), [1280, 720]),
        ("19_custom_game_scroll", MenuState::at(Screen::CustomGame), [1280, 600]),
        ("20_solo_tally_counting", MenuState::at(Screen::Solo), [1280, 720]),
        ("21_solo_tally_extra_life", MenuState::at(Screen::Solo), [1280, 720]),
        ("22_solo_many_lives", MenuState::at(Screen::Solo), [1280, 720]),
        ("23_solo_brain_level", MenuState::at(Screen::Solo), [1280, 720]),
        ("24_tutorial_1", slide(1), [1280, 720]),
        ("25_tutorial_2", slide(2), [1280, 720]),
        ("26_tutorial_3", slide(3), [1280, 720]),
        ("27_tutorial_4", slide(4), [1280, 720]),
        ("28_tutorial_5", slide(5), [1280, 720]),
        ("29_tutorial_6", slide(6), [1280, 720]),
        ("30_tutorial_7", slide(7), [1280, 720]),
        ("31_tutorial_8", slide(8), [1280, 720]),
        ("32_tutorial_1080p", slide(3), [1920, 1080]),
        ("33_solo_level_select", MenuState::at(Screen::Solo), [1280, 720]),
        ("34_round_results_teams", MenuState::at(Screen::Results), [1280, 720]),
        ("35_round_results_ffa", MenuState::at(Screen::Results), [1280, 720]),
        ("36_round_results_1080p", MenuState::at(Screen::Results), [1920, 1080]),
        ("40_online_join", { let mut s = MenuState::at(Screen::Online); s.join_sel = Some("IP:192.168.1.31:28000".into()); s }, [1280, 720]),
        ("41_online_join_empty", MenuState::at(Screen::Online), [1280, 720]),
        ("42_online_password", { let mut s = MenuState::at(Screen::Online); s.password = Some(("IP:192.168.1.31:28000".into(), String::new())); s }, [1280, 720]),
        ("43_online_host", { let mut s = MenuState::at(Screen::Online); s.online_tab = OnlineTab::Host; s }, [1280, 720]),
        ("44_online_connecting", MenuState::at(Screen::Loading), [1280, 720]),
        ("45_online_error", MenuState::at(Screen::Online), [1280, 720]),
        ("46_online_join_1080p", MenuState::at(Screen::Online), [1920, 1080]),
        ("47_main_menu_online", { let mut s = MenuState::at(Screen::MainMenu); s.focus = 3; s }, [1280, 720]),
        ("48_online_players", { let mut s = MenuState::at(Screen::Online); s.join_sel = Some("IP:192.168.1.31:28000".into()); s.show_players = true; s }, [1280, 720]),
        ("49_online_internet_no_master", { let mut s = MenuState::at(Screen::Online); s.set_server_source(tt_menu::ServerSource::Internet); s }, [1280, 720]),
        ("50_online_internet", { let mut s = MenuState::at(Screen::Online); s.set_server_source(tt_menu::ServerSource::Internet); s }, [1280, 720]),
        ("51_online_masters", { let mut s = MenuState::at(Screen::Online); s.set_server_source(tt_menu::ServerSource::Internet); s.masters = Some(tt_menu::MastersEdit { rows: vec!["master.example.org:28100".into(), "tt.example.net".into(), String::new()], focus: 2, invalid: false }); s }, [1280, 720]),
    ];
    // `TT_MENU_PREVIEW_ONLY=<text>`: only the shots whose name contains it.
    if let Some(only) = std::env::var("TT_MENU_PREVIEW_ONLY").ok().filter(|s| !s.is_empty()) {
        shots.retain(|(n, _, _)| n.contains(&only));
    }
    for (name, state, size) in &mut shots {
        let mut backend = MockBackend::new();
        backend.game_root = Some(root.clone());
        if name.contains("uncapped") {
            // Unlimited with VSync on: the note next to the FPS limit.
            backend.settings.max_fps = -1;
        }
        if name.contains("long_name") {
            backend.current = Some("Krevitzinator the Unrelenting Destroyer".into());
            backend.update = Some("v0.2.0".into());
        }
        if *name == "14_loading" {
            backend.loading = Some(tt_menu::Loading { map: "Greymatter Gorge".into(), phase: "Loading objects".into(), progress: 0.62 });
        }
        if name.starts_with("49_") || name.starts_with("50_") || name.starts_with("51_") {
            backend.source = tt_menu::ServerSource::Internet;
        }
        if *name == "50_online_internet" || *name == "51_online_masters" {
            backend.master = "master.example.org:28100, tt.example.net".into();
            backend.internet_servers = MockBackend::example_servers();
        }
        if *name == "41_online_join_empty" {
            backend.servers.clear();
            backend.querying = true;
        }
        if *name == "44_online_connecting" {
            backend.loading = Some(tt_menu::Loading { map: "Joining Krevitz's Scrum".into(), phase: "Connecting to 192.168.1.31:28000".into(), progress: 0.0 });
        }
        if *name == "45_online_error" {
            backend.message = Some(tt_menu::ScriptMessage { text: "This server is full.".into(), yes_no: false });
        }
        if *name == "15_script_message" {
            backend.message = Some(tt_menu::ScriptMessage {
                text: "Welcome to ThinkTanks \"Neo\". Would you like to try the tutorial now?".into(),
                yes_no: true,
            });
        }
        // The solo tally: final state, while counting (5.1 s: the silver row),
        // the extra-life text (6.8 s).
        let mut t = 60.0;
        if *name == "20_solo_tally_counting" {
            t = 5.1;
        }
        if *name == "21_solo_tally_extra_life" {
            t = 6.8;
        }
        if name.contains("tutorial_") {
            t = 1.0;
        }
        if *name == "33_solo_level_select" {
            // Between runs: the level select (5 levels reached).
            backend.solo = None;
        }
        if name.contains("round_results") {
            let mut r = tt_menu::RoundResults::example();
            if name.contains("ffa") {
                r.teams.clear();
                for p in &mut r.players {
                    p.team = None;
                }
                r.next_in = None;
                r.loading = Some(0.45);
            }
            backend.results = Some(r);
        }
        if let Some(solo) = backend.solo.as_mut() {
            if *name == "22_solo_many_lives" {
                // More than 4 lives: one icon and "x 7" (soloPlayGui::showLives).
                solo.lives = 7;
                solo.score = 61250;
                solo.tally_anim = None;
            }
            if *name == "23_solo_brain_level" {
                solo.title = "Green Brain".into();
                solo.mission = "greenbrain".into();
                let (mut score, lives) = (20400i64, 4u32);
                let mut steps = vec![tt_menu::TallyStepInfo { event: tt_menu::TallyEvent::BonusText, delay_ms: 1500, score, lives }, tt_menu::TallyStepInfo { event: tt_menu::TallyEvent::ShowRow(tt_menu::TallyKind::Brain), delay_ms: 750, score, lives }];
                for _ in 0..12 {
                    score += 200;
                    steps.push(tt_menu::TallyStepInfo { event: tt_menu::TallyEvent::Count(tt_menu::TallyKind::Brain), delay_ms: 100, score, lives });
                }
                steps.push(tt_menu::TallyStepInfo { event: tt_menu::TallyEvent::Done, delay_ms: -1, score, lives });
                solo.tally_anim = Some(tt_menu::SoloTally { start_score: 20400, start_lives: 4, pts_per: [0, 0, 0, 200], max_brains: 20, survival_pts: 0, boss_pts: 0, completion_pts: 10000, steps });
                solo.lives = 4;
                solo.score = score;
            }
        }
        let img = render(&gpu, &mut renderer, state, &mut backend, *size, t);
        // Something besides the clear colour was drawn.
        assert!(img.pixels().any(|p| p.0[0] > 40), "{name} is blank");
        img.save(out.join(format!("{name}.png"))).unwrap();
    }
    if std::env::var("TT_MENU_PREVIEW_ONLY").ok().filter(|s| !s.is_empty()).is_none_or(|o| "60_hud".contains(&o)) {
        let size = [1280, 720];
        let ppp = tt_menu::hud::pixels_per_point([size[0] as f32, size[1] as f32]);
        let mut art = tt_menu::hud::HudArt::new(&root);
        let view = example_hud();
        let img = render_with(&gpu, &mut renderer, size, ppp, 1.0, |ctx| {
            // A stand-in for the 3D view behind the HUD.
            ctx.layer_painter(egui::LayerId::background()).rect_filled(ctx.screen_rect(), 0.0, egui::Color32::from_rgb(0x4a, 0x5a, 0x6a));
            tt_menu::hud::draw(ctx, &view, &mut art);
        });
        img.save(out.join("60_hud.png")).unwrap();
    }
    eprintln!("previews in {}", out.display());
}

/// Layout checks at several window sizes (`7x_<screen>_<w>x<h>`): option
/// rows, the tab strips, a server with every detail, bot and map tables
/// with long names, "[Bot]" names, a team mode and a scrolling map list.
#[test]
#[allow(clippy::type_complexity)]
fn layout_previews() {
    use tt_menu::{BotSetup, BotSkill, CustomGame, HostGame, MissionInfo, Team};
    let Some(gpu) = gpu() else {
        eprintln!("no GPU adapter; skipping previews");
        return;
    };
    let out = std::env::var_os("TT_MENU_PREVIEW_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("tt-menu-preview"));
    std::fs::create_dir_all(&out).unwrap();
    let only = std::env::var("TT_MENU_PREVIEW_ONLY").ok().filter(|s| !s.is_empty());
    let root = tt_assets::layout::default_dir();
    let mut renderer = egui_wgpu::Renderer::new(&gpu.device, FORMAT, None, 1, false);
    let bot_names: Vec<String> = ["Hum [Bot]", "Tom [Bot]", "Mal [Bot]", "Krevitzinator [Bot]", "Gul [Bot]", "Gum [Bot]"].iter().map(|s| s.to_string()).collect();
    let bots = |n: usize| -> Vec<BotSetup> { bot_names.iter().take(n).enumerate().map(|(i, b)| BotSetup { name: b.clone(), skill: BotSkill::ALL[i % 3], tank: i % 3, team: Team::ALL[i % 3] }).collect() };
    let game = CustomGame { mission: tt_assets::layout::mission("tt1_1"), mode: 1, bots: bots(4), player_tank: 1, player_team: Team::Auto, minutes: 15, replace_bots: true };
    let screens: Vec<(&str, Box<dyn Fn() -> MenuState>)> = vec![
        ("70_display", Box::new(|| MenuState::at(Screen::Settings))),
        ("71_graphics", Box::new(|| { let mut s = MenuState::at(Screen::Settings); s.set_settings_tab(SettingsTab::Graphics); s })),
        ("72_controls", Box::new(|| { let mut s = MenuState::at(Screen::Settings); s.set_settings_tab(SettingsTab::Controls); s })),
        ("73_mods", Box::new(|| { let mut s = MenuState::at(Screen::Settings); s.set_settings_tab(SettingsTab::Mods); s })),
        ("74_join_full", Box::new(|| { let mut s = MenuState::at(Screen::Online); s.join_sel = Some("IP:203.0.113.77:28000".into()); s })),
        ("75_join_players", Box::new(|| { let mut s = MenuState::at(Screen::Online); s.join_sel = Some("IP:203.0.113.77:28000".into()); s.show_players = true; s })),
        ("76_custom_game", Box::new({ let game = game.clone(); move || { let mut s = MenuState::at(Screen::CustomGame); s.custom = Some(game.clone()); s } })),
        ("77_host", Box::new({ let game = game.clone(); move || { let mut s = MenuState::at(Screen::Online); s.online_tab = OnlineTab::Host; s.host = Some(HostGame { game: game.clone(), name: "Alice's game".into(), max_players: 16, password: String::new(), port: 28000, port_mapping: false, public: true }); s } })),
        ("78_help", Box::new(|| MenuState::at(Screen::Help))),
        ("79_credits", Box::new(|| MenuState::at(Screen::Credits))),
    ];
    for (name, make) in &screens {
        for size in [[1280u32, 720u32], [1600, 900], [1920, 1080], [1280, 800]] {
            let file = format!("{name}_{}x{}", size[0], size[1]);
            if only.as_ref().is_some_and(|o| !file.contains(o.as_str())) {
                continue;
            }
            let mut backend = MockBackend::new();
            backend.game_root = Some(root.clone());
            backend.bot_names = bot_names.clone();
            backend.extra_missions = ["Special Arena", "Lobotomy Lagoon of the Long Name", "Cerebellum Sky Bridge"]
                .iter()
                .enumerate()
                .map(|(i, n)| MissionInfo { file: format!("missions/X{i}.ron"), id: format!("X{i}"), name: n.to_string(), world: "Special".into() })
                .collect();
            let mut full = MockBackend::example_servers()[1].clone();
            full.address = "IP:203.0.113.77:28000".into();
            full.name = "Krevitz's Extremely Long Scrum Server Name".into();
            full.map = "Hypothalamus Hills".into();
            full.mode = "Team Battlemode".into();
            full.players = 15;
            full.max_players = 16;
            full.bots = 4;
            full.ping_ms = 212;
            full.mods = vec!["Scrum Arenas 1.1".into(), "HD Tank Skins 1.2".into(), "Lava Arenas With Jump Pads Over The Lava 0.9".into()];
            full.modded = true;
            backend.servers.push(full);
            let mut state = make();
            let img = render(&gpu, &mut renderer, &mut state, &mut backend, size, 60.0);
            assert!(img.pixels().any(|p| p.0[0] > 40), "{file} is blank");
            img.save(out.join(format!("{file}.png"))).unwrap();
        }
    }
}

/// A HUD of a team game: scoreboard with K/D/A, clock, score, lives, prints, chat.
fn example_hud() -> tt_menu::hud::HudView {
    use tt_menu::hud::*;
    let row = |team, name: &str, score: &str, kda: [&str; 3], me| ListRow { team, name: name.into(), score: score.into(), kda: Some(kda.map(String::from)), ping: None, me };
    HudView {
        time_ms: 0,
        chat: vec![
            ChatLine { text: "Krevitz: nice shot".into(), chat: true, alpha: 1.0, slot: 2.0 },
            ChatLine { text: "Alice took the lead".into(), chat: false, alpha: 1.0, slot: 1.0 },
        ],
        chat_input: Some(ChatInput { team: false, text: "on my way".into() }),
        name_colors: Vec::new(),
        center_print: Some("Round over in 30 seconds".into()),
        bottom_print: None,
        clock: Some("12:47".into()),
        score: Some("18640".into()),
        lives: Some(Lives { icons: 3, count: None }),
        brains: None,
        lag: false,
        radar: Some(Radar { heading_deg: 0.0, params: Default::default(), blips: Vec::new() }),
        names: Vec::new(),
        player_list: Some(PlayerList {
            teams: vec![row(TeamDot::Green, "Green team", "34", ["", "", ""], false), row(TeamDot::Blue, "Blue team", "27", ["", "", ""], false)],
            players: vec![
                row(TeamDot::Green, "Alice", "18", ["9", "3", "4"], true),
                row(TeamDot::Blue, "Krevitz", "15", ["7", "5", "2"], false),
                row(TeamDot::Green, "Hum", "16", ["8", "6", "1"], false),
                row(TeamDot::Blue, "Mal", "12", ["6", "7", "3"], false),
            ],
            kda: true,
            ping: false,
        }),
    }
}

/// The first-run screen ("Locate your ThinkTanks installation"), drawn with
/// no game data at all. The PNG goes to `$TT_SETUP_PREVIEW` (a file path),
/// else `<tmp>/tt-menu-preview/setup.png`; with
/// `TT_SETUP_PREVIEW_STAGE=progress` the import progress is drawn instead,
/// with `=report` the import report.
#[test]
fn setup_screen_preview() {
    let Some(gpu) = gpu() else {
        eprintln!("no GPU adapter; skipping the preview");
        return;
    };
    let mut st = SetupState::new(SetupMode::FirstRun);
    st.path = "/home/player/Games/ThinkTanks".into();
    st.target = format!("/home/player/.local/share/{}/gamedata", tt_assets::product::DATA_DIR);
    st.error = Some("No ThinkTanks installation in /home/player/Games/ThinkTanks: it has no \"game\" folder. Choose the folder ThinkTanks was installed to (it holds the \"game\" and \"common\" folders).".into());
    if std::env::var("TT_SETUP_PREVIEW_STAGE").as_deref() == Ok("progress") {
        st.error = None;
        st.progress = Some(SetupProgress { phase: "Converting textures".into(), done: 212, total: 467, file: "models/tanks/tank01.png".into() });
    }
    // The report of an import of an edition without the tutorial slides.
    if std::env::var("TT_SETUP_PREVIEW_STAGE").as_deref() == Ok("report") {
        st.error = None;
        let mut lines = vec!["The install has no tutorial art: the tutorial is not offered.".to_string(), "Skipped (9):".to_string()];
        lines.extend((1..=8).map(|n| format!("  ui/backgrounds/marquee{n}.png: optional, not found: tutorial disabled")));
        lines.push("  ui/backgrounds/marqueebackground.png: optional, not found: tutorial disabled".into());
        lines.push("Not found in the install (17):".into());
        lines.extend((1..=8).flat_map(|n| [format!("  game/client/ui/backgrounds/Marquee{n}.jpg"), format!("  game/client/ui/backgrounds/Marquee{n}.alpha.jpg")]));
        lines.push("Warnings (1):".into());
        lines.push("  game/data/missions/TT3_2.mis: decoded from UTF-16 (little-endian)".into());
        st.report = Some(SetupReport { ok: true, summary: "325 files imported, 9 skipped, 17 optional files not found in the install, 1 warning.".into(), lines, log: format!("/home/player/.local/share/{}/logs/import.log", tt_assets::product::DATA_DIR) });
    }
    let mut renderer = egui_wgpu::Renderer::new(&gpu.device, FORMAT, None, 1, false);
    for size in [[1280u32, 720u32], [1600, 900], [1920, 1080], [1280, 800]] {
        let ppp = theme::pixels_per_point([size[0] as f32, size[1] as f32]);
        let img = render_with(&gpu, &mut renderer, size, ppp, 1.0, |ctx| {
            let _ = st.ui(ctx);
        });
        // Something was drawn: the screen is not one flat colour.
        let first = *img.get_pixel(0, 0);
        assert!(img.pixels().any(|p| *p != first), "the setup screen drew nothing");
        // 1280x720 at the path given; other sizes beside it (`setup_<w>x<h>.png`).
        let mut path = std::env::var_os("TT_SETUP_PREVIEW").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("tt-menu-preview").join("setup.png"));
        if size != [1280, 720] {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("setup").to_string();
            path.set_file_name(format!("{stem}_{}x{}.png", size[0], size[1]));
        }
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        img.save(&path).unwrap();
        eprintln!("setup screen preview: {}", path.display());
    }
}
