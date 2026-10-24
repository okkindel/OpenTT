//! The golden scenarios (plan §4.2): the difftest scenarios
//! (`tests/gameplay_diff.rs`) in the shipped configuration, the solo
//! level select and K/D/A fight, the Power-up Test arena, the health-pack
//! minimum, network play and boot / connection errors.
//!
//! See `tests/golden/README`.

use super::action::{Aim, Bot, Cheat, ConnEvent, Mode, MsgTo, PrintKind, ScenarioAction as A, Who};
use super::run::{GoldenScenario as Sc, NetScenario};
use tt_gameplay::HudMsg as ClientMsg;
use tt_menu::{BotSetup, BotSkill, CustomGame, MenuAction, Team};
use tt_net::LinkConditions;

pub enum Entry {
    Local(Sc),
    Net(NetScenario),
}

impl Entry {
    pub fn name(&self) -> &str {
        match self {
            Entry::Local(s) => &s.name,
            Entry::Net(s) => &s.name,
        }
    }

    /// The seed written to the MANIFEST.
    pub fn seed(&self) -> u64 {
        match self {
            Entry::Local(s) => s.seed as u64,
            Entry::Net(s) => s.seed,
        }
    }

    pub fn run(&self) -> Result<Vec<String>, String> {
        match self {
            Entry::Local(s) => super::run::run(s),
            Entry::Net(s) => super::run::run_netplay(s),
        }
    }
}

fn s(x: &str) -> String {
    x.to_string()
}

fn mission(m: &str) -> String {
    tt_assets::layout::mission(m)
}

fn ffa_bots() -> Vec<Bot> {
    vec![
        Bot::new("GoldHeavyTank", "Hum [Bot]", "GreenTeam"),
        Bot::new("SilverMediumTank", "Tom [Bot]", "BlueTeam"),
        Bot::new("BronzeLightTank", "Mal [Bot]", "GreenTeam"),
        Bot::new("GoldLightTank", "Ri [Bot]", "BlueTeam"),
        Bot::new("SilverMediumTank", "Gul [Bot]", "GreenTeam"),
    ]
}

/// Drive and shoot; SPACE now and then (respawn after a death) – the
/// difftest's `play`.
fn play(sc: Sc) -> Sc {
    let mut sc = sc.at(3_000, A::key("w", true));
    let ms = sc.ms;
    let mut t = 4_000;
    while t < ms {
        sc = sc.tap(t, "space");
        sc = sc.at(t + 500, A::RespawnIfPlayer);
        t += 9_000;
    }
    sc.every(5_000, 2_000, A::Fire)
}

fn custom(m: &str, mode: Mode, bots: Vec<Bot>, minutes: f32, tank: u32) -> A {
    A::StartCustom { mission: m.into(), mode, bots, minutes, tank }
}

fn modes_on(out: &mut Vec<Entry>, map: &str, seed: u32) {
    for (i, (name, mode)) in [("battle", Mode::Battle), ("team-battle", Mode::TeamBattle), ("scrum", Mode::Scrum), ("team-scrum", Mode::TeamScrum)].into_iter().enumerate() {
        let n = format!("{name}-{}", map.to_ascii_lowercase());
        out.push(Entry::Local(play(Sc::new(&n, seed + i as u32, 180_000).setup(custom(map, mode, ffa_bots(), 1.5, i as u32 % 3)))));
    }
}

fn effects(name: &str, map: &str, mode: Mode, seed: u32) -> Sc {
    let sc = Sc::new(name, seed, 120_000)
        .setup(A::pref("server.replace_bots", "1"))
        .setup(A::pref("server.max_chat_len", "20"))
        .setup(custom(map, mode, ffa_bots(), 1.5, 1));
    play(sc)
        .every(8_000, 6_000, A::Cheat(Cheat::Pickups))
        .every(9_000, 5_000, A::Cheat(Cheat::KillOne))
        .at(12_000, A::Chat { text: s("hello everybody, this is long"), team: false })
        .at(12_000, A::Chat { text: s("team!"), team: true })
        .at(14_000, A::TogglePlayerAI)
        .at(14_000, A::SpawnPlayer)
        .at(20_000, A::Connect { conn: s("TestClient2"), name: s("Tester"), team: s(""), tank: 2 })
        .at(30_000, A::Connect { conn: s("TestClient3"), name: s("  "), team: s("GreenTeam"), tank: 3 })
        .at(30_000, A::Rename { conn: s("TestClient3"), name: s("Tester") })
        .at(40_000, A::Drop { conn: s("TestClient2"), reason: s("leaving") })
        .at(50_000, A::Drop { conn: s("TestClient3"), reason: s("leaving") })
}

