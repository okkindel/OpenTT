//! The game's presentation of world events: sounds, explosions, muzzle
//! flashes, brain jars (every role but `Dedicated`).

use super::GameSim;
use crate::host::EngineHost;
use glam::Vec3;
use tt_game::{Event, HitObject, TankId};
use tt_gameplay::EntityKind;

/// Looping engine sounds of all living tanks: two 3D sources (TankFxParams
/// `engine_sound_a/B`), pitch and volume interpolated between their `0` and `1`
/// values by speed / max_speed. A linear blend is used.
pub(super) fn update_engine_sounds(h: &mut EngineHost, g: &mut GameSim) {
    let live: Vec<(TankId, Vec3, f32, bool)> = g
        .world
        .tanks
        .iter()
        .filter(|t| g.entity_of_tank(t.id).is_some())
        .map(|t| (t.id, t.pos, (t.vel.length() / t.data.max_speed.max(0.1)).clamp(0.0, 1.0), t.is_corpse))
        .collect();
    let gone: Vec<TankId> = g.engine_sounds.keys().copied().filter(|id| !live.iter().any(|l| l.0 == *id && !l.3)).collect();
    for id in gone {
        if let Some((a, b)) = g.engine_sounds.remove(&id) {
            h.audio.stop(a);
            h.audio.stop(b);
        }
    }
    for (id, pos, f, corpse) in live {
        if corpse {
            continue;
        }
        let field = |n: &str| g.fx_value(id, n).unwrap_or("");
        let num = |n: &str| tt_game::fields::atof(field(n));
        // (pitch at 0, pitch at 1, volume at 0, volume at 1) of A and B.
        let curve = |k: [&str; 4]| (num(k[0]), num(k[1]), num(k[2]), num(k[3]));
        let curves = [curve(["engine_pitch_a0", "engine_pitch_a1", "engine_vol_a0", "engine_vol_a1"]), curve(["engine_pitch_b0", "engine_pitch_b1", "engine_vol_b0", "engine_vol_b1"])];
        let (a, b) = match g.engine_sounds.get(&id) {
            Some(&ab) => ab,
            None => {
                let (snd_a, snd_b) = (field("engine_sound_a").to_string(), field("engine_sound_b").to_string());
                let mut start = |snd: String| {
                    if snd.is_empty() || h.audio.profile_name(&snd).is_none() {
                        return 0;
                    }
                    let s = h.audio.create_source(&snd, None, Some(pos.into()));
                    h.audio.play(s)
                };
                let ab = (start(snd_a), start(snd_b));
                g.engine_sounds.insert(id, ab);
                ab
            }
        };
        for (src, (p0, p1, v0, v1)) in [(a, curves[0]), (b, curves[1])] {
            if src == 0 {
                continue;
            }
            h.audio.source_3f(src, tt_audio::AlParam::Position, pos.into());
            h.audio.source_f(src, tt_audio::AlParam::Pitch, (p0 + (p1 - p0) * f).max(0.05));
            h.audio.source_f(src, tt_audio::AlParam::Gain, (v0 + (v1 - v0) * f).max(0.0));
        }
    }
}

fn play_at(h: &mut EngineHost, profile: &str, pos: Vec3) {
    if !profile.is_empty() && h.audio.profile_name(profile).is_some() {
        h.audio.play_profile(profile, Some(pos.into()));
    }
}

