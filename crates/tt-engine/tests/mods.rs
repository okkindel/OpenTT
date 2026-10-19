//! Mods (docs/modding.md): enabled mods overlay the game data for every
//! loader, their maps join the map list, disabled ones are ignored; in a
//! network game a client without the server's gameplay mod is refused
//! with the mod's name, while cosmetic mods may differ.
//!
//! Each engine reads the game data through its own symbolic link to the
//! asset directory: the mod overlay is installed per asset directory for
//! the whole process, and these engines have different mods.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use tt_engine::{Config, Engine};
use tt_net::{LinkConditions, MemNet};

mod common;
use common::*;

const MISSION: &str = "missions/tt1_1.ron";

/// A mod: (folder name, [(asset path or `mod.ron`, contents)]).
type ModSpec<'a> = (&'a str, Vec<(&'a str, Vec<u8>)>);

fn base(tag: &str) -> PathBuf {
    let b = std::env::temp_dir().join(format!("tt-mods-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&b);
    std::fs::create_dir_all(&b).unwrap();
    b
}

/// An engine (virtual clock, not booted) on its own link to the game data,
/// with `mods` installed in its user data directory and `enabled` enabled.
fn modded_engine(tag: &str, mods: &[ModSpec], enabled: &[&str], args: &[&str]) -> Engine {
    let b = base(tag);
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
    for (id, files) in mods {
        for (rel, bytes) in files {
            let p = data.join("mods").join(id).join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
    }
    std::fs::create_dir_all(&data).unwrap();
    let mut settings = tt_shell::Settings::default();
    settings.mods.enabled = Some(enabled.iter().map(|e| e.to_string()).collect());
    std::fs::write(data.join("settings.ron"), settings.to_ron()).unwrap();
    let cfg = Config { assets: link, data, args: args.iter().map(|s| s.to_string()).collect(), audio_output: false, echo_log: false, windowed: true };
    let mut e = Engine::new(&cfg).unwrap();
    e.host.virtual_clock_ms = Some(0.0);
    e
}

fn text(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

/// A mod map: the Power-up Test arena renamed, placed in world 2.
fn mod_map() -> Vec<u8> {
    let src = std::fs::read_to_string(assets().join("missions/poweruptest.ron")).unwrap();
    let out = src.replacen("display_name: \"Power-up Test\",", "display_name: \"Mod  Arena\",\n          extra: [(\"world\", \"2\")],", 1);
    assert_ne!(out, src);
    out.into_bytes()
}

#[test]
fn overlay_and_map_list() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let mods: Vec<ModSpec> = vec![
            ("maps", vec![("mod.ron", text("(name: \"Arena Pack\", version: \"1.0\", author: \"Tester\")")), ("missions/ModArena.ron", mod_map())]),
            ("skins", vec![("ui/credits.txt", text("skins credits"))]),
            ("later", vec![("ui/credits.txt", text("later credits")), ("sound/README", text("x"))]),
            ("off", vec![("missions/OffMap.ron", mod_map()), ("ui/credits.txt", text("disabled"))]),
        ];
        let mut e = modded_engine("list", &mods, &["maps", "skins", "later"], &[]);
        let root = e.host.fs.root().to_path_buf();
        // The mods in load order; the later of two mods with a file wins,
        // for the file index and for loaders that open paths.
        let ids: Vec<&str> = e.host.fs.mods().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["maps", "skins", "later"]);
        assert!(e.host.fs.mods()[0].affects_gameplay && !e.host.fs.mods()[1].affects_gameplay);
        assert_eq!(e.host.fs.read("ui/credits.txt").unwrap(), b"later credits");
        let file = tt_assets::find_ci(&root.join("ui/credits.txt")).unwrap();
        assert_eq!(std::fs::read(file).unwrap(), b"later credits");
        // The map list: the game's maps in the catalog's order, then the mod's.
        let list = tt_engine::shell::missions(&mut e.host).to_vec();
        let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
        let game = ["tt1_1", "tt1_2", "tt1_3", "tt1_4", "tt1_5", "tt2_1", "tt2_2", "tt2_3", "tt2_4", "tt2_5", "tt3_1", "tt3_2", "tt3_3", "tt3_4", "tt3_5", "poweruptest"];
        assert_eq!(&ids[..16], game);
        assert_eq!(ids.len(), 17, "{ids:?}");
        let m = &list[16];
        assert_eq!((m.id.as_str(), m.name.as_str(), m.world, m.music.as_str()), ("modarena", "Mod Arena", 2, "Lush"));
        assert_eq!(list[12].name, "Neurotic Nexus");
        assert_eq!(list[15].world, 0);
        // A disabled mod is not used.
        assert!(!e.host.fs.exists("missions/OffMap.ron"));
        // The mod map is a mission like the others.
        assert!(tt_engine::mission::mission_exists(&e.host, "missions/ModArena.ron"));
    });
}

/// A mod written before the data vocabulary changed still loads: the old
/// names are read as the new ones, each noted in the log as deprecated.
#[test]
fn old_names_in_a_mod_still_load() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let game = std::fs::read_to_string(assets().join("data/weapons.ron")).unwrap();
        let mut old = game.replace("ProjectileDef(", "ProjectileData(").replace("ReticleDef(", "ReticleData(").replace("blast_radius:", "splash_area:").replace("rounds:", "count:").replace("volley_size:", "burst_count:");
        old = old.replacen("splash_area: 11,", "splash_area: 7,", 1);
        assert!(old.contains("ProjectileData(") && old.contains("splash_area: 7,"));
        let map = String::from_utf8(mod_map()).unwrap().replace("Group(", "SimGroup(").replace("Prop(", "TSStatic(").replace("def: \"", "data_block: \"");
        assert!(map.contains("TSStatic(") || map.contains("data_block:"));
        let mods: Vec<ModSpec> = vec![("old", vec![("mod.ron", text("(name: \"Old Mod\", version: \"1\", author: \"Tester\")")), ("data/weapons.ron", old.into_bytes()), ("missions/OldArena.ron", map.into_bytes())])];
        let mut e = modded_engine("old", &mods, &["old"], &[]);
        assert!(tt_engine::gameplay::defs::load_server_set(&mut e.host, "weapons"));
        let mortar = e.host.defs.get("MortarProjectile").expect("the mod's mortar");
        assert_eq!(mortar.class, "ProjectileDef");
        assert_eq!(mortar.get("blast_radius"), Some("7"));
        assert_eq!(mortar.get("rounds"), Some("6"));
        assert!(mortar.get("splash_area").is_none());
        assert!(log_has(&e, "deprecated name, still read") && log_has(&e, "field `splash_area` (in ProjectileDef) is now `blast_radius`"), "the deprecation notes");
        assert!(log_has(&e, "class `ProjectileData` is now `ProjectileDef`"));
        assert!(tt_engine::mission::exec_mission(&mut e.host, "missions/OldArena.ron"));
        assert!(log_has(&e, "class `SimGroup` is now `Group`"));
        let load = e.host.mission_load.as_ref().expect("the old mission loaded");
        assert!(load.nodes.iter().any(|(_, o)| o.class == "PowerUp" && o.get("def").is_some()));
    });
}

