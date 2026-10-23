//! The example mods that ship with the game (`mods/` of the repository,
//! `docs/modding.md` § Example mods) through the engine: Options → Mods
//! lists them out of the box, marked as examples and not enabled; enabled,
//! Desert Camo's textures replace the tanks' and Carpet Bombing's Mortar is
//! the game's (bots fire it), and a network game needs Carpet Bombing on
//! both sides.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tt_engine::gameplay::server_host;
use tt_engine::{Config, Engine};
use tt_menu::{BotSetup, BotSkill, CustomGame, MenuAction, MenuBackend, Team};
use tt_net::{LinkConditions, MemNet};

mod common;
use common::*;

const EXAMPLES: [&str; 2] = ["carpet-bombing", "desert-camo"];

fn repo_mods() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// An engine (virtual clock, not booted) on its own link to the game data
/// (the mod overlay is per asset directory and process), a fresh user
/// data directory, `enabled` enabled (`None`: no settings file at all).
fn engine_with(tag: &str, enabled: Option<&[&str]>, args: &[&str]) -> Engine {
    let b = std::env::temp_dir().join(format!("tt-example-mods-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&b);
    std::fs::create_dir_all(&b).unwrap();
    let real = assets().canonicalize().unwrap();
    #[cfg(unix)]
    let link = {
        let link = b.join("assets");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        link
    };
    #[cfg(not(unix))]
    let link = real;
    let data = b.join("data");
    std::fs::create_dir_all(&data).unwrap();
    if let Some(on) = enabled {
        let mut settings = tt_shell::Settings::default();
        settings.mods.enabled = Some(on.iter().map(|e| e.to_string()).collect());
        std::fs::write(data.join("settings.ron"), settings.to_ron()).unwrap();
    }
    let cfg = Config { assets: link, data, args: args.iter().map(|s| s.to_string()).collect(), audio_output: false, echo_log: false, windowed: true };
    let mut e = Engine::new(&cfg).unwrap();
    e.host.virtual_clock_ms = Some(0.0);
    e
}

#[test]
fn listed_as_examples_and_not_enabled() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let mut e = engine_with("listed", None, &[]);
        assert!(e.boot());
        assert!(e.host.fs.mods().is_empty(), "no mod is on out of the box");
        let info = backend_do(&mut e, |b| b.mods()).expect("Options → Mods");
        for id in EXAMPLES {
            let m = info.mods.iter().find(|m| m.id == id).unwrap_or_else(|| panic!("{id} is not listed: {:?}", info.mods));
            assert!(m.example && !m.enabled && m.name.ends_with("(example)") && m.author == "OpenTT", "{m:?}");
        }
        assert!(info.mods.iter().find(|m| m.id == "carpet-bombing").unwrap().gameplay);
        assert!(!info.mods.iter().find(|m| m.id == "desert-camo").unwrap().gameplay);
        assert!(info.enabled().is_empty() && info.running.is_empty());
    });
}