fn solo(sc: Sc) -> Sc {
    sc.setup(A::SoloStart)
}

fn client_scenario(name: &str, map: &str, mode: Mode, seed: u32) -> Sc {
    let sc = Sc::new(name, seed, 90_000)
        .setup(A::pref("server.replace_bots", "1"))
        // The scenario turns the turret with keys and the mouse.
        .setup(A::SetAim(Aim::Classic))
        .setup(custom(map, mode, ffa_bots(), 1.0, 1));
    let long = "this is a rather long chat line that has to be wrapped over several lines of the messages hud";
    let mut sc = play(sc)
        // Chat typed into the message HUD (u = all), team chat.
        .tap(6_000, "u")
        .typing(6_100, "hello there")
        .tap(6_200, "return")
        .tap(7_000, "return")
        .typing(7_100, long)
        .tap(7_200, "return")
        .at(8_000, A::OpenChat { team: true })
        .typing(8_100, "team only")
        .tap(8_200, "return")
        .tap(9_000, "slash")
        .tap(9_100, "escape")
        // Messages with sounds arriving, server messages.
        .at(10_000, A::ClientChat { sender: s("0"), pitch: s("1.5"), text: s("beep~wdryFire.wav") })
        .at(10_000, A::ClientChat { sender: s("0"), pitch: s("3"), text: s("none~wmissing") })
        .at(10_000, A::Message { to: MsgTo::Client(Who::Me), kind: s("MsgTest"), text: s("server says hi~wdryFire.wav"), args: vec![] })
        .at(10_500, A::Chat { text: s("via the server"), team: false })
        .at(10_500, A::Chat { text: s("team via the server"), team: true })
        .every(11_000, 7_000, A::Cheat(Cheat::KillOne))
        // Prints with and without timeouts, replaced before they expire.
        .at(12_000, A::Print { to: Some(Who::Me), kind: PrintKind::Center, text: s("center one"), secs: s("2"), lines: s("1") })
        .at(12_000, A::Print { to: Some(Who::Me), kind: PrintKind::Bottom, text: s("bottom one"), secs: s("3"), lines: s("2") })
        .at(13_000, A::ClientPrint { kind: PrintKind::Center, text: s("center two"), secs: s("0"), lines: s("3") })
        .at(13_000, A::ClientPrint { kind: PrintKind::Bottom, text: s("bottom two"), secs: s("1.5"), lines: s("1") })
        .at(17_000, A::ClientClearPrints)
        .at(17_000, A::Print { to: None, kind: PrintKind::Center, text: s("all"), secs: s("1"), lines: s("2") })
        // Message paging keys.
        .tap(20_500, "pageUp")
        .tap(20_600, "pageUp")
        .tap(20_700, "pageDown")
        // Keys, mouse axes and buttons, both sensitivity curves, inverted mouse.
        .tap(22_000, "a")
        .at(22_100, A::key("d", true))
        .at(23_000, A::key("d", false))
        .tap(23_100, "numpad4")
        .tap(23_200, "numpad8")
        .tap(23_300, "numpad2")
        .tap(23_400, "left")
        .tap(23_500, "down")
        .at(24_000, A::Mouse { dx: 12.0, dy: -5.0 })
        .at(24_032, A::Mouse { dx: -300.0, dy: 400.0 })
        .at(24_064, A::Mouse { dx: 0.5, dy: 0.25 })
        .at(24_100, A::pref("input.mouse_sensitivity", "0.1"))
        .at(24_100, A::pref("input.invert_mouse", "1"))
        .at(24_200, A::Mouse { dx: 7.0, dy: 9.0 })
        .at(24_300, A::pref("input.mouse_sensitivity", "0.625"))
        .at(24_300, A::pref("input.invert_mouse", "0"))
        .at(24_300, A::pref("camera.fov", "60"))
        .at(24_400, A::Mouse { dx: -3.0, dy: 2.0 })
        .at(24_500, A::Button { button: 0, down: true })
        .at(24_532, A::Button { button: 0, down: false })
        .at(24_600, A::Button { button: 1, down: true })
        .at(24_632, A::Button { button: 1, down: false });
    // The binding functions directly.
    for (t, f, v) in [
        (25_000, "turnLeft", "1"),
        (25_000, "turnRight", "1"),
        (25_000, "panUp", "1"),
        (25_000, "panDown", "1"),
        (25_000, "moveup", "1"),
        (25_000, "movedown", "1"),
        (25_000, "setSpeed", "0"),
        (25_000, "setSpeed", "2"),
        (25_100, "turnLeft", "0"),
        (25_100, "turnRight", "0"),
        (25_100, "panUp", "0"),
        (25_100, "panDown", "0"),
        (25_100, "moveup", "0"),
        (25_100, "movedown", "0"),
        (25_100, "moveleft", "0.5"),
        (25_100, "moveright", "0"),
        (25_100, "moveforward", "0"),
        (25_100, "movebackward", "1"),
        (25_200, "movebackward", "0"),
        (25_200, "moveleft", "0"),
        // Free look, first person, camera, escape.
        (26_000, "toggleFreeLook", "1"),
        (26_000, "toggleFirstPerson", "1"),
        (26_000, "toggleCamera", "1"),
        (26_000, "toggleCamera", "0"),
        (27_000, "toggleFreeLook", "0"),
        (27_000, "toggleFirstPerson", "1"),
        (27_000, "toggleFirstPerson", "0"),
    ] {
        sc = sc.at(t, A::bind(f, Some(v)));
    }
    sc.at(27_000, A::bind("escapeFromGame", None))
        .tap(27_500, "escape")
        .at(28_000, A::Menu(MenuAction::Resume))
        // A second player joins and leaves; the player list.
        .at(30_000, A::Connect { conn: s("TestClient2"), name: s("Second"), team: s("GreenTeam"), tank: 2 })
        .at(30_000, A::TogglePlayerList)
        .at(30_000, A::TogglePlayerList)
        .at(32_000, A::ClientGameStart)
        .at(36_000, A::Drop { conn: s("TestClient2"), reason: s("bye") })
        // Solo HUD widgets and the clock.
        .at(43_500, A::ClientHud(ClientMsg::Lives(s("2"))))
        .at(43_500, A::ClientHud(ClientMsg::Lives(s("7"))))
        .at(43_500, A::ClientHud(ClientMsg::Score(s("1234567"))))
        .at(43_500, A::ClientHud(ClientMsg::Score(s("42"))))
        .at(43_500, A::ClientHud(ClientMsg::BrainsMax(s("12"))))
        .at(43_500, A::ClientHud(ClientMsg::BrainsCount(s("3"))))
        .at(43_500, A::ClientHud(ClientMsg::BrainsCount(s("11"))))
        .at(43_500, A::ClientHud(ClientMsg::BrainsMax(s("5"))))
        .at(43_500, A::ClientHud(ClientMsg::BrainsCount(s("3"))))
        .at(43_500, A::ClientSyncClock { secs: s("30"), dir: s("1") })
        .at(43_500, A::ClientSyncClock { secs: s("12"), dir: s("0") })
}