/// A dedicated server on tt1_1 at 10.0.0.1 with `mods`, all enabled.
fn server(net: &MemNet, tag: &str, mods: &[ModSpec]) -> Engine {
    let ids: Vec<&str> = mods.iter().map(|m| m.0).collect();
    let mut s = modded_engine(tag, mods, &ids, &["-dedicated", "-mission", MISSION]);
    s.host.netgame.factory = Some(mem_factory(net, [10, 0, 0, 1]));
    assert!(s.boot());
    assert_eq!(tt_engine::netgame::server_addr(&s.host), Some(SocketAddr::from(([10, 0, 0, 1], 28000))));
    s
}

/// A booted client at `ip` with `mods` installed and `enabled` enabled.
fn client(net: &MemNet, tag: &str, mods: &[ModSpec], enabled: &[&str], ip: u8) -> Engine {
    let mut c = modded_engine(tag, mods, enabled, &[]);
    assert!(c.boot());
    new_player(&mut c, "Tester");
    c.host.netgame.factory = Some(mem_factory(net, [10, 0, 0, ip]));
    c
}

/// Connects and runs until the client is in the game or refused; the
/// refusal's text ("" when it got in).
fn join(net: &MemNet, t: &mut u64, s: &mut Engine, c: &mut Engine) -> String {
    tt_engine::net::connect_to(&mut c.host, "10.0.0.1:28000", "", "Tester", "");
    for _ in 0..2000 {
        step_all(net, t, &mut [&mut *s, &mut *c]);
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
}

#[test]
fn client_without_the_servers_gameplay_mod_is_refused() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let net = MemNet::new(11);
        net.set_conditions(LinkConditions { latency: Duration::from_millis(20), ..Default::default() });
        let rules: ModSpec = ("rules", vec![("mod.ron", text("(name: \"Rule Pack\", version: \"1.0\")")), ("data/rule_pack_marker.ron", text("()"))]);
        let mut s = server(&net, "refuse-server", std::slice::from_ref(&rules));
        // The browser rows say "modded" and name the mod.
        let info = tt_engine::netgame::info::server_info(&s.host);
        assert_eq!(info.mods.len(), 1);
        assert_eq!(info.mods[0].label(), "Rule Pack 1.0");
        let decoded = tt_net::ServerInfo::decode(&info.encode()).unwrap();
        assert!(decoded.modded);
        let mut t = 0u64;
        // Without the mod: refused before any data is loaded, naming it.
        let mut c = client(&net, "refuse-client", &[], &[], 2);
        let err = join(&net, &mut t, &mut s, &mut c);
        assert!(err.contains("\"Rule Pack 1.0\" is missing"), "{err}");
        assert!(c.host.game.is_none());
        // Installed but disabled: told to enable it.
        let c2 = modded_engine("refuse-disabled", std::slice::from_ref(&rules), &[], &[]);
        let err = tt_engine::mods::check_server_mods(&c2.host, &info.mods).unwrap_err();
        assert!(err.contains("enable the mod \"Rule Pack 1.0\""), "{err}");
        // A gameplay mod the server does not run: told to disable it.
        let err = tt_engine::mods::check_server_mods(&c2.host, &[]);
        assert!(err.is_ok(), "a disabled mod is not used");
        let c3 = modded_engine("refuse-extra", std::slice::from_ref(&rules), &["rules"], &[]);
        let err = tt_engine::mods::check_server_mods(&c3.host, &[]).unwrap_err();
        assert!(err.contains("disable the mod \"Rule Pack 1.0\""), "{err}");
        // With the same mod: in the game.
        let mut ok = client(&net, "refuse-same", std::slice::from_ref(&rules), &["rules"], 3);
        assert_eq!(join(&net, &mut t, &mut s, &mut ok), "");
    });
}

#[test]
fn cosmetic_mods_may_differ() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let net = MemNet::new(12);
        net.set_conditions(LinkConditions { latency: Duration::from_millis(20), ..Default::default() });
        let skins: ModSpec = ("skins", vec![("mod.ron", text("(name: \"Credits Skin\")")), ("ui/credits.txt", text("server credits"))]);
        let sounds: ModSpec = ("other", vec![("ui/credits.txt", text("client credits")), ("missions/tt1_1.png", text("not a picture"))]);
        let mut s = server(&net, "cosmetic-server", &[skins]);
        assert!(tt_engine::netgame::info::server_info(&s.host).mods.is_empty(), "cosmetic mods are not announced");
        let mut c = client(&net, "cosmetic-client", &[sounds], &["other"], 2);
        assert!(!c.host.fs.mods()[0].affects_gameplay, "a mission picture is cosmetic");
        let mut t = 0u64;
        assert_eq!(join(&net, &mut t, &mut s, &mut c), "");
        assert_eq!(c.host.cl.mission_file, MISSION);
    });
}
