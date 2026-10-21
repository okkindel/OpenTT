//! The player's aim: the move from the move state, camera aim's free
//! camera and the turret target, the drawn mouse aim.

use super::camera::{free_eye_height, free_view, heading_of, wrap_pi, FREE_AIM_RANGE, FREE_PITCH_RANGE};
use super::GameSim;
use glam::{Affine3A, Vec3};
use tt_game::TankId;

/// The player's move from the client's move state; resets the yaw/pitch/roll as the
/// engine does.
///
/// For the local player's move (`MoveState::take_move`). DEVIATION: when set, the turn
/// input is negated while the throttle is reversed, so A/D steer like a car backing up.
/// Only the local player's move is touched — bots and the simulation are unchanged.
pub(super) fn reverse_steering(prefs: &tt_shell::Prefs) -> bool {
    let v = prefs.get("input.reverse_steering");
    let v = v.trim();
    // Unset: off, as the settings default (tt_shell::Prefs::reverse_steering).
    !v.is_empty() && (v.eq_ignore_ascii_case("true") || tt_game::fmt::atof(v) != 0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimMode {
    Classic,
    /// 1: free camera: the mouse turns the camera, the turret follows it
    /// within its limits and the hull turns towards it beyond them.
    Camera,
}

pub fn aim_mode(prefs: &tt_shell::Prefs) -> AimMode {
    let v = prefs.get("input.aim_mode");
    if !v.is_empty() && tt_game::fmt::atoi(&v) == 0 {
        AimMode::Classic
    } else {
        AimMode::Camera
    }
}

/// Camera aim's free camera: client-only view state turned by the mouse. The player's
/// turret is steered towards where it looks through the normal moves.
#[derive(Debug, Clone, Copy, Default)]
pub struct FreeAim {
    /// The player's tank while camera aim is on (None: classic, or dead).
    pub tank: Option<TankId>,
    /// World heading of the view (radians, + = clockwise from above).
    pub yaw: f32,
    pub pitch: f32,
    /// Half the vertical field of view of the game view (radians), set by
    /// the frontend each frame.
    pub half_vfov: f32,
}

/// Where the free camera wants the turret (see [`GameSim::turret_target`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurretTarget {
    /// The aim point: under the screen centre, or [`FREE_AIM_RANGE`] along the view.
    pub point: Vec3,
    /// Turret yaw/pitch (`yawN`/`pitchN`) within the turret's range.
    pub yaw_n: f32,
    pub pitch_n: f32,
    /// -1/+1 when the aim point is beyond the turret's yaw range to the
    /// left/right (the hull should turn), else 0.
    pub beyond: f32,
}

/// Drawn aim of the local player from the tick-interpolated aim `lerped`:
/// the mouse part of the last tick's change (`consumed`) is shown in full,
/// not interpolated, plus the mouse motion since (`pending`); all in
/// `yawN`/`pitchN` units, clamped to the turret range.
pub fn predict_aim(lerped: (f32, f32), consumed: (f32, f32), pending: (f32, f32), alpha: f32) -> (f32, f32) {
    let k = 1.0 - alpha.clamp(0.0, 1.0);
    ((lerped.0 + k * consumed.0 + pending.0).clamp(-1.0, 1.0), (lerped.1 + k * consumed.1 + pending.1).clamp(-1.0, 1.0))
}

/// Mouse aim of the local player for rendering. Drawing the turret from
/// tick states alone makes it step with the uneven number of mouse events
/// per tick. Render-only: the simulation never reads this.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerAim {
    /// The player's tank while its moves reach the sim.
    pub tank: Option<TankId>,
    /// Mouse yaw/pitch (`yawN`/`pitchN` units, i.e. radians / π) the last
    /// tick consumed, as the server quantises it.
    pub consumed: (f32, f32),
    /// Mouse yaw/pitch (same units) accumulated since that tick.
    pub pending: (f32, f32),
}