/// Presentation of one world event: sounds, explosions, muzzle flashes,
/// brain jars (every role but `Dedicated`). Never calls scripts.
pub(super) fn cosmetic_event(h: &mut EngineHost, g: &mut GameSim, e: &Event, player: Option<TankId>) {
    match e {
        Event::Fired { tank, def, .. } => {
            g.last_fired.insert(*tank, g.world.time_ms);
            if let Some(t) = g.world.tank(*tank) {
                let p = t.pos;
                // A cartridge's own `fire_sound` (mortar, scatter shot).
                let own = if def.is_empty() { None } else { g.projectile(def).map(|d| d.fire_sound.clone()).filter(|s| !s.is_empty()) };
                play_at(h, own.as_deref().unwrap_or("fireSound"), p);
            }
        }
        Event::DryFired { tank } => {
            if let Some(t) = g.world.tank(*tank) {
                let p = t.pos;
                play_at(h, "dryFireSound", p);
            }
        }
        Event::ProjectileBounced { projectile, pos, .. } => {
            if let Some(p) = g.world.projectile(*projectile).map(|p| p.data.bounce_explosion.clone()).filter(|e| !e.is_empty()) {
                g.explosions.push((p, *pos));
            }
        }
        Event::ProjectileExploded { pos, hit, def, .. } => {
            if let Some(p) = g.db.projectile(def) {
                let ex = match hit {
                    // The own-tank variant when the player's tank is hit.
                    HitObject::Tank(t) if Some(*t) == player && !p.own_tank_explosion.is_empty() => p.own_tank_explosion,
                    HitObject::Tank(_) => p.tank_explosion,
                    _ => p.other_explosion,
                };
                if !ex.is_empty() {
                    g.explosions.push((ex.clone(), *pos));
                }
                let snd = g.db.fields(&ex).and_then(|f| f.get("sound").map(str::to_string)).unwrap_or_default();
                play_at(h, &snd, *pos);
            }
        }
        Event::PadUsed { trigger, .. } | Event::VolcanoErupted { trigger, .. } => {
            if let (Some(_), Some(tr)) = (g.entities.of_trigger(*trigger), g.world.trigger(*trigger)) {
                let p: Vec3 = tr.transform.translation.into();
                play_at(h, g.def_fields(EntityKind::Trigger(*trigger)).as_ref().and_then(|f| f.get("sound")).unwrap_or(""), p);
            }
        }
        Event::PowerUpSwitched { trigger, on } => {
            if g.known_triggers.contains(trigger) {
                g.activations.insert(*trigger, (g.world.time_ms, *on));
            }
            // PowerUpParams `sound` when it (re)appears, `sound_off` when taken.
            if let (Some(_), Some(tr)) = (g.entities.of_trigger(*trigger), g.world.trigger(*trigger)) {
                let p: Vec3 = tr.transform.translation.into();
                play_at(h, g.def_fields(EntityKind::Trigger(*trigger)).as_ref().and_then(|f| f.get(if *on { "sound" } else { "sound_off" })).unwrap_or(""), p);
            }
        }
        Event::BrainEjected { tank, projectile, shape } => {
            if let Some(p) = projectile.filter(|_| !shape.is_empty()) {
                g.brain_shapes.insert(p, shape.clone());
            }
            if let (Some(_), Some(t)) = (g.entity_of_tank(*tank), g.world.tank(*tank)) {
                let p = t.pos;
                play_at(h, g.fx_value(*tank, "loose_head_sound").unwrap_or(""), p);
            }
        }
        Event::Collided { tank, other, speed } => {
            // TankFxDef hard/soft collision sounds by relative speed,
            // at most once per `*_collision_interval_ms` per tank and kind.
            if let (Some(_), Some(t)) = (g.entity_of_tank(*tank), g.world.tank(*tank)) {
                let (kind, k) = match other {
                    HitObject::Tank(_) => ("tank", 0u8),
                    HitObject::Static(_) => ("static", 1),
                    HitObject::Terrain => ("ground", 2),
                };
                let num = |n: String| tt_game::fields::atof(g.fx_value(*tank, &n).unwrap_or(""));
                let interval = num(format!("{kind}_collision_interval_ms")) as i64;
                let now = g.world.time_ms;
                if g.last_coll_sound.get(&(*tank, k)).is_none_or(|&last| now - last >= interval) {
                    let snd = if *speed >= num(format!("hard_{kind}_collision_speed")) {
                        g.fx_value(*tank, &format!("hard_{kind}_collision_sound")).unwrap_or_default().to_string()
                    } else if *speed >= num(format!("soft_{kind}_collision_speed")) {
                        g.fx_value(*tank, &format!("soft_{kind}_collision_sound")).unwrap_or_default().to_string()
                    } else {
                        String::new()
                    };
                    if !snd.is_empty() {
                        let p = t.pos;
                        play_at(h, &snd, p);
                        g.last_coll_sound.insert((*tank, k), now);
                    }
                }
            }
        }
        Event::ProjectileRemoved { projectile } => {
            // Expired brain jars (cosmetic, 5 s) stop being drawn.
            g.brain_shapes.remove(projectile);
        }
        _ => {}
    }
}
