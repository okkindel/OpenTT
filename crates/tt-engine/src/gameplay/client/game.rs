//! The key bindings are data; the bound actions build the move in
//! `EngineHost::client.moves`, which the simulation reads each tick.

use crate::host::EngineHost;
use crate::input::Command;
use tt_gameplay::client::binds::{applies, BindAction, Binds, MouseSettings, MoveAxis, TurnKey};
use tt_gameplay::client::value::{lf, lu, Val};
use tt_gameplay::{ClientCmd, DebugCmd};
use tt_shell::VarValue;


/// The round clock. The first call of a round (`second = 0`) keeps the
/// time for the end-of-round stats key (`synch_time`) and hides the
/// clock; later ones set it, shown for the last 30 seconds.
pub fn sync_clock(h: &mut EngineHost, time: &str, second: &str) {
    if lu(second) == 0 {
        h.cl.synch_time = Some(Val::s(time));
        h.client.clock.show_time = "-1".into();
        h.client.clock.visible = false;
        return;
    }
    let now = super::hud::now(h);
    h.client.clock.set_time(time, now);
    h.client.clock.show_time = "30".into();
}


/// A new round, the scores back to 0 (multiplayer).
pub fn game_start(h: &mut EngineHost) {
    if !h.srv.state().setup.single_player() {
        h.client.players.zero_scores();
    }
}

/// A counted multiplayer round goes to the current player's stats
/// (saved); single player: the level is over (the solo screen);
/// multiplayer: the round results (`EndGameGui`).
pub fn game_end(h: &mut EngineHost, message: &str) {
    if !h.srv.state().setup.single_player() {
        let key = tt_game::obfuscation::strswiz(&format!("{}echo", h.cl.synch_time_text()), "12");
        let results = tt_game::obfuscation::strfrap_decode(&key, message).unwrap_or_default();
        let word = |i: usize| tt_shell::vars::get_word(&results, i);
        if tt_game::fmt::atof(&word(0)) != 0.0 {
            h.shell.players.record_mp_game(tt_game::fmt::atof(&word(1)), tt_game::fmt::atof(&word(2)));
            crate::shell::save_players(h);
        }
    }
    h.audio.stop_all();
    if h.srv.state().setup.single_player() {
        crate::shell::show(h, crate::canvas::Ctrl::SoloPlayGui, "");
        return;
    }
    h.cl.synch_time = Some(Val::Int(0));
    h.client.game_end();
    crate::canvas::set_content(h, crate::canvas::Ctrl::EndGameGui);
}


/// A bound action with the value the action map gives it ("1"/"0" for a
/// key, the axis delta, "" for a press-only binding).
pub fn bind_action(h: &mut EngineHost, a: BindAction, v: &str) {
    use BindAction::*;
    match a {
        EscapeFromGame => crate::shell::quit_show(h),
        MoveLeft => h.client.moves.set_axis(MoveAxis::Left, v),
        MoveRight => h.client.moves.set_axis(MoveAxis::Right, v),
        MoveForward => h.client.moves.set_axis(MoveAxis::Forward, v),
        MoveBackward => h.client.moves.set_axis(MoveAxis::Backward, v),
        MoveUp => h.client.moves.set_axis(MoveAxis::Up, v),
        MoveDown => h.client.moves.set_axis(MoveAxis::Down, v),
        TurnLeft => turn(h, v, TurnKey::Left),
        TurnRight => turn(h, v, TurnKey::Right),
        PanUp => turn(h, v, TurnKey::Up),
        PanDown => turn(h, v, TurnKey::Down),
        Yaw => {
            let m = mouse_settings(h);
            h.client.moves.mouse_yaw(v, &m);
        }
        Pitch => {
            let m = mouse_settings(h);
            h.client.moves.mouse_pitch(v, &m);
        }
        Fire => h.client.moves.trigger(0),
        AltFire => h.client.moves.trigger(1),
        ToggleFreeLook => h.client.moves.free_look = lf(v) != 0.0,
        // No one reads it.
        ToggleFirstPerson => {}
        ToggleCamera => {
            if lf(v) != 0.0 {
                debug_cmd(h, DebugCmd::ToggleCamera);
            }
        }
        Chat => super::hud::message_hud_key(h, v, false),
        TeamChat => super::hud::message_hud_key(h, v, true),
        // The paging of the old chat HUD (nothing to page).
        PageMessagesUp | PageMessagesDown => {}
        ResizeMessages => {
            if lf(v) != 0.0 {
                super::hud::next_chat_hud_len(h);
            }
        }
        // Demo recording is not supported; the debug actions are not in a release build.
        StartDemoRecording | StopDemoRecording | DropCameraAtPlayer | DropPlayerAtCamera | CycleDebugRenderMode | CycleRenderMode => {}
        Spawn => send_spawn_command(h, v),
        // The game's console dialog (the dedicated server's console is stdin).
        ToggleConsole => h.stub("toggleConsole"),
        ToggleFullScreen => crate::video::toggle_fullscreen(h),
        ToggleHelp => crate::shell::toggle_help(h),
        ToggleLogMode => {
            if lu(v) != 0 {
                if h.log.mode == 0 {
                    h.log.info("Setting console logging to on...");
                    h.log.mode = 2;
                } else {
                    h.log.mode = 0;
                    h.log.info("Setting console logging to off...");
                }
            }
        }
        // No scripts to trace, no profiler.
        ToggleScriptTrace | DoProfile => h.stub(a.function()),
        CycleTank => debug_cmd(h, DebugCmd::CycleTank),
        TogglePlayerAi => {
            if lu(v) != 0 && (!h.platform.release || h.srv.state().setup.demo()) {
                debug_cmd(h, DebugCmd::TogglePlayerAI);
                let s = &mut h.srv.state_mut().setup;
                s.ai_control = !s.ai_control;
            }
        }
        Quit => {
            if lf(v) != 0.0 {
                crate::shell::quit(h);
            }
        }
        Screenshot => do_screenshot(h, v),
    }
}