impl GameSim {
    /// Camera aim: starts the free camera for the player's tank (looking
    /// along its turret), or stops it (classic aim, no player, dead).
    pub fn sync_free_aim(&mut self, aim: AimMode, player: Option<TankId>) {
        let want = player
            .filter(|_| aim == AimMode::Camera && !self.world.ai_control_mode)
            .filter(|&p| self.world.tank(p).is_some_and(|t| !t.is_corpse));
        if want == self.free_aim.tank {
            return;
        }
        let half_vfov = self.free_aim.half_vfov;
        self.free_aim = FreeAim { half_vfov, ..FreeAim::default() };
        if let Some(t) = want.and_then(|p| self.world.tank(p)) {
            let yaw = wrap_pi(heading_of(t.rot.y_axis) + (t.data.turret_yaw_range * t.yaw_n).to_radians());
            self.free_aim = FreeAim { tank: want, yaw, pitch: 0.0, half_vfov };
        }
    }

    /// Camera aim: turns the free camera by `yaw`/`pitch` radians of mouse
    /// aim.
    pub fn turn_free_aim(&mut self, yaw: f32, pitch: f32) {
        let a = &mut self.free_aim;
        a.yaw = wrap_pi(a.yaw + yaw);
        a.pitch = (a.pitch - pitch).clamp(FREE_PITCH_RANGE.0, FREE_PITCH_RANGE.1);
    }

    /// Camera aim: the mouse motion not yet taken by a tick turns the free
    /// camera at once and is taken out of the turret's move. Called every
    /// frame and before each tick.
    pub fn free_look_input(&mut self, moves: &mut tt_gameplay::client::MoveState) {
        if self.free_aim.tank.is_none() {
            return;
        }
        let (yaw, pitch) = moves.aim();
        if yaw != 0.0 || pitch != 0.0 {
            moves.clear_aim();
            self.turn_free_aim(yaw, pitch);
        }
    }

    /// The free camera of tank `id` at its tick pose (unsmoothed), for steering.
    pub fn free_view_at_tick(&self, id: TankId) -> Option<Affine3A> {
        let t = self.world.tank(id)?;
        let cd = self.camera_data(id);
        let ground = |x: f32, y: f32| self.world.terrain.as_ref().map(|h| h.height(x, y));
        let h = free_eye_height(t.pos, self.free_aim.yaw, &cd, ground);
        Some(free_view(t.pos, self.free_aim.yaw, self.free_aim.pitch, h, self.free_aim.half_vfov, &cd, ground).0)
    }

    /// The point under the screen centre of `view` (or [`FREE_AIM_RANGE`]
    /// along it), ignoring tank `id`.
    pub fn aim_point(&self, view: &Affine3A, id: TankId) -> Vec3 {
        let eye: Vec3 = view.translation.into();
        let end = eye + Vec3::from(view.matrix3.y_axis) * FREE_AIM_RANGE;
        self.world.segment_hit_point(eye, end, Some(id)).unwrap_or(end)
    }