fn menu_bot(name: &str, skill: BotSkill, tank: usize, team: Team) -> BotSetup {
    BotSetup { name: name.into(), skill, tank, team }
}

/// Netplay client input (`tests/netplay_game.rs` `drive`): forward,
/// turning every 90 ticks, a shot every 30 ticks, a respawn request every
/// 125 ticks, from `from` ms.
fn net_drive(i: usize, from: u32, ms: u32) -> Vec<(u32, A)> {
    let mut v = vec![(0, A::Menu(MenuAction::Join { address: s("10.0.0.1:28000"), password: s(""), team: Team::Auto }))];
    let ticks = (ms - from) / crate::TICK_MS;
    for tick in 0..ticks as u64 {
        let t = from + tick as u32 * crate::TICK_MS;
        let turn = if (tick / 90 + i as u64).is_multiple_of(2) { 1.0 } else { 0.0 };
        v.push((t, A::Move { forward: 1.0, right: turn }));
        if tick % 30 == i as u64 * 7 {
            v.push((t, A::Fire));
        }
        if tick % 125 == 3 {
            v.push((t, A::RequestSpawn));
        }
    }
    v
}

/// Every golden scenario, in fixture order.
pub fn all() -> Vec<Entry> {
    let mut out = Vec::new();
    out.push(Entry::Local(play(Sc::new("quick-tt1_1", 11, 120_000).setup(A::QuickPlay { mission: s("tt1_1") }))));
    modes_on(&mut out, "tt1_1", 100);
    modes_on(&mut out, "tt2_3", 200);
    modes_on(&mut out, "tt3_1", 300);
    modes_on(&mut out, "tt2_1", 400);
    // Bot replacement.
    out.push(Entry::Local(
        play(Sc::new("bots-tt1_2", 13, 60_000).setup(custom("tt1_2", Mode::TeamBattle, ffa_bots(), 5.0, 1)))
            .at(10_000, A::DropBot { index: 0 })
            .at(10_000, A::UpdateBotCount)
            .at(20_000, A::DropBot { index: 2 })
            .at(20_000, A::DropBot { index: 1 })
            .at(20_000, A::UpdateTeamBotCount)
            .at(20_000, A::UpdateBotCount)
            .at(30_000, A::AddFakeClient { name: s("FakeClient1"), team: s("BlueTeam") })
            .at(30_000, A::UpdateBotCount)
            .at(35_000, A::AddFakeClient { name: s("FakeClient2"), team: s("GreenTeam") })
            .at(35_000, A::UpdateBotCount)
            .at(35_000, A::UpdateTeamBotCount)
            .at(45_000, A::RemoveFakeClient { name: s("FakeClient1") })
            .at(45_000, A::RemoveFakeClient { name: s("FakeClient2") })
            .at(45_000, A::UpdateBotCount)
            .at(45_000, A::UpdateBotCount),
    ));
    // Solo campaign: level ends and the next levels.
    {
        let mut sc = play(solo(Sc::new("solo", 17, 240_000)));
        let mut t = 30_000;
        while t < 240_000 {
            sc = sc.at(t, A::Cheat(Cheat::EndLevel)).at(t + 8_000, A::SoloNextLevel);
            t += 45_000;
        }
        out.push(Entry::Local(sc));
    }
    out.push(Entry::Local(play(Sc::new("target", 19, 90_000).setup(A::TargetPractice)).every(10_000, 7_000, A::Cheat(Cheat::KillTarget))));
    out.push(Entry::Local(Sc::new("demo", 29, 60_000).setup(A::DemoServer { secs: 40 })));
    out.push(Entry::Local(play(Sc::new("launch-quick", 3, 30_000).setup(A::LaunchQuickServer))));
    out.push(Entry::Local(play(
        Sc::new("launch-mp", 37, 40_000)
            .setup(A::SetBots(vec![Bot::new("GoldLightTank", "A", ""), Bot::new("BronzeHeavyTank", "B", ""), Bot::new("SilverMediumTank", "C", "GreenTeam")]))
            .setup(A::RoundLength(24.0))
            .setup(A::LaunchListenServer { mission: mission("tt1_3"), mission_type: s("Scrum"), teams: true })
            .setup(A::SimLag { loss: 0, lag_ms: 100 }),
    )));
    out.push(Entry::Local(
        Sc::new("launch-ded", 43, 30_000)
            .setup(A::DedicatedMode(s("TS")))
            .setup(A::SetBots(vec![Bot::new("GoldLightTank", "", ""), Bot::new("GoldHeavyTank", "", "")]))
            .setup(A::LaunchDedicated { arg: s("GREEN") })
            .at(3_000, A::CycleGame)
            .at(12_000, A::DedicatedMode(s("S")))
            .at(12_000, A::LaunchDedicated { arg: s("ALL") })
            .at(12_000, A::CycleGame)
            .at(20_000, A::DedicatedMode(s("")))
            .at(20_000, A::LaunchDedicated { arg: mission("tt2_2") })
            .at(20_000, A::CycleGame),
    ));
    out.push(Entry::Local(effects("fx-team-battle-tt1_1", "tt1_1", Mode::TeamBattle, 501)));
    out.push(Entry::Local(effects("fx-battle-tt2_3", "tt2_3", Mode::Battle, 502)));
    out.push(Entry::Local(effects("fx-team-scrum-tt3_1", "tt3_1", Mode::TeamScrum, 503)));
    // Solo kills in quick succession, brains, level ends.
    {
        let mut sc = play(solo(Sc::new("fx-solo", 601, 200_000))).every(6_000, 4_000, A::Cheat(Cheat::KillOne)).every(6_000, 4_000, A::Cheat(Cheat::KillOne)).every(6_000, 4_000, A::Cheat(Cheat::BrainsToPlayer));
        let mut t = 25_000;
        while t < 200_000 {
            sc = sc.at(t, A::Cheat(Cheat::EndLevel)).at(t + 8_000, A::SoloNextLevel);
            t += 30_000;
        }
        out.push(Entry::Local(sc));
    }
    out.push(Entry::Local(play(Sc::new("fx-target", 602, 60_000).setup(A::TargetPractice)).every(5_000, 2_000, A::Cheat(Cheat::KillOne))));
    // Server helpers: flood protection, team messages, prints, sounds,
    // camera modes, kick and ban, a mission load and a reset.
    {
        let mut sc = play(
            Sc::new("fx-server-tt1_3", 701, 90_000)
                .setup(A::SetBots(vec![Bot::new("GoldLightTank", "A", ""), Bot::new("BronzeHeavyTank", "B", ""), Bot::new("SilverMediumTank", "C", "")]))
                .setup(A::RoundLength(600.0))
                .setup(A::LaunchListenServer { mission: mission("tt1_3"), mission_type: s("Deathmatch"), teams: true }),
        );
        for i in 0..7 {
            sc = sc.at(8_000, A::Chat { text: format!("spam {i}"), team: false });
        }
        sc = sc
            .at(9_000, A::Message { to: MsgTo::Team(s("BlueTeam")), kind: s("MsgTest"), text: s("team %1"), args: vec![s("1")] })
            .at(9_000, A::Message { to: MsgTo::TeamOf(Who::Me), kind: s("MsgTest"), text: s("team %1"), args: vec![s("2")] })
            .at(9_000, A::Message { to: MsgTo::TeamExcept(Who::Me), kind: s("MsgTest"), text: s("x"), args: vec![] })
            .at(9_000, A::Message { to: MsgTo::AllExcept(Who::Me), kind: s("MsgTest"), text: s("y"), args: vec![] })
            .at(10_000, A::Print { to: None, kind: PrintKind::Center, text: s("c"), secs: s("2"), lines: s("5") })
            .at(10_000, A::Print { to: None, kind: PrintKind::Bottom, text: s("b"), secs: s("2"), lines: s("") })
            .at(10_000, A::Print { to: Some(Who::Me), kind: PrintKind::Center, text: s("one"), secs: s("1"), lines: s("2") })
            .at(10_000, A::Print { to: Some(Who::Me), kind: PrintKind::Bottom, text: s("two"), secs: s("1"), lines: s("0") })
            .at(10_000, A::ClearPrints { to: Some(Who::Me) })
            .at(10_000, A::ClearPrints { to: None })
            .at(11_000, A::ServerSound { profile: s("PupOnSound"), at: None })
            .at(11_000, A::ServerSound { profile: s("PupOffSound"), at: Some(s("0 0 100 1 0 0 0")) })
            .at(13_000, A::CameraModes)
            .at(20_000, A::Connect { conn: s("TestClient4"), name: s("Kicked"), team: s(""), tank: 1 })
            .at(20_000, A::Connect { conn: s("TestClient5"), name: s("Banned"), team: s("BlueTeam"), tank: 0 })
            .at(25_000, A::Kick { conn: s("TestClient4") })
            .at(25_000, A::Ban { conn: s("TestClient5") })
            .at(30_000, A::LoadMission { mission: mission("tt1_4") })
            .at(60_000, A::ResetMission);
        out.push(Entry::Local(sc));
    }
    out.push(Entry::Local(client_scenario("client-team-tt1_1", "tt1_1", Mode::TeamBattle, 801)));
    out.push(Entry::Local(client_scenario("client-ffa-tt2_3", "tt2_3", Mode::Battle, 802)));
    // Boot and connection errors, disconnect, a second game.
    {
        let mut sc = Sc::new("boot-connection", 901, 70_000).setup(A::QuickPlay { mission: s("tt1_1") });
        let mut ev = vec![ConnEvent::ErrorMessage(s("Bad thing")), ConnEvent::Error(s("x1")), ConnEvent::TimedOut, ConnEvent::Dropped(s("bye")), ConnEvent::RequestTimedOut];
        for r in ["CR_INVALID_PROTOCOL_VERSION", "CR_INVALID_CONNECT_PACKET", "CR_YOUAREBANNED", "CR_SERVERFULL", "CHR_PROTOCOL", "CHR_CLASSCRC", "CHR_INVALID_CHALLENGE_PACKET", "SOMETHING_ELSE"] {
            ev.push(ConnEvent::Rejected(s(r)));
        }
        ev.push(ConnEvent::PasswordRejected(s("")));
        ev.push(ConnEvent::PasswordRejected(s("pw")));
        for e in ev {
            sc = sc.at(5_000, A::Connection(e));
        }
        sc = sc
            .at(6_000, A::Connection(ConnEvent::LagIcon(true)))
            .at(6_000, A::Connection(ConnEvent::Accepted))
            .at(8_000, A::ClientLoadInfo { name: s("Some Map"), lines: vec![s("a"), s(""), s("c")] })
            .at(30_000, A::Disconnect)
            .at(32_000, A::QuickPlay { mission: s("tt2_3") })
            .at(60_000, A::Disconnect);
        out.push(Entry::Local(play(sc)));
    }
    // Solo level select at positions 1, 7 and 20, then a game over into
    // the high scores.
    for pos in [1u32, 7, 20] {
        out.push(Entry::Local(play(Sc::new(&format!("solo-select-{pos}"), 1000 + pos, 60_000).setup(A::Menu(MenuAction::StartSoloAt(pos)))).at(45_000, A::Cheat(Cheat::SoloGameOver(s("420000"))))));
    }
    // K/D/A in a scripted team fight (kill 2, assist 1, team kill).
    {
        let g = CustomGame {
            mission: mission("tt1_1"),
            mode: 1,
            bots: vec![menu_bot("Mate", BotSkill::Bronze, 0, Team::Blue), menu_bot("Foe", BotSkill::Bronze, 0, Team::Green), menu_bot("Foe2", BotSkill::Bronze, 0, Team::Green)],
            player_tank: 1,
            player_team: Team::Blue,
            minutes: 5,
            replace_bots: true,
        };
        let dmg = |v: Who, a: Who, amount: f32| A::Cheat(Cheat::Damage { victim: v, attacker: a, amount });
        let sc = Sc::new("kda-fight", 1101, 40_000)
            .setup(A::Menu(MenuAction::StartCustom(g)))
            .tap(4_000, "space")
            .at(10_000, dmg(Who::Bot(1), Who::Me, 1.0))
            .at(10_032, dmg(Who::Bot(1), Who::Bot(0), 1000.0))
            .at(14_000, dmg(Who::Bot(0), Who::Bot(2), 1.0))
            .at(14_032, dmg(Who::Bot(0), Who::Me, 1000.0))
            .at(18_000, dmg(Who::Me, Who::Bot(2), 1000.0))
            .every(20_000, 3_000, A::RespawnIfPlayer)
            .tap(22_000, "space");
        out.push(Entry::Local(sc));
    }
    // The Power-up Test arena (assets overlay), battle and scrum: every
    // pick-up, the reload arch, heal after damage, the pads, then the
    // Mortar and Scatter shot pick-ups.
    for (mode, seed) in [(0u32, 1201u32), (2, 1202)] {
        let g = CustomGame { mission: mission("poweruptest"), mode, bots: Vec::new(), player_tank: 1, player_team: Team::Green, minutes: 10, replace_bots: true };
        let mut sc = Sc::new(&format!("powerup-test-{mode}"), seed, 116_000).setup(A::SetAim(Aim::Classic)).setup(A::Menu(MenuAction::StartCustom(g)));
        let mut t = 2_000;
        while t < 8_000 {
            sc = sc.tap(t, "space");
            t += 1_920;
        }
        let mut t = 10_000;
        for name in ["PupSpeedy", "PupBounce", "PupSplash", "PupReload", "PupWeapon", "PupHeal", "BoostWest", "BoostEast", "JumpWest", "JumpEast", "PupMortar", "PupScatter"] {
            sc = sc.at(t, A::Cheat(Cheat::Teleport { trigger: s(name), back: if name.starts_with("Pup") { 10.0 } else { 8.0 } }));
            if name == "PupHeal" {
                sc = sc.at(t + 2_600, A::Cheat(Cheat::Hurt(8.0)));
            }
            sc = sc.at(t + 2_700, A::key("w", true)).at(t + 6_000, A::key("w", false));
            t += 8_000;
        }
        out.push(Entry::Local(sc));
    }
    // Health-pack minimum: a bot free-for-all through the menu.
    {
        let bots = vec![menu_bot("B1", BotSkill::Gold, 0, Team::Auto), menu_bot("B2", BotSkill::Silver, 1, Team::Auto), menu_bot("B3", BotSkill::Bronze, 2, Team::Auto), menu_bot("B4", BotSkill::Gold, 2, Team::Auto)];
        let g = CustomGame { mission: mission("tt2_2"), mode: 0, bots, player_tank: 0, player_team: Team::Auto, minutes: 10, replace_bots: true };
        out.push(Entry::Local(play(Sc::new("health-floor-tt2_2", 1301, 180_000).setup(A::Menu(MenuAction::StartCustom(g))))));
    }
    // Network play: dedicated server (6 bots + 1) and two clients.
    let extra_bot = vec![A::SetLastBotSlot { index: 6, tank: s("SilverMediumTank"), name: s("Extra [Bot]"), team: s("BlueTeam") }, A::UpdateBotCount];
    out.push(Entry::Net(NetScenario {
        name: s("netplay-lan"),
        seed: 11,
        cond: LinkConditions::lan(),
        server_args: vec![],
        mission: mission("tt1_1"),
        server_setup: extra_bot.clone(),
        clients: (0..2).map(|i| net_drive(i, 8_000, 60_000)).collect(),
        ms: 60_000,
    }));
    out.push(Entry::Net(NetScenario {
        name: s("netplay-dsl-scrum"),
        seed: 12,
        cond: LinkConditions::dsl(),
        server_args: vec![s("-game"), s("S")],
        mission: mission("tt1_1"),
        server_setup: extra_bot,
        clients: (0..2).map(|i| net_drive(i, 8_000, 45_000)).collect(),
        ms: 45_000,
    }));
    out
}

/// The scenario called `name`.
pub fn by_name(name: &str) -> Option<Entry> {
    all().into_iter().find(|e| e.name() == name)
}