#[test]
fn enabled_examples_change_the_game() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let mut e = engine_with("enabled", Some(&EXAMPLES), &[]);
        assert!(e.boot());
        let ids: Vec<&str> = e.host.fs.mods().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, EXAMPLES);
        // Desert Camo: its lowercase textures in place of the game's, by
        // any spelling.
        for (game, ours) in [("Tank01.png", "tank01.png"), ("TANK02.png", "tank02.png"), ("tank03.png", "tank03.png"), ("tank04.png", "tank04.png")] {
            let want = std::fs::read(repo_mods().join("desert-camo/models/tanks").join(ours)).unwrap();
            assert!(e.host.fs.read(&format!("models/tanks/{game}")).unwrap() == want, "{game}");
            let root = e.host.fs.root().to_path_buf();
            assert_eq!(std::fs::read(tt_assets::find_ci(&root.join("models/tanks").join(game)).unwrap()).unwrap(), want, "{game}");
        }
        // Carpet Bombing: the game's Mortar fires 9 shells, 3 volleys a pull.
        new_player(&mut e, "Tester");
        let bot = |name: &str, team| BotSetup { name: name.into(), skill: BotSkill::Gold, tank: 0, team };
        let g = CustomGame { mission: "missions/tt1_1.ron".into(), mode: 1, bots: vec![bot("A", Team::Blue), bot("B", Team::Green), bot("C", Team::Green), bot("D", Team::Blue)], player_tank: 1, player_team: Team::Blue, minutes: 10, replace_bots: true };
        apply(&mut e, MenuAction::StartCustom(g));
        for _ in 0..(5000 / tt_engine::TICK_MS) {
            tick(&mut e);
        }
        let p = e.host.game.as_mut().unwrap().projectile("MortarProjectile").unwrap();
        assert_eq!((p.pellet_count, p.volley_size, p.rounds), (9, 3, 12));
        // Bots with the Mortar fire it (as many rounds as they like).
        let mut shells = BTreeSet::new();
        for k in 0..(60_000 / tt_engine::TICK_MS) {
            if k % 30 == 0 {
                let tanks: Vec<_> = server_host::bots(&e.host).into_iter().filter_map(|c| server_host::player_tank(&e.host, c)).collect();
                let g = e.host.game.as_mut().unwrap();
                for t in tanks {
                    let empty = g.world.tank(t).is_some_and(|t| t.current.as_ref().is_none_or(|c| c.name != "MortarProjectile") || t.ammo < 3);
                    if empty {
                        g.world.set_projectile(t, p.clone());
                    }
                }
            }
            tick(&mut e);
            let g = e.host.game.as_ref().unwrap();
            shells.extend(g.world.projectiles.iter().filter(|s| s.data.name == "MortarProjectile").map(|s| s.id));
            if shells.len() >= 2 * 27 {
                break;
            }
        }
        assert!(shells.len() >= 27, "the bots fired {} Mortar shells", shells.len());
    });
}

#[test]
fn network_games_need_carpet_bombing_on_both_sides() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let net = MemNet::new(31);
        net.set_conditions(LinkConditions { latency: Duration::from_millis(20), ..Default::default() });
        let mut s = engine_with("net-server", Some(&EXAMPLES), &["-dedicated", "-mission", "missions/tt1_1.ron"]);
        s.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 1]));
        assert!(s.boot());
        assert_eq!(tt_engine::netgame::server_addr(&s.host), Some(SocketAddr::from(([10, 0, 0, 1], 28000))));
        // Only the gameplay mod is announced.
        let info = tt_engine::netgame::info::server_info(&s.host);
        assert_eq!(info.mods.iter().map(|m| m.label()).collect::<Vec<_>>(), ["Carpet Bombing (example) 1.0"]);
        let mut t = 0u64;
        let mut join = |s: &mut Engine, c: &mut Engine| -> String {
            tt_engine::net::connect_to(&mut c.host, "10.0.0.1:28000", "", "Tester", "");
            for _ in 0..2000 {
                step_all(&net, &mut t, &mut [&mut *s, &mut *c]);
                if !conn_err(c).is_empty() {
                    return conn_err(c);
                }
                if c.host.game.is_some() && log_has(c, "*** Initial Control Object") {
                    return String::new();
                }
            }
            dump_log(s, "server");
            dump_log(c, "client");
            panic!("neither in the game nor refused");
        };
        // Installed (shipped) but not enabled: told to enable it.
        let mut off = engine_with("net-off", None, &[]);
        assert!(off.boot());
        new_player(&mut off, "Tester");
        off.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 2]));
        let err = join(&mut s, &mut off);
        assert!(err.contains("enable the mod \"Carpet Bombing (example) 1.0\""), "{err}");
        // Enabled (the cosmetic one need not be): in the game, the defs agree.
        let mut on = engine_with("net-on", Some(&["carpet-bombing"]), &[]);
        assert!(on.boot());
        new_player(&mut on, "Tester");
        on.host.netgame.factory = Some(mem_factory(&net, [10, 0, 0, 3]));
        assert_eq!(join(&mut s, &mut on), "");
    });
}

