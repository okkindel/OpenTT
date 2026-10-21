//! The world's life cycle and the simulation tick.

use super::aim::reverse_steering;
use super::cosmetic::{cosmetic_event, update_engine_sounds};
use super::{aim_mode, parse_transform, GameSim, PlayerAim, Role};
use crate::host::EngineHost;
use tt_game::fields::Defs;
use tt_game::{Event, TankId};
use tt_gameplay::server::combat::KdaEvent;
use tt_gameplay::{EntityId, GameEvent};

fn label(e: &Event) -> &'static str {
    match e {
        Event::Fired { .. } => "Fired",
        Event::DryFired { .. } => "DryFired",
        Event::ProjectileBounced { .. } => "ProjectileBounced",
        Event::ProjectileExploded { .. } => "ProjectileExploded",
        Event::ProjectileRemoved { .. } => "ProjectileRemoved",
        Event::Damaged { .. } => "Damaged",
        Event::Killed { .. } => "Killed",
        Event::TankRemoved { .. } => "TankRemoved",
        Event::Collided { .. } => "Collided",
        Event::FlagDropped { .. } => "FlagDropped",
        Event::PowerUpEntered { .. } => "PowerUpEntered",
        Event::PowerUpLeft { .. } => "PowerUpLeft",
        Event::PowerUpSwitched { .. } => "PowerUpSwitched",
        Event::PadUsed { .. } => "PadUsed",
        Event::PadStateChanged { .. } => "PadStateChanged",
        Event::VolcanoErupted { .. } => "VolcanoErupted",
        Event::FlagPickedUp { .. } => "FlagPickedUp",
        Event::FlagOutOfBounds { .. } => "FlagOutOfBounds",
        Event::BrainEjected { .. } => "BrainEjected",
        Event::UserThreadPlayed { .. } => "UserThreadPlayed",
    }
}

/// The native defs as the sim reads them.
pub fn native_defs(h: &EngineHost) -> Defs {
    let mut db = Defs::new();
    let scales = h.srv.solo_scales.as_ref();
    db.add(h.defs.objects().filter(|o| !o.name.is_empty()).map(|o| {
        let mut o = o.clone();
        // `setSPTankParams`: the solo campaign's scales of the tank defs.
        if let Some(sc) = scales {
            for (k, v) in &mut o.fields {
                if let Some(x) = sc.get(&o.name, k) {
                    *v = x.to_string();
                }
            }
        }
        o
    }));
    db
}

/// Builds or drops the world as `MissionGroup` comes and goes.
pub fn update_world(h: &mut EngineHost) {
    let mg = h.mission_load.as_ref().map(|l| l.group);
    match (mg, h.game.as_ref().map(|g| g.mission_group)) {
        (Some(m), Some(cur)) if m == cur => {}
        (Some(_), _) => {
            let dir = crate::mission::mission_dir(h);
            // The mission as its RON file declared it.
            let load = h.mission_load.clone().expect("the group is the loaded mission's");
            let db = native_defs(h);
            let first = h.srv.next_entity.max(h.game.as_ref().map_or(1, |g| g.entities.next_id()));
            match GameSim::build(h.fs.root(), &dir, &load, db, first) {
                Ok(mut g) => {
                    // The mission's balls (the Power-up Test's practice ball),
                    // then the tanks the server created before the world.
                    for (_, o) in &load.nodes {
                        if o.class.eq_ignore_ascii_case("Flag") {
                            let (pos, _) = parse_transform(o.get("position").unwrap_or(""));
                            g.add_flag(o.get("def").unwrap_or(""), &o.name, pos, None);
                        }
                    }
                    crate::gameplay::server_host::materialize_pending(h, &mut g);
                    h.srv.next_entity = g.entities.next_id();
                    let seed = h.rng.seed() as u32;
                    g.world.set_random_seed(seed);
                    // Network roles (single player and listen servers stay Local).
                    if crate::netgame::is_remote_client(h) && h.net.server.is_none() {
                        g.set_role(Role::Client);
                    } else if h.srv.get().is_some_and(|s| s.setup.dedicated) {
                        g.set_role(Role::Dedicated);
                    }
                    if g.role == Role::Client {
                        crate::netgame::world_rebuilt(&mut h.netgame);
                    }
                    // Initial power-up states are not switches (no sounds).
                    let _ = g.world.take_events();
                    h.log.info(&format!("Simulation: {} statics, {} triggers, terrain {}", g.world.statics.len(), g.world.triggers.len(), g.world.terrain.is_some()));
                    h.game = Some(g);
                }
                Err(e) => {
                    h.log.warn(&format!("simulation: {e}"));
                    h.game = None;
                }
            }
        }
        (None, Some(_)) => {
            let next = h.game.as_ref().map_or(1, |g| g.entities.next_id());
            h.srv.next_entity = h.srv.next_entity.max(next);
            h.game = None;
        }
        (None, None) => {}
    }
}