fn debug_cmd(h: &mut EngineHost, cmd: DebugCmd) {
    crate::net::send_cmd(h, ClientCmd::Debug { cmd, args: Vec::new() });
}

/// `sendSpawnCommand(val)`: respawn while the player's tank is a corpse.
fn send_spawn_command(h: &mut EngineHost, v: &str) {
    if lu(v) == 0 {
        return;
    }
    let Some(e) = crate::net::client_control_object(h) else { return };
    let corpse = h.game.as_ref().and_then(|g| g.tank_of(e).map(|t| g.world.is_corpse(t))).unwrap_or(false);
    if corpse {
        crate::net::send_cmd(h, ClientCmd::SpawnPlayer);
        h.client.clear_center();
    }
}

/// `doScreenShot(val)`: `screenshots/screenshot_<n>.png` in the data
/// dir.
fn do_screenshot(h: &mut EngineHost, v: &str) {
    if lf(v) == 0.0 {
        return;
    }
    h.cl.screenshot_number = (h.cl.screenshot_number as f64 + 1.0) as f32;
    let mut num = tt_game::fmt::format_g(h.cl.screenshot_number as f64);
    for limit in [10.0, 100.0, 1000.0, 10000.0] {
        if lf(&num) < limit {
            num = format!("0{num}");
        }
    }
    h.screenshot_request = Some(format!("{}/screenshot_{num}.png", tt_shell::save::SCREENSHOTS));
}

/// The default bindings (`assets/data/binds.ron`, or a mod's).
pub fn default_binds(h: &EngineHost) -> Binds {
    let path = h.fs.root().join("data/binds.ron");
    let path = tt_assets::mods::redirect(&path).unwrap_or(path);
    match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| Binds::parse(&t)) {
        Ok(b) => b,
        Err(e) => {
            // The asset directory always has it; a broken copy leaves the game without keys.
            eprintln!("{}: {e}", path.display());
            Binds::default()
        }
    }
}

/// The player's bindings file in the data directory.
pub const USER_BINDS: &str = tt_shell::save::BINDS;

/// The bindings in effect: the defaults with the player's maps over them.
pub fn load_binds(h: &mut EngineHost) -> Binds {
    let mut b = default_binds(h);
    if let Some(t) = h.fs.read_data(USER_BINDS).map(|d| String::from_utf8_lossy(&d).into_owned()) {
        match Binds::parse(&t) {
            Ok(u) => b.overlay(u),
            Err(e) => h.log.warn(&format!("{USER_BINDS}: {e} (the default bindings apply)")),
        }
    }
    b
}

/// The bindings into the engine's action maps.
pub fn apply_binds(h: &mut EngineHost, binds: &Binds) {
    let (rel, demo, mac) = (h.platform.release, h.srv.state().setup.demo(), h.platform.platform.eq_ignore_ascii_case("macos"));
    for map in &binds.maps {
        if !h.input.has_map(&map.name) {
            continue;
        }
        for b in map.binds.iter().filter(|b| applies(b.when, rel, demo, mac)) {
            let command = if b.action.press_only() {
                Command::Press(b.action)
            } else {
                Command::Action { action: b.action, inverted: b.invert, scale: b.scale.unwrap_or(1.0) }
            };
            let r = h.input.bind(&map.name, b.device.into(), &b.key, command);
            if let Err(e) = r {
                h.log.warn(&e);
            }
        }
    }
}

