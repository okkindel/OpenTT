//! Render state between ticks: interpolated poses and aim, the posed tank.

use super::{predict_aim, GameSim};
use glam::{Mat3, Quat, Vec3};

impl GameSim {
    /// Time into the power-up's `activate` sequence (`dur` s long) at sim
    /// time `now_s`, while the thread runs: forwards after switching on,
    /// backwards after switching off. None once it is done or when it
    /// never switched.
    pub fn activation_time(&self, id: tt_game::TriggerId, now_s: f32, dur: f32) -> Option<f32> {
        let &(at, on) = self.activations.get(&id)?;
        let since = now_s - at as f32 / 1000.0;
        (since >= 0.0 && since < dur).then_some(if on { since } else { dur - since })
    }

    /// Pose of a tank for rendering: interpolated between the previous and
    /// the current tick by `alpha`.
    pub fn render_pose(&self, t: &tt_game::Tank, alpha: f32) -> (Vec3, Mat3) {
        let (p, r) = self.render_pose_raw(t, alpha);
        match self.render_offset.get(&t.id) {
            Some((dp, dq)) => (p + *dp, Mat3::from_quat(*dq * Quat::from_mat3(&r))),
            None => (p, r),
        }
    }

    fn render_pose_raw(&self, t: &tt_game::Tank, alpha: f32) -> (Vec3, Mat3) {
        match self.prev_pose.get(&t.id) {
            // A teleport (respawn, setTransform) is not interpolated.
            Some(&(p, r)) if (t.pos - p).length_squared() < 100.0 => {
                let q = Quat::from_mat3(&r).slerp(Quat::from_mat3(&t.rot), alpha);
                (p.lerp(t.pos, alpha), Mat3::from_quat(q))
            }
            _ => (t.pos, t.rot),
        }
    }

    /// Turret aim (`yaw_n`, `pitch_n`) of a tank for rendering, interpolated
    /// between the previous and the current tick like [`Self::render_pose`].
    ///
    /// For the local player the mouse motion is drawn as it arrives: the
    /// part of the last tick's change that came from the mouse is shown in
    /// full at once (it was already on screen as pending input before the
    /// tick), plus the mouse motion not yet consumed by a tick. The result
    /// is continuous across ticks and follows the mouse frame by frame;
    /// the rest of the change (auto-level, key aiming) still interpolates.
    pub fn render_aim(&self, t: &tt_game::Tank, alpha: f32) -> (f32, f32) {
        let (yaw, pitch) = match self.prev_aim.get(&t.id) {
            Some(p) => (lerp(p.0, t.yaw_n, alpha), lerp(p.1, t.pitch_n, alpha)),
            None => (t.yaw_n, t.pitch_n),
        };
        let a = &self.player_aim;
        if a.tank != Some(t.id) || t.is_corpse {
            return (yaw, pitch);
        }
        // Without a previous tick nothing is interpolated: `consumed` is in already.
        let alpha = if self.prev_aim.contains_key(&t.id) { alpha } else { 1.0 };
        predict_aim((yaw, pitch), a.consumed, a.pending, alpha)
    }

    /// Reads the mouse aim not yet consumed by a tick for
    /// [`Self::render_aim`]. Called once per frame before drawing.
    pub fn set_pending_aim(&mut self, moves: &tt_gameplay::client::MoveState) {
        let (yaw, pitch) = moves.aim();
        self.player_aim.pending = (yaw / std::f32::consts::PI, pitch / std::f32::consts::PI);
    }

    /// Position of a flag (scrum ball) for rendering.
    pub fn render_flag_pos(&self, f: &tt_game::Flag, alpha: f32) -> Vec3 {
        match self.prev_flags.get(&f.id) {
            Some(&p) if (f.pos - p).length_squared() < 100.0 => p.lerp(f.pos, alpha),
            _ => f.pos,
        }
    }

    /// A copy of a tank at its render pose and aim (for what is drawn
    /// attached to it, such as the reticle).
    pub fn render_tank(&self, t: &tt_game::Tank, alpha: f32) -> tt_game::Tank {
        let mut r = t.clone();
        (r.pos, r.rot) = self.render_pose(t, alpha);
        (r.yaw_n, r.pitch_n) = self.render_aim(t, alpha);
        r
    }
}

