
use tt_engine::gameplay::scoreboard::Kda;
use tt_engine::gameplay::server_host;
use tt_gameplay::server::ClientId;
use tt_engine::Engine;
use tt_menu::{BotSetup, BotSkill, CustomGame, MenuAction, MenuBackend, Team};

mod common;
use common::*;

fn until(e: &mut Engine, ms: u32, f: impl Fn(&Engine) -> bool) -> bool {
    for _ in 0..ms / tt_engine::TICK_MS {
        if f(e) {
            return true;
        }
        tick(e);
    }
    f(e)
}

#[test]
fn level_select_starts_with_the_played_through_difficulty() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = engine(&root, "levelselect", &[]);
        assert!(e.boot());
        backend_do(&mut e, |b| b.create_player("Picker").unwrap());
        // A new player has only the first level open.
        let s = backend_do(&mut e, |b| b.solo_status());
        assert!(!s.in_run);
        assert_eq!(s.levels.len(), 20);
        assert_eq!(s.levels.iter().filter(|l| l.unlocked).count(), 1);
        assert_eq!((s.levels[4].mission.as_str(), s.levels[4].level_type.as_str()), ("greenbrain", "Brain"));
        assert_eq!((s.levels[15].mission.as_str(), s.levels[15].cycle), ("tt1_1", 2));

        // Position 8, as a run that never lost reaches it (slot 6 bumped the rest).
        const POS: u32 = 8;
        let table = tt_engine::shell::levels(&mut e.host).unwrap();
        let mut played = tt_shell::SoloRun::new(table);
        let mut expected = None;
        for _ in 0..POS {
            expected = played.advance_level().cloned();
        }
        let expected = expected.unwrap();
        apply(&mut e, MenuAction::StartSoloAt(POS));
        assert!(until(&mut e, 60_000, |e| content(e) == "PlayGui"), "{}", content(&e));
        for _ in 0..30 {
            tick(&mut e);
        }
        assert!(e.server().session.mission_file.clone().contains(&format!("{}.ron", expected.mission)), "{} vs {}", e.server().session.mission_file.clone(), expected.mission);
        assert_eq!(e.server().campaign.mission_seq_total, Some(POS));
        assert_eq!(e.server().campaign.mission_seq, Some(played.mission_seq));
        assert_eq!(e.server().campaign.kind().map(|k| k.name()), Some(expected.level_type.as_str()));
        assert_eq!(e.server().campaign.level().bronze_points, expected.bronze_pts as f32);
        assert_eq!(e.server().campaign.level().boss_points, expected.boss_pts as f32);
        // setSPTankParams with the bumped scales: armor 1/armor_scale, fire 1/fire_scale, damage as is.
        for (i, (fld, want)) in [("damage_scale", expected.damage_scale), ("armor_scale", 1.0 / expected.armor_scale), ("fire_scale", 1.0 / expected.fire_scale)].into_iter().enumerate() {
            // The bronze light tank's own scale.
            let base = e.server().campaign.base_scales[0][i] as f64;
            let got = tt_engine::gameplay::server_host::tank_field(&e.host, "BronzeLightTank", fld);
            let got = got.parse::<f64>().unwrap_or_else(|_| panic!("{got:?}"));
            assert!((got - base * want).abs() < 1e-3, "{fld}: {got} vs {base} x {want}");
        }
        {
            let run = e.host.shell.run.as_ref().unwrap();
            assert_eq!((run.lives, run.score_value(), run.start_level, run.mission_seq_total), (3, 0.0, POS, POS));
        }
        // Progress is saved in its own file (progress.ron).
        let data = e.host.fs.data_dir().to_path_buf();
        let p = tt_shell::save::read_dir(&data).progress;
        assert_eq!(p.highest("Picker"), POS);
        let players = std::fs::read_to_string(data.join("players.ron")).unwrap_or_default();
        assert!(!players.contains("reached"));

        // Game over: the run enters the high scores with its start level.
        tt_engine::gameplay::server_host::with_local_record(&mut e.host, |r| {
            r.spscore = Some(420000.0);
            r.lives = Some(0);
        });
        spawn(&mut e);
        assert!(until(&mut e, 5_000, |e| content(e) == "soloGameOverGui"), "{}", content(&e));
        let hs = backend_do(&mut e, |b| b.high_scores());
        let mine = hs.iter().find(|h| h.name == "Picker").expect("in the top 10");
        assert_eq!((mine.score, mine.level, mine.start_level), (420000, POS, Some(POS)));
        assert!(hs.iter().filter(|h| h.name != "Picker").all(|h| h.start_level.is_none()));
        let table = tt_shell::save::read_dir(&data).hiscores;
        assert!(table.entries().unwrap().iter().any(|h| h.name == "Picker" && h.level == POS));

        // Reloaded from disk (a new session): the levels up to POS are open.
        e.host.shell.progress = tt_shell::Progress::default();
        tt_engine::shell::load_players(&mut e.host);
        e.host.shell.run = None;
        let s = backend_do(&mut e, |b| b.solo_status());
        assert_eq!(s.levels.iter().filter(|l| l.unlocked).count(), POS as usize);
        let _ = std::fs::remove_dir_all(&data);
    });
}

/// The tank of a client (`client.player`) in the world.
/// The local player's respawn request, run by the server now.
fn spawn(e: &mut Engine) {
    let c = server_host::local_client(&e.host).unwrap();
    server_host::client_cmd(&mut e.host, c, &tt_gameplay::ClientCmd::SpawnPlayer);
}

fn tank(e: &mut Engine, client: ClientId) -> tt_game::TankId {
    server_host::player_tank(&e.host, client).unwrap()
}