    /// Camera aim: the turret aim of tank `id` that hits the point under the centre of `view`. Yaw points the
    /// barrel at it (in the hull's frame, less the hull's turn of the last tick, clamped to `turret_yaw_range`);
    /// pitch is found by bisection over the turret's range so the shell's ballistic path (the current cartridge's
    /// velocity and gravity, as the reticle draws it) passes through it, or the nearest limit when out of reach.
    /// For a high-angle cartridge (the mortar) the pitch is solved over its own launch
    /// (`Tank::high_angle_pitch_n`): the shell lands on the point.
    pub fn turret_target(&self, id: TankId, view: &Affine3A) -> Option<TurretTarget> {
        let t = self.world.tank(id)?;
        let point = self.aim_point(view, id);
        let d = &t.data;
        let local = t.rot.transpose() * (point - t.pos);
        // Too close to the tank for a direction: the view's heading.
        let rel = if local.truncate().length() > 3.0 {
            local.x.atan2(local.y)
        } else {
            wrap_pi(heading_of(view.matrix3.y_axis.into()) - heading_of(t.rot.y_axis))
        };
        // The hull turns during the coming tick about as much as in the
        // last one; the turret yaw is relative to it.
        let turn = self.prev_pose.get(&id).map_or(0.0, |(_, r)| wrap_pi(heading_of(t.rot.y_axis) - heading_of(r.y_axis)));
        let rel = wrap_pi(rel - turn);
        let range = d.turret_yaw_range.to_radians().max(1e-3);
        let beyond = if rel > range { 1.0 } else if rel < -range { -1.0 } else { 0.0 };
        let yaw_n = (rel / range).clamp(-1.0, 1.0);
        // A high-angle cartridge (the mortar) launches at its own
        // elevation and speed, not along the barrel: its pitch comes from
        // the shell's real flight, so it comes down on the aim point. The
        // yaw is corrected for the barrel's heading on a tilted hull
        // (with the hull's turn of the coming tick kept out).
        if let Some(p) = t.current.as_ref().filter(|p| p.high_angle()) {
            let turn_n = turn / range;
            let (y, pitch_n) = t.high_angle_aim(p, (yaw_n + turn_n).clamp(-1.0, 1.0), point, self.world.gravity);
            let yaw_n = if beyond == 0.0 { (y - turn_n).clamp(-1.0, 1.0) } else { yaw_n };
            return Some(TurretTarget { point, yaw_n, pitch_n, beyond });
        }
        let (speed, grav) = t.current.as_ref().map_or((100.0, 0.0), |p| (p.velocity, -p.fall(self.world.gravity)));
        // Height of the shell's path over the aim point where it passes it
        // horizontally (decreasing in pitchN: + = barrel down).
        let over = |pitch_n: f32| {
            let mut c = t.clone();
            (c.yaw_n, c.pitch_n) = (yaw_n, pitch_n);
            let m = c.muzzle();
            let (p0, v) = (Vec3::from(m.translation), Vec3::from(m.matrix3.y_axis) * speed);
            let vh = v.truncate().length();
            if vh < 1e-3 {
                return v.z;
            }
            let s = (point - p0).truncate().length() / vh;
            p0.z + v.z * s + 0.5 * grav * s * s - point.z
        };
        let pitch_n = if over(-1.0) <= 0.0 {
            -1.0
        } else if over(1.0) >= 0.0 {
            1.0
        } else {
            let (mut up, mut down) = (-1.0f32, 1.0f32);
            for _ in 0..20 {
                let m = 0.5 * (up + down);
                if over(m) > 0.0 {
                    up = m;
                } else {
                    down = m;
                }
            }
            0.5 * (up + down)
        };
        Some(TurretTarget { point, yaw_n, pitch_n, beyond })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tt_gameplay::client::binds::MoveAxis;
    use tt_gameplay::client::MoveState;
    use tt_game::Move;
    #[cfg(feature = "dev")]
    use crate::game::render::lerp;

    #[test]
    fn player_moves() {
        let prefs = tt_shell::Prefs::default();
        let mut m = MoveState::default();
        m.set_axis(MoveAxis::Forward, "1");
        m.set_axis(MoveAxis::Left, "1");
        m.yaw = tt_gameplay::client::text::SVal::Str("0.01".into());
        m.trigger(0);
        let mut last = 0;
        let mv = m.take_move(reverse_steering(&prefs), &mut last);
        assert_eq!((mv.x, mv.y, mv.yaw), (-1.0, 1.0, 0.01));
        assert!(mv.trigger[0]);
        assert_eq!(m.yaw.string(), "0");
        m.trigger(0);
        assert!(m.take_move(reverse_steering(&prefs), &mut last).trigger[0]); // changed
        assert!(!m.take_move(reverse_steering(&prefs), &mut last).trigger[0]); // even, unchanged
    }

    /// Yaw change (radians, + = right/clockwise from above) after holding
    /// the given keys for `ticks` ticks on flat ground.
    fn held_turn(prefs: &tt_shell::Prefs, keys: &[MoveAxis], ticks: usize) -> f32 {
        let mut m = MoveState::default();
        for k in [MoveAxis::Forward, MoveAxis::Backward, MoveAxis::Left, MoveAxis::Right] {
            m.set_axis(k, if keys.contains(&k) { "1" } else { "0" });
        }
        let f = tt_game::fields::Fields::from_pairs(
            "MediumTank",
            "TankDef",
            &[("turn_rate", "68"), ("accel_rate", "12"), ("decel_rate", "20"), ("coast_decel_rate", "10"), ("anti_slide_rate", "11.1"), ("max_speed", "11.9"), ("suspension_range", "0.6"), ("damage_amount", "14")],
        );
        let data = std::sync::Arc::new(tt_game::TankParams::from_fields(&f, None));
        let mut world = tt_game::World::new(Some(tt_game::HeightField::flat(0.0)));
        let id = world.add_tank(data, Vec3::ZERO, 0.0);
        let mut last = 0;
        for _ in 0..ticks {
            let mv = m.take_move(reverse_steering(prefs), &mut last);
            world.set_move(id, mv);
            world.tick();
        }
        let f = world.tank(id).unwrap().forward();
        f.x.atan2(f.y)
    }

    #[test]
    fn reverse_steering_turns_like_a_car() {
        use MoveAxis::*;
        let mut prefs = tt_shell::Prefs::default();
        // Pref unset: off (pivot steering), like the settings default.
        assert!(!reverse_steering(&prefs));
        // On: S+A backs up turning the hull right, S+D left.
        prefs.set("input.reverse_steering", "1");
        let sa = held_turn(&prefs, &[Backward, Left], 20);
        let sd = held_turn(&prefs, &[Backward, Right], 20);
        let wa = held_turn(&prefs, &[Forward, Left], 20);
        assert!(sa > 0.1, "S+A should yaw right, got {sa}");
        assert!(sd < -0.1, "S+D should yaw left, got {sd}");
        assert!(wa < -0.1, "W+A yaws left, got {wa}");
        prefs.set("input.reverse_steering", "0");
        let sa0 = held_turn(&prefs, &[Backward, Left], 20);
        assert!(sa0 < -0.1, "classic S+A yaws left, got {sa0}");
        // The move itself: x is negated only while y < 0.
        prefs.set("input.reverse_steering", "1");
        let mut m = MoveState::default();
        m.set_axis(Backward, "1");
        m.set_axis(Left, "1");
        let mut last = 0;
        assert_eq!(m.take_move(reverse_steering(&prefs), &mut last).x, 1.0);
        m.set_axis(Backward, "0");
        assert_eq!(m.take_move(reverse_steering(&prefs), &mut last).x, -1.0);
    }

    #[test]
    fn predicted_aim_is_continuous_across_a_tick() {
        // Before the tick: tick aim 0.2, mouse 0.05 pending.
        let before = predict_aim((0.2, 0.0), (0.0, 0.0), (0.05, 0.0), 1.0);
        // The tick consumed it: prev 0.2, cur 0.25, alpha ~0.
        let after = predict_aim((0.2, 0.0), (0.05, 0.0), (0.0, 0.0), 0.0);
        assert!((before.0 - after.0).abs() < 1e-6);
        // Late in the tick the lerp has caught up; nothing is counted twice.
        assert!((predict_aim((0.25, 0.0), (0.05, 0.0), (0.0, 0.0), 1.0).0 - 0.25).abs() < 1e-6);
        // Clamped to the turret range.
        assert_eq!(predict_aim((0.98, 0.0), (0.0, 0.0), (0.1, -2.0), 1.0), (1.0, -1.0));
    }

    /// Turret aim drawn while the mouse turns the turret and frames run at
    /// `fps`, with `jitter(frame)` scaling each frame's mouse motion.
    /// Returns the frame-stats judder and the largest lag behind the mouse
    /// (yawN units) of (the mouse itself, tick interpolation only, with the
    /// pending mouse motion drawn).
    #[cfg(feature = "dev")]
    fn turret_motion(fps: f64, jitter: impl Fn(usize) -> f32) -> [(f64, f32); 3] {
        let f = tt_game::fields::Fields::from_pairs("MediumTank", "TankDef", &[("turret_yaw_range", "30"), ("turret_pitch_range", "30"), ("turret_pitch_center", "10")]);
        let data = std::sync::Arc::new(tt_game::TankParams::from_fields(&f, None));
        let mut world = tt_game::World::new(Some(tt_game::HeightField::flat(0.0)));
        let id = world.add_tank(data, Vec3::ZERO, 0.0);
        let mut probes = [crate::framestats::Probe::default(); 3];
        let mut lag = [0.0f32; 3];
        let (mut acc, mut pending, mut consumed, mut t, mut total) = (0.0f64, 0.0f32, 0.0f32, 0.0f64, 0.0f32);
        let mut prev = 0.0f32;
        let dt = 1000.0 / fps;
        for i in 0..(3.0 * fps) as usize {
            // Mouse events between frames.
            pending += 0.5 * (dt / 1000.0) as f32 * jitter(i);
            acc += dt;
            t += dt;
            while acc >= tt_game::TICK_MS as f64 {
                prev = world.tank(id).unwrap().yaw_n;
                let q = tt_game::netmove::u16_to_angle(tt_game::netmove::angle_to_u16(pending));
                consumed = q / std::f32::consts::PI;
                total += consumed;
                world.set_move(id, Move { yaw: pending, ..Move::NULL });
                pending = 0.0;
                world.tick();
                acc -= tt_game::TICK_MS as f64;
            }
            let alpha = (acc / tt_game::TICK_MS as f64) as f32;
            let lerped = lerp(prev, world.tank(id).unwrap().yaw_n, alpha);
            let drawn = predict_aim((lerped, 0.0), (consumed, 0.0), (pending / std::f32::consts::PI, 0.0), alpha).0;
            // Mouse input so far as the sim gets it (16-bit quantised per move).
            let mouse = total + pending / std::f32::consts::PI;
            for (k, v) in [mouse, lerped, drawn].into_iter().enumerate() {
                probes[k].add(Vec3::new(v, 0.0, 0.0), t);
                lag[k] = lag[k].max(mouse - v);
            }
        }
        assert!(world.tank(id).unwrap().yaw_n < 0.99, "stayed off the limit");
        [0, 1, 2].map(|k| (probes[k].judder(), lag[k]))
    }

    #[test]
    #[cfg(feature = "dev")]
    fn mouse_aim_drawn_without_tick_judder() {
        for fps in [60.0, 75.0, 144.0, 240.0] {
            let [_, old, new] = turret_motion(fps, |_| 1.0);
            let [mouse_j, old_j, new_j] = turret_motion(fps, |i| if i % 3 == 0 { 1.3 } else { 0.85 });
            println!("{fps} fps steady mouse: judder {:.2} -> {:.2}, lag {:.4} -> {:.4} | uneven mouse (judder {:.2}): {:.2} -> {:.2}", old.0, new.0, old.1, new.1, mouse_j.0, old_j.0, new_j.0);
            // Steady motion stays steady, and the drawn turret is where the
            // mouse put it.
            assert!(new.0 <= old.0 + 0.01 && new.0 < 0.05, "{fps}: {old:?} -> {new:?}");
            assert!(new.1 < 1e-4 && old.1 > 0.004, "{fps}: lag {old:?} -> {new:?}");
            // Uneven mouse motion is drawn as it came, not made worse.
            assert!((new_j.0 - mouse_j.0).abs() < 0.05, "{fps}: {mouse_j:?} {new_j:?}");
        }
    }
}
