//! Tank tread and wheel animation (client-only cosmetics).
//!
//! * Only for living tanks.
//! * The node animation turns that travel `s` into
//!   - (right side for RF/RR, left for LR/LF).
//!
//! The IFL frame then follows from the thread time `pos · duration` plus the
//! sequence's toolBegin ([`tt_render::pose::sequence_ifl_of`]).

use glam::{Mat3, Vec3};
use tt_assets::dts::Shape;

/// Probe nodes TankParams::preload measures the wheels at.
const PROBE_NODES: [&str; 8] = ["inner_RF", "inner_RR", "inner_LR", "inner_LF", "outer_RF", "outer_RR", "outer_LR", "outer_LF"];

/// Spinning wheel nodes, in [`TreadRig::wheel_radius`] order.
pub const WHEEL_NODES: [&str; 8] = ["inner_RF_rot", "inner_RR_rot", "inner_LR_rot", "inner_LF_rot", "outer_RF_rot", "outer_RR_rot", "outer_LR_rot", "outer_LF_rot"];

/// Track sides: left (bounds min.x edge, `ltread`), right (max.x, `rtread`).
pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;

/// Tread sequence names.
pub const SEQUENCES: [&str; 2] = ["LTread", "RTread"];

/// `0.5` (period factor).
const HALF: f32 = 0.5;
const TWO_PI: f32 = std::f64::consts::TAU as f32;
/// `0.25` (mean of the four inner wheels).
const QUARTER: f32 = 0.25;

/// Per-shape values TankParams::preload derives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreadRig {
    /// Tread cycles per metre of track travel, per side: `1 / y` of the
    /// sequence's last ground translation, 1 if the sequence is missing or
    /// has no ground frames.
    pub cycles_per_metre: [f32; 2],
    /// Duration of the `LTread` / `RTread` sequences (None: not in the shape).
    pub duration: [Option<f32>; 2],
    /// Wheel radii of [`WHEEL_NODES`]: the inner wheels' node height above
    /// the bounds' bottom; the outer ones' mesh half-height plus the inner
    /// wheels' mean (height − mesh half-height).
    pub wheel_radius: [f32; 8],
    /// Wrap period of the travel accumulators: `0.5 / (k_R·k_L) · Π
    /// 2π·r_i` over the eight wheels.
    pub period: f32,
    /// Shape-space x of the left and right track edges.
    pub edge_x: [f32; 2],
}

impl TreadRig {
    pub fn from_shape(shape: &Shape) -> TreadRig {
        let seq = |name: &str| shape.sequences.iter().find(|s| s.name.eq_ignore_ascii_case(name));
        // 1 / groundTranslations[first + num − 1].y.
        let k = |name: &str| {
            seq(name)
                .filter(|s| s.num_ground_frames > 0)
                .and_then(|s| shape.ground_translations.get((s.first_ground_frame + s.num_ground_frames - 1).max(0) as usize))
                .map(|g| 1.0 / g[1])
                .filter(|k| k.is_finite())
                .unwrap_or(1.0)
        };
        let cycles_per_metre = [k(SEQUENCES[LEFT]), k(SEQUENCES[RIGHT])];
        let duration = SEQUENCES.map(|n| seq(n).map(|s| s.duration));
        let min_z = shape.bounds.min[2];
        let world = shape.node_world_transforms();
        // Default shape-space height of a probe node and the half-height of
        // the detail-0 mesh of its first object (searching down the
        // first-child chain, as preload walks `firstObject` / `firstChild`).
        let probe = |name: &str| -> (f32, f32) {
            let Some(mut n) = shape.nodes.iter().position(|x| x.name.eq_ignore_ascii_case(name)) else { return (min_z, 0.0) };
            let z = world[n][2][3];
            let half = loop {
                if let Some(o) = shape.objects.iter().find(|o| o.node == Some(n)) {
                    break shape.meshes.get(o.first_mesh).map_or(0.0, |m| HALF * (m.bounds.max[2] - m.bounds.min[2]));
                }
                match shape.nodes.iter().position(|x| x.parent == Some(n)) {
                    Some(c) => n = c,
                    None => break 0.0,
                }
            };
            (z, half)
        };
        let mut period = (1.0 / cycles_per_metre[RIGHT]) * (1.0 / cycles_per_metre[LEFT]) * HALF;
        let mut wheel_radius = [0.0f32; 8];
        let mut mean = 0.0f32;
        for (i, name) in PROBE_NODES.iter().enumerate().take(4) {
            let (z, half) = probe(name);
            let h = z - min_z;
            wheel_radius[i] = h;
            mean += (h - half) * QUARTER;
            period = h * period * TWO_PI;
        }
        for (i, name) in PROBE_NODES.iter().enumerate().skip(4) {
            let (_, half) = probe(name);
            wheel_radius[i] = mean + half;
            period = wheel_radius[i] * period * TWO_PI;
        }
        TreadRig { cycles_per_metre, duration, wheel_radius, period, edge_x: [shape.bounds.min[0], shape.bounds.max[0]] }
    }