fn kda(e: &Engine, c: ClientId) -> Kda {
    server_host::kda(&e.host, c)
}

fn plus(a: Kda, k: u32, d: u32, s: u32) -> Kda {
    Kda { kills: a.kills + k, deaths: a.deaths + d, assists: a.assists + s }
}

#[test]
fn kda_in_a_scripted_team_fight() {
    tt_assets::require_game_data!();
    let root = assets();
    big_stack(move || {
        let mut e = engine(&root, "fight", &[]);
        assert!(e.boot());
        backend_do(&mut e, |b| b.create_player("Bait").unwrap());
        let bot = |name: &str, team| BotSetup { name: name.into(), skill: BotSkill::Bronze, tank: 0, team };
        let g = CustomGame { mission: "missions/tt1_1.ron".into(), mode: 1, bots: vec![bot("Mate", Team::Blue), bot("Foe", Team::Green), bot("Foe2", Team::Green)], player_tank: 1, player_team: Team::Blue, minutes: 5, replace_bots: true };
        apply(&mut e, MenuAction::StartCustom(g));
        assert!(until(&mut e, 60_000, |e| content(e) == "PlayGui"), "{}", content(&e));
        for _ in 0..60 {
            tick(&mut e);
        }
        let me = server_host::local_client(&e.host).unwrap();
        let bots = server_host::bots(&e.host);
        let team_of = |e: &mut Engine, c: ClientId| server_host::team_of(&e.host, c);
        let mine = team_of(&mut e, me);
        let mut mate = None;
        let mut foes = Vec::new();
        for b in bots {
            if team_of(&mut e, b) == mine {
                mate = Some(b);
            } else {
                foes.push(b);
            }
        }
        let (mate, foe, foe2) = (mate.expect("a team mate"), foes[0], foes[1]);
        let hit = |e: &mut Engine, victim: ClientId, attacker: ClientId, dmg: f32| {
            let (v, a) = (tank(e, victim), tank(e, attacker));
            e.host.game.as_mut().unwrap().world.apply_damage(v, dmg, Some(a));
        };
        let snap = |e: &Engine| [me, mate, foe, foe2].map(|c| kda(e, c));

        // 1. I hit Foe, Mate finishes it: Mate's kill, my assist, Foe's death.
        let score = |e: &mut Engine, c: ClientId| server_host::record(&e.host, c).and_then(|r| r.score).unwrap_or(0.0);
        let p0 = [me, mate, foe].map(|c| score(&mut e, c));
        let s0 = snap(&e);
        hit(&mut e, foe, me, 1.0);
        tick(&mut e);
        hit(&mut e, foe, mate, 1000.0);
        tick(&mut e);
        tick(&mut e);
        let s1 = snap(&e);
        assert_eq!(s1[0], plus(s0[0], 0, 0, 1), "me");
        assert_eq!(s1[1], plus(s0[1], 1, 0, 0), "mate");
        assert_eq!(s1[2], plus(s0[2], 0, 1, 0), "foe");
        // Score: a kill is worth 2, an assist 1, a death −1.
        let p1 = [me, mate, foe].map(|c| score(&mut e, c));
        assert_eq!((p1[0] - p0[0], p1[1] - p0[1]), (1.0, 2.0), "assist +1, kill +2");
        assert!(p1[2] <= p0[2], "a death never adds score");

        // 2. A team kill: Foe2 hit Mate, then I kill Mate: Mate's death only.
        for _ in 0..10 {
            tick(&mut e);
        }
        let s1 = snap(&e);
        hit(&mut e, mate, foe2, 1.0);
        tick(&mut e);
        hit(&mut e, mate, me, 1000.0);
        tick(&mut e);
        tick(&mut e);
        let s2 = snap(&e);
        assert_eq!(s2[0], s1[0], "me: no kill for a team mate");
        assert_eq!(s2[1], plus(s1[1], 0, 1, 0), "mate");
        assert_eq!(s2[3], s1[3], "foe2: no assist on a team kill");

        // 3. Foe2 kills me: my death, its kill.
        hit(&mut e, me, foe2, 1000.0);
        tick(&mut e);
        tick(&mut e);
        let s3 = snap(&e);
        assert_eq!(s3[0], plus(s2[0], 0, 1, 0), "me");
        assert_eq!(s3[3], plus(s2[3], 1, 0, 0), "foe2");

        // The player list rows show them: icon, name, score, K, D, A, cumScore.
        for c in [me, mate, foe, foe2] {
            let row = e.host.client.players.players.text(&c.text()).unwrap_or_default().to_string();
            let f: Vec<&str> = row.split('\t').collect();
            let k = kda(&e, c);
            assert_eq!(f.len(), 7, "{row:?}");
            assert_eq!(&f[3..6], &[k.kills.to_string(), k.deaths.to_string(), k.assists.to_string()], "{row:?}");
        }
        // The round results get the rows; a new round starts from zero.
        server_host::with_server(&mut e.host, |s, b| s.end_round(b));
        assert!(until(&mut e, 5_000, |e| content(e) == "EndGameGui"));
        let rows: Vec<String> = e.host.client.end_game.players.rows.iter().map(|r| r.text.clone()).collect();
        assert!(rows.iter().any(|r| r.contains("Bait") && r.split('\t').nth(4) == Some(&s3[0].deaths.to_string()[..])), "{rows:?}");
        tick(&mut e);
        e.host.srv.state_mut().round.running = Some(true);
        tick(&mut e);
        assert!([me, mate, foe, foe2].iter().all(|&c| kda(&e, c) == Kda::default()), "{:?}", snap(&e));
        let _ = std::fs::remove_dir_all(e.host.fs.data_dir());
    });
}