/// Readable Pads, a cosmetic mod that ships with the game: enabled, each
/// boost and jump pad is drawn with its overlay (`boostpad.overlay.dts`,
/// `jumppad.overlay.dts`) on top, without a lamp of its own, the boost
/// arrow pointing where the pad launches and turning with the pad's state;
/// not enabled, the pads are drawn alone.
#[test]
fn readable_pads_draw_overlays_over_the_pads() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let node = |i: &tt_render::DynamicInstance, n: &str| i.nodes.iter().find(|(m, _)| m.eq_ignore_ascii_case(n)).map(|x| x.1);
        // The boost overlay's arrow (its root `direction`'s +Y) along the
        // launch direction of the pad drawn as `model`.
        let along_launch = |e: &Engine, model: &tt_render::DynamicInstance, overlay: &tt_render::DynamicInstance| {
            let pos = model.transform.w_axis.truncate();
            let pad = e.host.game.as_ref().unwrap().world.triggers.iter().find(|t| (glam::Vec3::from(t.transform.translation) - pos).length() < 0.01).expect("the pad");
            let arrow = (overlay.transform * node(overlay, "direction").unwrap()).y_axis.truncate().normalize();
            assert!(arrow.dot(pad.boost_direction().normalize()) > 0.999, "{arrow} {}", pad.boost_direction());
        };
        // (overlay file name, model, overlay) of each drawn overlay.
        let overlays = |e: &Engine| {
            let (shapes, anims) = e.trigger_shapes_with_anim();
            let mut out = Vec::new();
            for i in 1..shapes.len() {
                let name = shapes[i].shape.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
                if name.ends_with(".overlay.dts") {
                    assert_eq!(anims[i].lamp, tt_engine::gfx::Lamp::None);
                    assert_eq!(shapes[i].transform, shapes[i - 1].transform);
                    out.push((name, shapes[i - 1].clone(), shapes[i].clone()));
                }
            }
            out
        };
        for on in [false, true] {
            let mods: &[&str] = if on { &["readable-pads"] } else { &[] };
            let mut e = engine_with(if on { "pads-on" } else { "pads-off" }, Some(mods), &[]);
            assert!(e.boot());
            assert_eq!(e.host.fs.mods().len(), mods.len());
            assert!(!e.host.fs.mods().iter().any(|m| m.affects_gameplay));
            new_player(&mut e, "Tester");
            let g = CustomGame { mission: "missions/poweruptest.ron".into(), mode: 0, bots: Vec::new(), player_tank: 1, player_team: Team::Green, minutes: 10, replace_bots: true };
            apply(&mut e, MenuAction::StartCustom(g));
            for _ in 0..(3000 / tt_engine::TICK_MS) {
                tick(&mut e);
            }
            let found = overlays(&e);
            if !on {
                assert!(found.is_empty(), "{:?}", found.iter().map(|o| &o.0).collect::<Vec<_>>());
                continue;
            }
            let mut names: Vec<&str> = found.iter().map(|o| o.0.as_str()).collect();
            names.sort();
            // The Power-up Test map's two boost and two jump pads.
            assert_eq!(names, ["boostpad.overlay.dts", "boostpad.overlay.dts", "jumppad.overlay.dts", "jumppad.overlay.dts"]);
            for (name, model, overlay) in &found {
                if name.starts_with("boostpad") {
                    assert!(node(overlay, "direction").is_some() && node(overlay, "direction") == node(model, "direction"));
                    assert!(node(overlay, "padglow_arrow").is_some(), "the arrow bobs");
                    along_launch(&e, model, overlay);
                }
            }
            // The 4-state pad in its second state: the arrow turns a quarter.
            let g = e.host.game.as_mut().unwrap();
            let pad = g.world.triggers.iter_mut().find(|t| matches!(&t.kind, tt_game::TriggerKind::Boost(b) if b.data.num_states == 4)).expect("the 4-state boost pad");
            let pos = glam::Vec3::from(pad.transform.translation);
            if let tt_game::TriggerKind::Boost(b) = &mut pad.kind {
                b.state = 1;
            }
            let (_, model, turned) = overlays(&e).into_iter().find(|(_, m, _)| (m.transform.w_axis.truncate() - pos).length() < 0.01).unwrap();
            along_launch(&e, &model, &turned);
            let dir = node(&turned, "direction").unwrap().y_axis.truncate();
            assert!(dir.dot(glam::Vec3::Y).abs() < 1e-3 && dir.length() > 0.99, "{dir}");
        }
    });
}