    /// World positions of the two track edges.
    pub fn edges(&self, pos: Vec3, rot: Mat3) -> [Vec3; 2] {
        self.edge_x.map(|x| pos + rot * Vec3::new(x, 0.0, 0.0))
    }

    /// Side whose travel turns wheel `i` of [`WHEEL_NODES`]: `i & 3 < 2`
    /// (RF, RR) the right one.
    pub fn wheel_side(i: usize) -> usize {
        if i & 3 < 2 { RIGHT } else { LEFT }
    }
}

/// Travel accumulators of one tank.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TreadState {
    /// Track travel per side, metres.
    pub travel: [f32; 2],
    /// `old − new` of the last step per side.
    pub back: [f32; 2],
}

impl TreadState {
    /// Adds the steps `d` and wraps by `period` once both the new and the
    /// old value are past the same end (so the interpolated value never
    /// crosses the wrap).
    pub fn advance(&mut self, d: [f32; 2], period: f32) {
        for ((travel, back_out), d) in self.travel.iter_mut().zip(&mut self.back).zip(d) {
            let old = *travel;
            let new = d + old;
            let back = old - new;
            *back_out = back;
            let old = back + new;
            *travel = if 0.0 <= new || 0.0 <= old {
                if period < new && period < old { new - period } else { new }
            } else {
                new + period
            };
        }
    }

    /// Steps for a move from pose `(old_pos, old_rot)` to `(pos, rot)`:
    /// `(old edge − new edge) · new forward` per side.
    pub fn steps(rig: &TreadRig, (old_pos, old_rot): (Vec3, Mat3), (pos, rot): (Vec3, Mat3)) -> [f32; 2] {
        let (a, b) = (rig.edges(old_pos, old_rot), rig.edges(pos, rot));
        let f = rot.y_axis;
        [(a[0] - b[0]).dot(f), (a[1] - b[1]).dot(f)]
    }

    /// Travel between the last two ticks: `alpha` 0 is the previous tick, 1
    /// the current one.
    pub fn travel_at(&self, alpha: f32) -> [f32; 2] {
        let back_delta = 1.0 - alpha.clamp(0.0, 1.0);
        [0, 1].map(|i| back_delta * self.back[i] + self.travel[i])
    }

    pub fn thread_pos(&self, rig: &TreadRig, alpha: f32) -> [f32; 2] {
        let s = self.travel_at(alpha);
        [0, 1].map(|i| {
            let p = (1.0 - s[i] * rig.cycles_per_metre[i]) % 1.0;
            let p = p - p.trunc();
            let p = if p < 0.0 { p + 1.0 } else { p };
            if p >= 1.0 { p - 1.0 } else { p }
        })
    }

    /// Spin angle of each wheel of [`WHEEL_NODES`]: `−s / r`.
    pub fn wheel_angles(&self, rig: &TreadRig, alpha: f32) -> [f32; 8] {
        let s = self.travel_at(alpha);
        std::array::from_fn(|i| -s[TreadRig::wheel_side(i)] / rig.wheel_radius[i])
    }
}

/// A tank's tread rig and travel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Treads {
    pub rig: TreadRig,
    pub state: TreadState,
}