/// One simulation tick: moves in, world tick, events to scripts, poses out.
pub fn tick(h: &mut EngineHost) {
    update_world(h);
    let Some(mut g) = h.game.take() else { return };
    // The player's move (server-side control object of the local connection).
    let player_obj = if g.role == Role::Client { crate::netgame::control_object(h) } else { h.net.server.and_then(|s| h.net.control.get(&s).copied()) };
    let player = player_obj.and_then(|o| g.tank_of(o));
    // Camera aim: the mouse turns the free camera, not the turret.
    g.sync_free_aim(aim_mode(&h.shell.prefs), player);
    g.free_look_input(&mut h.client.moves);
    // Mouse aim this move consumes, as the server will see it (render
    // prediction only, see `GameSim::render_aim`).
    let mouse = {
        let (yaw, pitch) = h.client.moves.aim();
        let q = |a: f32| if g.world.quantize_moves { tt_game::netmove::u16_to_angle(tt_game::netmove::angle_to_u16(a)) } else { a };
        (q(yaw) / std::f32::consts::PI, q(pitch) / std::f32::consts::PI)
    };
    let mut mv = h.client.moves.take_move(reverse_steering(&h.shell.prefs), &mut g.last_trigger_count);
    g.player_aim = PlayerAim { tank: player.filter(|_| !g.world.ai_control_mode), consumed: mouse, pending: (0.0, 0.0) };
    if let Some(p) = player {
        if g.free_aim.tank == Some(p) {
            // Camera aim: key aiming turns the camera too; the move turns
            // the turret towards the camera's aim point, and the hull
            // towards it when it is beyond the turret's range.
            g.turn_free_aim(mv.yaw, mv.pitch);
            let target = g.free_view_at_tick(p).and_then(|v| g.turret_target(p, &v));
            if let (Some(tg), Some(t)) = (target, g.world.tank(p)) {
                let lim = 0.99 * std::f32::consts::PI;
                mv.yaw = ((tg.yaw_n - t.yaw_n) * std::f32::consts::PI).clamp(-lim, lim);
                mv.pitch = ((tg.pitch_n - t.pitch_n) * std::f32::consts::PI).clamp(-lim, lim);
                if tg.beyond != 0.0 && mv.x as i32 == 0 {
                    mv.x = tg.beyond;
                }
            }
            g.player_aim.tank = None;
        } else if tt_game::fmt::atoi(&h.shell.prefs.get("input.auto_turn")) != 0 {
            if let Some(t) = g.world.tank(p) {
                tt_game::tank::apply_auto_turn(&mut mv, t);
            }
        }
        if g.role == Role::Client {
            // A remote client's moves go to the server (no prediction yet).
            crate::netgame::push_move(&mut h.netgame, mv);
        } else {
            g.world.set_move(p, mv);
        }
    }
    // Remote clients' moves (server).
    if g.role.authority() {
        crate::netgame::apply_remote_moves(&mut h.netgame, &h.net, &mut g);
    }
    // Player tanks are a connection's control object: their corpses stay
    // until the client takes a new control object (respawn); then they
    // fade like a bot's.
    let controlled: std::collections::HashSet<TankId> = h.net.control.values().filter_map(|e| g.tank_of(*e)).collect();
    let flags: Vec<(TankId, bool)> = g.world.tanks.iter().map(|t| (t.id, t.controlled)).collect();
    for (t, was) in flags {
        let now = controlled.contains(&t);
        if was != now {
            g.world.set_controlling_client(t, now);
        }
    }
    g.world.ai_control_mode = h.srv.get().is_some_and(|s| s.setup.ai_control);
    g.world.team_game = h.srv.get().is_some_and(|s| s.setup.team_game);
    g.prev_pose = g.world.tanks.iter().map(|t| (t.id, (t.pos, t.rot))).collect();
    let was_corpse: std::collections::HashSet<TankId> = g.world.tanks.iter().filter(|t| t.is_corpse).map(|t| t.id).collect();
    g.prev_aim = g.world.tanks.iter().map(|t| (t.id, (t.yaw_n, t.pitch_n))).collect();
    g.prev_flags = g.world.flags.iter().map(|f| (f.id, f.pos)).collect();
    if g.role == Role::Client {
        // Puppets to the render time (their previous poses are saved above).
        crate::netgame::apply_states(&mut h.netgame, &mut g);
    }
    let mut events = g.world.take_events();
    events.extend(g.world.tick());
    if g.role.authority() {
        // The health power-up floor, before the clients see the tick.
        let respawned: Vec<tt_game::TriggerId> = events.iter().filter_map(|e| if let Event::PowerUpSwitched { trigger, on: true } = e { Some(*trigger) } else { None }).collect();
        let single = h.srv.get().is_some_and(|s| s.setup.single_player());
        crate::gameplay::health_packs::apply(&h.shell.prefs, single, &mut g, &respawned);
        crate::netgame::capture(&mut h.netgame, &g, &events);
    }
    // Per-tank render state of tanks that are gone.
    let live: std::collections::HashSet<TankId> = g.world.tanks.iter().map(|t| t.id).collect();
    g.last_fired.retain(|k, _| live.contains(k));
    g.last_coll_sound.retain(|k, _| live.contains(&k.0));
    g.cameras.retain(|k, _| live.contains(k));
    g.treads.retain(|k, _| live.contains(k));
    let projectiles: std::collections::HashSet<tt_game::ProjectileId> = g.world.projectiles.iter().map(|p| p.id).collect();
    g.brain_shapes.retain(|k, _| projectiles.contains(k));
    // Treads and wheels: each side's travel from the track edges'
    // displacement along the tank's forward axis; living tanks only (alive
    // before this tick, as processTick checks before it moves). Presentation
    // only.
    let cosmetic = g.role.cosmetic();
    for t in g.world.tanks.iter().filter(|_| cosmetic) {
        let Some(&old) = g.prev_pose.get(&t.id) else { continue };
        if !g.treads.contains_key(&t.id) {
            let Some(shape) = (!t.data.shape_file.is_empty()).then(|| tt_render::pose::shape(&tt_assets::resolve(&g.root, &g.mission_dir, &t.data.shape_file))).flatten() else { continue };
            g.treads.insert(t.id, crate::treads::Treads::new(crate::treads::TreadRig::from_shape(&shape)));
        }
        let alive = !was_corpse.contains(&t.id);
        if let Some(tr) = g.treads.get_mut(&t.id) {
            tr.tick(alive, old, (t.pos, t.rot));
        }
    }
    let mut game_events = Vec::new();
    if g.role.authority() {
        crate::gameplay::golden::record::on_world_events(h, &events);
        // K/D/A from the damage and kills.
        let kda: Vec<KdaEvent> = events
            .iter()
            .filter_map(|e| match e {
                Event::Damaged { tank, amount, source: Some(src) } if *amount > 0.0 && src != tank => Some(KdaEvent::Damaged { victim: g.entity_of_tank(*tank)?, source: g.entity_of_tank(*src)? }),
                Event::Killed { tank, killer } => Some(KdaEvent::Killed { victim: g.entity_of_tank(*tank)?, killer: killer.and_then(|k| g.entity_of_tank(k)) }),
                _ => None,
            })
            .collect();
        let now = g.world.time_ms;
        h.game = Some(g);
        crate::gameplay::scoreboard::track(h, &kda, now);
        g = h.game.take().expect("the world");
        game_events = GameEvent::of_world_events(&g.entities, &events);
    }
    for e in &events {
        *g.event_counts.entry(label(e)).or_default() += 1;
        if g.role.cosmetic() {
            cosmetic_event(h, &mut g, e, player);
        }
    }
    if g.role.cosmetic() {
        update_engine_sounds(h, &mut g);
    }
    g.known_triggers = g.world.triggers.iter().map(|t| t.id).collect();
    let known = &g.known_triggers;
    g.activations.retain(|k, _| known.contains(k));
    // Pickup hints for the player's tank.
    let mut hints: Vec<(&str, &str, &str)> = Vec::new();
    if let Some(t) = player.and_then(|p| g.world.tank(p)).filter(|_| cosmetic) {
        let (dmg, ammo) = (t.damage, t.ammo);
        let default = t.current.as_ref().zip(t.data.default_projectile.as_ref()).is_some_and(|(c, d)| c.name.eq_ignore_ascii_case(&d.name));
        if let Some((_, pd, pa)) = g.prev_player.filter(|(id, ..)| *id == t.id) {
            // Health: damage crosses int(damage_amount * 0.65); not in single player.
            let thr = (t.data.damage_amount * 0.65) as i32 as f32;
            if pd < thr && dmg >= thr && !h.srv.get().is_some_and(|s| s.setup.single_player()) {
                hints.push(("You need a health power-up!", "2", "2"));
            }
            // Ammo: the default projectile's ammo drops to 20 % of its count.
            if let Some(c) = t.current.as_ref().filter(|_| default) {
                let thr = (c.rounds as f32 * 0.2) as i32;
                if pa > thr && ammo <= thr {
                    hints.push(("You need ammo!", "4", "2"));
                }
            }
        }
        g.ammo_warning = default && t.current.as_ref().is_some_and(|c| ammo <= (c.rounds as f32 * 0.2) as i32);
        g.prev_player = Some((t.id, dmg, ammo));
    }
    h.game = Some(g);
    for (msg, time, size) in hints {
        let now = h.sim_time() as u64;
        h.client.bottom_print(msg, time, size, now);
    }
    // The game rules react to the tick (destroyed tanks, power-ups, the
    // ball out of bounds), then the corpses that timed out go.
    let exists = |h: &EngineHost, e: EntityId| h.game.as_ref().is_some_and(|g| g.entities.get(e).is_some());
    for e in &game_events {
        match e {
            GameEvent::TankDestroyed { tank, killer } if exists(h, *tank) => {
                let (tank, killer) = (*tank, *killer);
                crate::gameplay::server_host::with_server(h, |s, b| s.tank_destroyed(b, tank, killer));
            }
            GameEvent::PowerUpEnter { pup, tank, .. } if exists(h, *pup) && exists(h, *tank) => {
                let db = h.game.as_ref().and_then(|g| g.entities.get(*pup)).map(|e| e.def.clone()).unwrap_or_default();
                let (pup, tank) = (*pup, *tank);
                if let Some(def) = h.data.pickups.find(&db) {
                    crate::gameplay::server_host::with_server(h, |s, b| s.pickup_entered(b, def, pup, tank));
                }
            }
            GameEvent::FlagOutOfBounds { flag } if exists(h, *flag) => {
                let flag = *flag;
                crate::gameplay::server_host::with_server(h, |s, b| s.ball_out_of_bounds(b, flag));
            }
            _ => {}
        }
    }
    for e in &game_events {
        if let GameEvent::TankRemoved { tank } = e {
            if exists(h, *tank) {
                let tank = *tank;
                crate::gameplay::server_host::with_server(h, |s, b| s.delete_tank(b, tank));
            }
        }
    }
}
