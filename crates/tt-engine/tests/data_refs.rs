//! The game data's references: every name a definition gives (a tank's
//! driver and cartridge, a projectile's explosions, a power-up's sounds,
//! the power-up weapons, ...) is declared, with the server's data alone
//! and with each mission's own defs.

use tt_engine::gameplay::defs;

mod common;
use common::*;

#[test]
fn every_reference_names_a_definition() {
    tt_assets::require_game_data!();
    let root = assets();
    let mut e = engine(&root, "data-refs", &[]);
    assert!(e.boot());
    defs::load_server_data(&mut e.host);
    let d = &e.host.data;
    assert!(d.errors.is_empty(), "{}", d.errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"));
    assert!(d.tanks.len() >= 18 && d.projectiles.len() >= 10 && d.pickups.len() >= 5, "{} tanks", d.tanks.len());
    assert_eq!(d.weapons.len(), 9);
    let medium = d.tanks.named("MediumTank").unwrap();
    assert!(medium.default_projectile.is_some_and(|p| d.projectiles[p].name.eq_ignore_ascii_case("DefaultProjectile")));
    assert_eq!(medium.ai.map(|a| d.ai[a].name.as_str()), Some("GoldMediumAI"));
    let mut missions: Vec<_> = std::fs::read_dir(root.join("missions")).unwrap().flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|x| x == "ron")).collect();
    missions.sort();
    for m in missions {
        let rel = format!("missions/{}", m.file_name().unwrap().to_string_lossy());
        if !tt_engine::mission::exec_mission(&mut e.host, &rel) {
            continue;
        }
        let d = &e.host.data;
        assert!(d.errors.is_empty(), "{rel}: {}", d.errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"));
        tt_engine::mission::delete_mission(&mut e.host);
    }
}