impl Treads {
    pub fn new(rig: TreadRig) -> Treads {
        Treads { rig, state: TreadState::default() }
    }

    /// One tick of a tank moving from `old` to `new` pose. Corpses do not
    /// update their treads; their last step is dropped so the frozen treads
    /// stay still.
    pub fn tick(&mut self, alive: bool, old: (Vec3, Mat3), new: (Vec3, Mat3)) {
        if alive {
            let d = TreadState::steps(&self.rig, old, new);
            self.state.advance(d, self.rig.period);
        } else {
            self.state.back = [0.0; 2];
        }
    }

    /// Sequence times (seconds) of the `LTread` / `RTread` threads:
    /// `pos · duration`.
    pub fn sequence_times(&self, alpha: f32) -> [Option<f32>; 2] {
        let pos = self.state.thread_pos(&self.rig, alpha);
        [0, 1].map(|i| self.rig.duration[i].map(|d| pos[i] * d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tank_shape(n: u32) -> Option<Shape> {
        let p = tt_assets::layout::default_dir().join(format!("models/tanks/tank0{n}.dts"));
        Shape::parse(&std::fs::read(p).ok()?).ok()
    }

    fn rig() -> TreadRig {
        TreadRig {
            cycles_per_metre: [5.0, 5.0],
            duration: [Some(0.2), Some(0.2)],
            wheel_radius: [0.25; 8],
            period: 4.0,
            edge_x: [-1.0, 1.0],
        }
    }

    #[test]
    fn advance_wraps_one_tick_late() {
        let mut s = TreadState::default();
        s.advance([-0.5, 0.5], 4.0);
        assert_eq!(s.travel, [-0.5, 0.5]);
        assert_eq!(s.back, [0.5, -0.5]);
        // Old and new both below 0: wraps up by the period.
        s.advance([-0.5, 0.0], 4.0);
        assert_eq!(s.travel[0], 3.0);
        assert_eq!(s.back[0], 0.5);
        // The interpolated value stays continuous across the wrap.
        assert!((s.travel_at(0.0)[0] - 3.5).abs() < 1e-6);
        let mut s = TreadState { travel: [4.5, 0.0], back: [0.0; 2] };
        s.advance([0.25, 0.0], 4.0);
        assert_eq!(s.travel[0], 0.75);
    }

    #[test]
    fn driving_forward_advances_both_threads_and_turning_splits_them() {
        let r = rig();
        let mut t = Treads::new(r);
        let rot = Mat3::IDENTITY;
        t.tick(true, (Vec3::ZERO, rot), (Vec3::new(0.0, 0.02, 0.0), rot));
        // Forward: old − new = −0.02 along F on both sides.
        assert!((t.state.travel[0] + 0.02).abs() < 1e-6 && (t.state.travel[1] + 0.02).abs() < 1e-6);
        let p = t.state.thread_pos(&r, 1.0);
        assert!((p[0] - 0.1).abs() < 1e-5 && (p[1] - 0.1).abs() < 1e-5, "{p:?}");
        // Between the ticks the position is interpolated.
        assert!((t.state.thread_pos(&r, 0.5)[0] - 0.05).abs() < 1e-5);
        // Turning in place to the left (counter-clockwise from above): the
        // left edge moves back, the right edge forward.
        let mut t = Treads::new(r);
        let turned = Mat3::from_rotation_z(0.05);
        t.tick(true, (Vec3::ZERO, rot), (Vec3::ZERO, turned));
        assert!(t.state.travel[LEFT] > 0.0 && t.state.travel[RIGHT] < 0.0, "{:?}", t.state.travel);
        assert!((t.state.travel[LEFT] + t.state.travel[RIGHT]).abs() < 1e-6);
        // Standing still keeps the position (the right side's negative
        // travel wraps by the period now, a whole number of cycles here);
        // corpses keep theirs.
        let before = t.state.thread_pos(&r, 1.0);
        let close = |a: [f32; 2], b: [f32; 2]| (0..2).all(|i| (a[i] - b[i]).abs() < 1e-4);
        t.tick(true, (Vec3::ZERO, turned), (Vec3::ZERO, turned));
        assert!(t.state.travel[RIGHT] > 3.9, "{:?}", t.state.travel);
        assert!(close(t.state.thread_pos(&r, 0.3), before));
        t.tick(false, (Vec3::ZERO, turned), (Vec3::new(5.0, 5.0, 0.0), turned));
        assert!(close(t.state.thread_pos(&r, 0.3), before));
    }

    #[test]
    fn wheels_spin_by_travel_over_radius() {
        let r = rig();
        let mut t = Treads::new(r);
        t.tick(true, (Vec3::ZERO, Mat3::IDENTITY), (Vec3::new(0.0, 0.1, 0.0), Mat3::IDENTITY));
        // s = −0.1 on both sides: angle −s / r = 0.4 rad.
        for a in t.state.wheel_angles(&r, 1.0) {
            assert!((a - 0.4).abs() < 1e-5);
        }
        assert_eq!(TreadRig::wheel_side(0), RIGHT);
        assert_eq!(TreadRig::wheel_side(2), LEFT);
        assert_eq!(TreadRig::wheel_side(5), RIGHT);
        assert_eq!(TreadRig::wheel_side(7), LEFT);
    }

    /// TankParams::preload values of the medium tank (tank01.dts).
    #[test]
    fn tank01_rig() {
        let Some(shape) = tank_shape(1) else {
            eprintln!("skipped: no game data");
            return;
        };
        let r = TreadRig::from_shape(&shape);
        // Ground travel of one tread cycle: 0.19379 m (left), 0.18968 m (right).
        assert!((r.cycles_per_metre[LEFT] - 1.0 / 0.193_787_7).abs() < 1e-3, "{r:?}");
        assert!((r.cycles_per_metre[RIGHT] - 1.0 / 0.189_676_6).abs() < 1e-3, "{r:?}");
        assert!((r.duration[LEFT].unwrap() - 7.0 / 30.0).abs() < 1e-4);
        // Inner wheels: node height 0.233; outer: mesh half-height + 0.0665.
        assert!((r.wheel_radius[0] - 0.232_95).abs() < 1e-4, "{r:?}");
        assert!((r.wheel_radius[4] - (0.390_319 + 0.066_518)).abs() < 1e-4, "{r:?}");
        assert!(r.period > 4.0 && r.period < 6.0, "{r:?}");
        assert_eq!(r.edge_x, [-1.24, 1.24]);
    }

    /// Driving at the medium tank's max_speed (11.9 m/s): the threads move
    /// `v·k` cycles per second forward (≈ 61 for the left tread), so the
    /// tread images change every tick, the same rate on both sides.
    #[test]
    fn tread_rate_at_max_speed() {
        let Some(shape) = tank_shape(1) else {
            eprintln!("skipped: no game data");
            return;
        };
        let r = TreadRig::from_shape(&shape);
        let mut t = Treads::new(r);
        let v = 11.9;
        let rot = Mat3::IDENTITY;
        let mut cycles = [0.0f32; 2];
        let mut last = t.state.thread_pos(&r, 1.0);
        let ticks = (1.0 / tt_game::DT).round() as usize;
        let mut pos = Vec3::ZERO;
        for _ in 0..ticks {
            let next = pos + Vec3::new(0.0, v * tt_game::DT, 0.0);
            t.tick(true, (pos, rot), (next, rot));
            pos = next;
            // Several sub-tick samples (render frames) per tick.
            for k in 1..=8 {
                let p = t.state.thread_pos(&r, k as f32 / 8.0);
                for ((c, p), l) in cycles.iter_mut().zip(p).zip(last) {
                    *c += (p - l).rem_euclid(1.0);
                }
                last = p;
            }
        }
        let secs = ticks as f32 * tt_game::DT;
        for (i, c) in cycles.iter().enumerate() {
            let rate = c / secs;
            let expect = v * r.cycles_per_metre[i];
            // Accumulator wraps (period·k is not whole) add a fraction of a
            // cycle each; less than one per metre.
            assert!((rate - expect).abs() < 2.0 + v / r.period, "side {i}: {rate} cycles/s vs {expect}");
        }
    }
}