fn sequence_duration(shape: &tt_assets::dts::Shape, name: &str) -> Option<f32> {
    shape.sequences.iter().find(|s| s.name.eq_ignore_ascii_case(name)).map(|s| s.duration)
}

pub(super) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Poses a tank's nodes for drawing; `alpha` interpolates the turret aim and
/// wheel spin between the last two ticks.
pub fn pose_tank(g: &GameSim, t: &tt_game::Tank, alpha: f32, shape: &tt_assets::dts::Shape, inst: &mut tt_render::DynamicInstance) {
    use glam::Mat4;
    use tt_render::pose::{node_default, node_transform, sequence_pose_of, sequence_visibility_of};
    let d = &t.data;
    if t.is_corpse {
        // Corpse render transform: during corpse_shake_time the tank
        // shakes (Euler x/y wobble); then the death sequence flips it by
        // moving `codeFlip`: drawn at T · codeFlip · tankall⁻¹.
        let (ct, fx) = ((t.corpse_timer - (1.0 - alpha) * tt_game::DT).max(0.0), &d.fx);
        if ct < fx.corpse_shake_time {
            let (w, a) = (std::f32::consts::PI * fx.corpse_shake_speed, 0.15 * fx.corpse_shake_amp);
            let (x, y) = ((10.0 * w * ct).sin() * a, (9.0 * w * (ct - fx.corpse_shake_time)).sin() * a);
            inst.transform *= Mat4::from_mat3(tt_game::geom::rot_x(x) * Mat3::from_rotation_y(-y));
        } else if let Some((k, _)) = t.death_sequence {
            // The world picks death1..4 once the corpse shake is over.
            let seq = format!("death{}", k + 1);
            inst.nodes = sequence_pose_of(shape, &seq, ct - fx.corpse_shake_time);
            for (o, vis) in sequence_visibility_of(shape, &seq, ct - fx.corpse_shake_time) {
                if vis { inst.shown.push(o) } else { inst.hidden.push(o) }
            }
            if let (Some(flip), Some(all)) = (node_transform(shape, &inst.nodes, "codeFlip"), node_transform(shape, &inst.nodes, "tankall")) {
                inst.transform = inst.transform * flip * all.inverse();
            }
            return;
        }
    }
    let (yaw_n, pitch_n) = g.render_aim(t, alpha);
    let yaw = (d.turret_yaw_range * yaw_n).to_radians();
    let pitch = (d.turret_pitch_range * pitch_n - d.turret_pitch_center).to_radians();
    if let Some(m) = node_default(shape, "codeTurret") {
        inst.nodes.push(("codeTurret".into(), m * Mat4::from_mat3(tt_game::geom::rot_z(yaw))));
    }
    if let Some(m) = node_default(shape, "codeWeapon") {
        inst.nodes.push(("codeWeapon".into(), m * Mat4::from_mat3(tt_game::geom::rot_x(pitch))));
    }
    if let Some(tr) = g.treads.get(&t.id) {
        let angles = tr.state.wheel_angles(&tr.rig, alpha);
        for (n, a) in crate::treads::WHEEL_NODES.iter().zip(angles) {
            if let Some(m) = node_default(shape, n) {
                inst.nodes.push(((*n).into(), Mat4::from_translation(m.w_axis.truncate()) * Mat4::from_mat3(tt_game::geom::rot_x(a))));
            }
        }
    }
    // The `fire` thread, restarted by every shot: its visibility track
    // (muzzle flash) and its node tracks (the Light tank's
    // `barrelA2`/`barrelB2` spin 120° per shot, a gatling spin while it
    // keeps firing). Nodes posed above by code stay as they are.
    if let (Some(&at), Some(dur)) = (g.last_fired.get(&t.id), sequence_duration(shape, "fire")) {
        let since = (g.world.time_ms - at) as f32 / 1000.0;
        if since >= 0.0 && since <= dur {
            for (o, vis) in sequence_visibility_of(shape, "fire", since) {
                if vis { inst.shown.push(o) } else { inst.hidden.push(o) }
            }
        }
        // Between ticks for a smooth spin: `alpha` 0 is the previous tick.
        let since = since - (1.0 - alpha) * tt_game::DT;
        if since <= dur {
            for (n, m) in sequence_pose_of(shape, "fire", since.max(0.0)) {
                if !inst.nodes.iter().any(|(k, _)| *k == n) {
                    inst.nodes.push((n, m));
                }
            }
        }
    }
}