pub fn init_binds(h: &mut EngineHost) {
    h.input.new_map(crate::input::MOVE_MAP);
    h.cl.camera_fov = Some(Val::Int(90));
    h.client.moves.movement_speed = tt_gameplay::client::text::SVal::Int(1);
    let binds = load_binds(h);
    apply_binds(h, &binds);
}

/// The prefs the mouse axes read.
fn mouse_settings(h: &EngineHost) -> MouseSettings {
    let p = &h.shell.prefs;
    MouseSettings { sensitivity: mouse_sensitivity(h), camera_fov: h.cl.camera_fov_num(), invert: super::hud::pref_float(p, "input.invert_mouse") }
}

/// The sensitivity slider.
pub fn mouse_sensitivity(h: &EngineHost) -> f64 {
    use tt_gameplay::client::binds::{DEFAULT_MOUSE_SENSITIVITY, MOUSE_SENSITIVITY_PREF};
    let p = &h.shell.prefs;
    if p.get(MOUSE_SENSITIVITY_PREF).is_empty() {
        DEFAULT_MOUSE_SENSITIVITY
    } else {
        super::hud::pref_float(p, MOUSE_SENSITIVITY_PREF).clamp(0.0, 1.0)
    }
}

fn turn(h: &mut EngineHost, v: &str, k: TurnKey) {
    let speed = h.shell.prefs.get("input.keyboard_turn_speed");
    h.client.moves.turn(k, v, &speed);
}


enum D {
    U(&'static str, i32),
    F(&'static str, f32),
    T(&'static str, &'static str),
}

fn defaults(h: &mut EngineHost, list: &[D]) {
    let p = &mut h.shell.prefs;
    for d in list {
        match d {
            D::U(n, v) => p.declare(n, VarValue::Int(*v)),
            D::F(n, v) => p.declare(n, VarValue::Float(*v)),
            D::T(n, v) => p.declare(n, VarValue::Str(v.to_string())),
        }
    }
}

/// The client prefs.
pub fn client_defaults(h: &mut EngineHost) {
    let idle = tt_shell::IDLE_TEXT;
    let lock = tt_game::obfuscation::strfrip(idle, "0");
    h.shell.prefs.declare("player.parental_lock", VarValue::Str(lock));
    defaults(h, &[D::U("demo.never_show_marquee", 0), D::T("player.name", "Bait"), D::U("player.tank", 0), D::T("player.team", "")]);
    defaults(h, &[D::U("input.auto_turn", 0)]);
    defaults(
        h,
        &[
            D::U("net.lag_threshold", 400),
            D::U("hud.message_log_size", 40),
            D::U("hud.chat_length", 1),
            D::U("shell.fade_time", 150),
            D::U("shell.trans_delay", 250),
            D::F("input.link_mouse_sensitivity", 0.5),
            D::U("input.keyboard_enabled", 1),
            D::U("input.mouse_enabled", 1),
            D::U("input.joystick_enabled", 0),
            D::F("input.keyboard_turn_speed", 0.1),
            D::U("lighting.cache_size", 20000),
            D::T("lighting.purge_method", "lastCreated"),
            D::U("lighting.cache_lighting", 1),
            D::U("lighting.terrain_generate_level", 1),
            D::U("terrain.dynamic_lights", 1),
            D::T("video.display_device", "OpenGL"),
            D::U("video.allow_opengl", 1),
            D::U("video.allow_d3d", 1),
            D::U("video.prefer_opengl", 1),
            D::U("video.applied_pref", 0),
            D::U("video.disable_vertical_sync", 1),
            D::U("video.monitor_num", 0),
            D::T("video.sim_resolution", "800 600 32"),
            D::T("video.shell_resolution", "800 600 32"),
            D::T("video.full_screen", "1"),
            D::T("video.detail_level", "2"),
            D::T("opengl.force_16bit_texture", "0"),
            D::T("opengl.force_paletted_texture", "0"),
            D::U("opengl.max_hardware_lights", 3),
            D::F("video.visible_distance_mod", 1.0),
            D::T("audio.driver", "OpenAL"),
            D::U("audio.force_max_distance_update", 0),
            D::U("audio.environment_enabled", 0),
            D::U("audio.master_volume", 1),
            D::U("audio.channel_volume.1", 1),
            D::U("audio.sound_volume", 1),
            D::U("audio.channel_volume.3", 1),
            D::F("audio.music_volume", 0.8),
            D::U("audio.channel_volume.5", 1),
            D::U("audio.channel_volume.6", 1),
            D::U("audio.channel_volume.7", 1),
            D::U("audio.channel_volume.8", 1),
            D::U("unix.openal_frequency", 44100),
        ],
    );
    // VSync on by default.
    defaults(h, &[D::T("video.disable_vertical_sync", "0")]);
    let d = h.shell.prefs.get("video.detail_level");
    set_detail_level(h, &d);
}

/// `setThinkTanksDetailLevel(detail)`: the detail prefs of a level 0..2.
pub fn set_detail_level(h: &mut EngineHost, detail: &str) {
    let detail = lf(detail);
    if detail == 2.0 {
        defaults(
            h,
            &[
                D::U("terrain.screen_error", 3),
                D::U("shapes.detail_adjust", 1),
                D::U("shapes.skip_render_dls", 0),
                D::F("shapes.shadow_detail", 0.5),
                D::U("opengl.mip_reduction", 0),
                D::U("opengl.sky_mip_reduction", 0),
                D::U("particle.detail_adjust", 1),
                D::U("terrain.use_small", 0),
                D::U("terrain.tex_detail", 0),
            ],
        );
    } else if detail == 1.0 {
        defaults(
            h,
            &[
                D::U("terrain.screen_error", 4),
                D::F("shapes.detail_adjust", 0.9),
                D::U("shapes.skip_render_dls", 0),
                D::F("shapes.shadow_detail", 0.5),
                D::U("opengl.mip_reduction", 0),
                D::U("opengl.sky_mip_reduction", 0),
                D::F("particle.detail_adjust", 0.8),
                D::U("terrain.use_small", 1),
                D::U("terrain.tex_detail", 0),
            ],
        );
    } else if detail == 0.0 {
        defaults(
            h,
            &[
                D::U("terrain.screen_error", 10),
                D::F("shapes.detail_adjust", 0.8),
                D::U("shapes.skip_render_dls", 1),
                D::F("shapes.shadow_detail", 0.0),
                D::U("opengl.mip_reduction", 1),
                D::U("opengl.sky_mip_reduction", 1),
                D::F("particle.detail_adjust", 0.6),
                D::U("terrain.use_small", 1),
                D::U("terrain.tex_detail", 1),
            ],
        );
    }
}

pub fn set_video_profile(h: &mut EngineHost) {
    let (vendor, renderer) = ("wgpu", "wgpu");
    h.log.info("Checking video card profiles...");
    let p = &h.shell.prefs;
    let up_to_date = super::hud::pref_float(p, "video.profile_version") == 4.0
        && tt_game::fmt::stricmp(&p.get("video.profiled_vendor"), vendor) == 0
        && tt_game::fmt::stricmp(&p.get("video.profiled_renderer"), renderer) == 0;
    if up_to_date {
        h.log.info("...up to date.");
        return;
    }
    h.log.info("Updating video card profile.");
    defaults(
        h,
        &[
            D::U("interior.lock_arrays", 1),
            D::U("shapes.fog_texture", 0),
            D::U("shapes.skip_first_fog", 0),
            D::U("opengl.disable_ext_compiled_vertex_array", 0),
            D::U("opengl.disable_sub_image", 0),
            D::U("opengl.no_env_color", 0),
            D::U("opengl.disable_arb_texture_compression", 0),
            D::U("opengl.no_draw_arrays_alpha", 0),
            D::U("opengl.max_texture_dim", 512),
            D::U("video.only16", 0),
            D::U("video.safe_mode_on", 1),
            D::U("video.clip_high", 0),
            D::U("video.delete_context", 1),
            D::T("video.sim_resolution", "800 600 32"),
            D::U("video.detail_level", 2),
        ],
    );
    // (No vendor-specific profile matches "wgpu".)
    let d = h.shell.prefs.get("video.detail_level");
    set_detail_level(h, &d);
    defaults(h, &[D::T("video.profile_version", "4"), D::T("video.profiled_vendor", vendor), D::T("video.profiled_renderer", renderer)]);
}

// ---- the client's sound data ------------------------------------------------------

/// The client's interface sounds: the GUI/message/music sound
/// descriptions and sounds.
pub fn declare_sounds(h: &mut EngineHost) {
    declare_client_file(h, tt_assets::data::defs::CLIENT_AUDIO);
}

/// The `MusicPlayer` and the music tracks.
pub fn declare_music(h: &mut EngineHost) {
    h.cl.music_player = true;
    declare_client_file(h, tt_assets::data::defs::MUSIC);
}

fn declare_client_file(h: &mut EngineHost, stem: &str) {
    let file = match tt_assets::data::defs::load_client_file(h.fs.root(), stem) {
        Ok(f) => f,
        Err(e) => {
            h.log.error(&format!("data file: {e}"));
            return;
        }
    };
    crate::gameplay::defs::log_data_notes(h);
    // Plain objects of the client (not defs): the sound database only.
    for o in file {
        let fields: Vec<(String, String)> = o.to_fields().into_iter().map(|f| (f.key(), f.value)).collect();
        h.audio.set_object(o.class(), o.name(), &fields);
    }
}
