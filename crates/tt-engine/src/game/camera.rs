//! The chase camera and camera aim's free camera.

use super::{AimMode, FreeAim, GameSim};
use glam::{Affine3A, Mat3, Quat, Vec3};
use tt_game::TankId;
use tt_gameplay::{EntityId, EntityKind};

/// How far up the view really goes is also limited by keeping the tank on
/// screen (see [`free_view`]).
pub const FREE_PITCH_RANGE: (f32, f32) = (-0.6, 1.45);

/// Free camera: the lowest elevation of the eye seen from the pivot above
/// the tank (radians); looking further up turns only the view.
pub const FREE_ARM_MIN_ELEVATION: f32 = 0.0;

/// Free camera: how far inside the bottom edge of the view the pivot above
/// the tank is kept (radians), so the tank stays visible.
pub const FREE_TANK_MARGIN: f32 = 0.03;

/// Half the vertical field of view assumed until a frame tells the real one
/// (90° horizontal at 16:9).
pub const DEFAULT_HALF_VFOV: f32 = 0.513;

/// Clearance of the free camera's eye over the terrain, metres.
pub const FREE_CAM_CLEARANCE: f32 = 1.0;

/// How far the free camera's aim ray looks for the point under the screen
/// centre, metres (beyond it the aim point is on the ray at that distance).
pub const FREE_AIM_RANGE: f32 = 500.0;

/// Heading of a direction (radians, + = clockwise from above; 0 = +y).
pub fn heading_of(v: Vec3) -> f32 {
    v.x.atan2(v.y)
}

/// An angle wrapped to -π..π.
pub fn wrap_pi(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

#[derive(Debug, Clone, Copy)]
pub struct CameraParams {
    pub focus_dist: f32,
    pub focus_up_dist: f32,
    pub backup_dist: f32,
    pub float_height: f32,
    pub smooth: f32,
    pub smooth_turn: f32,
    pub smooth_up: f32,
    pub fov: f32,
    /// The camera never goes below the mission floor.
    pub floor: f32,
}

impl Default for CameraParams {
    fn default() -> Self {
        CameraParams { focus_dist: 10.1, focus_up_dist: 0.9, backup_dist: 6.2, float_height: 6.22, smooth: 0.33, smooth_turn: 0.77, smooth_up: 0.75, fov: 90.0, floor: -100.0 }
    }
}

/// Smoothed chase-camera state kept per tank.
#[derive(Debug, Clone, Default)]
pub struct ChaseCam {
    prev: Option<(Vec3, Vec3, Vec3, f64)>,
    /// Free camera (camera aim): smoothed eye height over the tank and its time.
    free_z: Option<(f32, f64)>,
}

impl ChaseCam {
    /// Returns the camera transform (columns right, look, up; translation
    /// = eye). `now_ms` is real time (fractional: at hundreds of frames
    /// per second whole milliseconds would make the smoothing step
    /// unevenly).
    pub fn update(&mut self, now_ms: f64, pos: Vec3, rot: Mat3, cd: &CameraParams, ground: impl Fn(f32, f32) -> Option<f32>) -> Affine3A {
        self.free_z = None;
        let (mut r, mut u) = (rot.x_axis, rot.z_axis);
        let (snap, t, prev_cam) = match self.prev {
            Some((pr, pu, pc, last)) if now_ms - last <= 100.0 => {
                let t = ((now_ms - last).max(0.0) / 100.0) as f32;
                let f = cd.smooth_turn.clamp(0.1, 1.0).powf(t);
                r = r.lerp(pr, f).try_normalize().unwrap_or(r);
                let f = cd.smooth_up.clamp(0.1, 1.0).powf(t);
                u = u.lerp(pu, f).try_normalize().unwrap_or(u);
                (false, t, Some(pc))
            }
            _ => (true, 0.0, None),
        };
        let fwd = if snap { rot.y_axis } else { u.cross(r).try_normalize().unwrap_or(rot.y_axis) };
        let focus = pos + fwd * cd.focus_dist + Vec3::Z * cd.focus_up_dist;
        let mut cam = pos - fwd * cd.backup_dist;
        let gz = ground(cam.x, cam.y).unwrap_or(f32::MIN);
        cam.z = cam.z.max(gz) + cd.float_height;
        cam.z = cam.z.max(cd.floor);
        if let Some(pc) = prev_cam.filter(|pc| (cam - *pc).length() <= 50.0) {
            let f = cd.smooth.clamp(0.1, 1.0).powf(t);
            cam = cam.lerp(pc, f);
        }
        self.prev = Some((r, u, cam, now_ms));
        let look = (focus - cam).try_normalize().unwrap_or(fwd);
        let right = Vec3::new(look.y, -look.x, 0.0).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(look);
        Affine3A::from_cols(right.into(), look.into(), up.into(), cam.into())
    }

    /// Camera aim: the free camera `aim`, see [`free_view`]. Yaw and pitch are the
    /// mouse's, unsmoothed.
    pub fn free(&mut self, now_ms: f64, pos: Vec3, aim: &FreeAim, cd: &CameraParams, ground: impl Fn(f32, f32) -> Option<f32>) -> (Affine3A, f32) {
        let (yaw, pitch, half_vfov) = (aim.yaw, aim.pitch, aim.half_vfov);
        let mut h = free_eye_height(pos, yaw, cd, &ground);
        if let Some((prev, last)) = self.free_z.filter(|&(prev, last)| now_ms - last <= 100.0 && (prev - h).abs() <= 50.0) {
            let t = ((now_ms - last).max(0.0) / 100.0) as f32;
            h += (prev - h) * cd.smooth.clamp(0.1, 1.0).powf(t);
        }
        self.free_z = Some((h, now_ms));
        // The hull camera starts over (snapped) when camera aim is left.
        self.prev = None;
        free_view(pos, yaw, pitch, h, half_vfov, cd, ground)
    }
}

/// Free camera eye height over the tank before smoothing: the game's
/// `cam_float_height` over the ground (or the tank) behind it.
pub fn free_eye_height(pos: Vec3, yaw: f32, cd: &CameraParams, ground: impl Fn(f32, f32) -> Option<f32>) -> f32 {
    let fwd = Vec3::new(yaw.sin(), yaw.cos(), 0.0);
    let cam = pos - fwd * cd.backup_dist;
    let gz = ground(cam.x, cam.y).unwrap_or(f32::MIN);
    (cam.z.max(gz) + cd.float_height).max(cd.floor) - pos.z
}

/// Camera aim's free camera. Returns the view and the pitch it shows (`pitch`
/// less what the tank-visibility limit took off).
///
/// `pitch` (+ = up) turns the whole rig (eye and look) about a pivot
/// `cam_focus_up_dist` above the tank, around the view's right axis: looking
/// down raises the eye, looking up lowers it behind the tank, until the eye is
/// level with the pivot ([`FREE_ARM_MIN_ELEVATION`]); further up only the view
/// turns and the tank drops lower in the frame. The eye is kept
/// [`FREE_CAM_CLEARANCE`] over the terrain (lifted), and the view is tilted
/// back down where the pivot would come closer than [`FREE_TANK_MARGIN`] to the
/// bottom edge (`half_vfov` = half the vertical field of view), so the tank
/// never leaves the screen.
pub fn free_view(pos: Vec3, yaw: f32, pitch: f32, eye_height: f32, half_vfov: f32, cd: &CameraParams, ground: impl Fn(f32, f32) -> Option<f32>) -> (Affine3A, f32) {
    let fwd = Vec3::new(yaw.sin(), yaw.cos(), 0.0);
    let right = Vec3::new(fwd.y, -fwd.x, 0.0);
    let elev = |v: Vec3| v.normalize_or_zero().z.clamp(-1.0, 1.0).asin();
    let focus = pos + fwd * cd.focus_dist + Vec3::Z * cd.focus_up_dist;
    let cam0 = pos - fwd * cd.backup_dist + Vec3::Z * eye_height;
    let pivot = pos + Vec3::Z * cd.focus_up_dist;
    let orbit = pitch.min((elev(cam0 - pivot) - FREE_ARM_MIN_ELEVATION).max(0.0));
    let q = Quat::from_axis_angle(right, orbit);
    let mut cam = pivot + q * (cam0 - pivot);
    let mut look = Quat::from_axis_angle(right, pitch) * (focus - cam0).try_normalize().unwrap_or(fwd);
    if let Some(g) = ground(cam.x, cam.y) {
        cam.z = cam.z.max(g + FREE_CAM_CLEARANCE);
    }
    // The pivot's angle below the view centre.
    let below = elev(look) - elev(pivot - cam);
    let excess = below - (half_vfov - FREE_TANK_MARGIN).max(0.1);
    let mut shown = pitch;
    // Only looking up is limited.
    let excess = excess.min(pitch);
    if excess > 0.0 {
        look = Quat::from_axis_angle(right, -excess) * look;
        shown -= excess;
    }
    let up = right.cross(look);
    (Affine3A::from_cols(right.into(), look.into(), up.into(), cam.into()), shown)
}

impl GameSim {
    /// Camera def of a tank's def (`camera = DefaultTankCamera`).
    pub fn camera_data(&self, t: TankId) -> CameraParams {
        let cd = CameraParams { floor: self.world.floor, ..CameraParams::default() };
        let Some(cam) = self.def_fields(EntityKind::Tank(t)).and_then(|d| d.get("camera").map(str::to_string)).and_then(|c| self.db.fields(&c)) else { return cd };
        let f = |n: &str, d: f32| match cam.get(n) {
            Some(v) if !v.is_empty() => tt_game::fields::atof(v),
            _ => d,
        };
        CameraParams {
            focus_dist: f("cam_focus_dist", cd.focus_dist),
            focus_up_dist: f("cam_focus_up_dist", cd.focus_up_dist),
            backup_dist: f("cam_backup_dist", cd.backup_dist),
            float_height: f("cam_float_height", cd.float_height),
            smooth: f("cam_smooth", cd.smooth),
            smooth_turn: f("cam_smooth_turn", cd.smooth_turn),
            smooth_up: f("cam_smooth_up", cd.smooth_up),
            fov: f("cam_fov", cd.fov),
            floor: cd.floor,
        }
    }

    /// Chase-camera transform of a tank object at real time `now_ms`, from
    /// its pose interpolated by `alpha`. In camera aim the player's tank
    /// has the free camera.
    pub fn camera_transform(&mut self, aim: AimMode, obj: EntityId, now_ms: f64, alpha: f32) -> Option<Affine3A> {
        let id = self.tank_of(obj)?;
        let t = self.world.tank(id)?;
        let (pos, rot) = self.render_pose(t, alpha);
        let cd = self.camera_data(id);
        let terrain = self.world.terrain.clone();
        let ground = |x: f32, y: f32| terrain.as_ref().map(|h| h.height(x, y));
        let cam = self.cameras.entry(id).or_default();
        if self.free_aim.tank == Some(id) && aim == AimMode::Camera {
            let a = self.free_aim;
            let (m, pitch) = cam.free(now_ms, pos, &a, &cd, ground);
            // What the view could not take (tank out of sight) is dropped,
            // so turning back down responds at once.
            self.free_aim.pitch = pitch;
            return Some(m);
        }
        Some(cam.update(now_ms, pos, rot, &cd, ground))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chase_camera_behind_and_above() {
        let mut c = ChaseCam::default();
        let cd = CameraParams::default();
        let m = c.update(0.0, Vec3::ZERO, Mat3::IDENTITY, &cd, |_, _| Some(0.0));
        let eye: Vec3 = m.translation.into();
        assert!((eye - Vec3::new(0.0, -6.2, 6.22)).length() < 1e-4);
        assert!(m.matrix3.y_axis.y > 0.5);
        // Smoothing keeps part of the previous position after 32 ms.
        let m2 = c.update(32.0, Vec3::new(0.0, 5.0, 0.0), Mat3::IDENTITY, &cd, |_, _| Some(0.0));
        assert!(m2.translation.y < 5.0 - 6.2);
        // A gap over 100 ms snaps.
        let m3 = c.update(500.0, Vec3::new(0.0, 5.0, 0.0), Mat3::IDENTITY, &cd, |_, _| Some(0.0));
        assert!((m3.translation.y - (5.0 - 6.2)).abs() < 1e-4);
    }

    /// Heading (radians, + = clockwise from above) of a camera's look.
    fn heading(m: &Affine3A) -> f32 {
        heading_of(m.matrix3.y_axis.into())
    }

    /// Pitch (radians, + = up) of a camera's look.
    fn look_pitch(m: &Affine3A) -> f32 {
        m.matrix3.y_axis.z.clamp(-1.0, 1.0).asin()
    }

    #[test]
    fn free_camera_turns_with_the_mouse_at_once() {
        let cd = CameraParams::default();
        let flat = |_: f32, _: f32| Some(0.0);
        let mut c = ChaseCam::default();
        let aim = |yaw: f32| FreeAim { tank: None, yaw, pitch: 0.0, half_vfov: DEFAULT_HALF_VFOV };
        let m = c.free(0.0, Vec3::ZERO, &aim(0.7), &cd, flat).0;
        let o = ChaseCam::default().update(0.0, Vec3::ZERO, Mat3::from_rotation_z(-0.7), &cd, flat);
        assert!((Vec3::from(m.translation) - Vec3::from(o.translation)).length() < 1e-4);
        assert!((Vec3::from(m.matrix3.y_axis) - Vec3::from(o.matrix3.y_axis)).length() < 1e-4);
        // The next frame looks wherever the mouse turned it, any amount (no
        // smoothing, no limit), even all the way round.
        for (i, yaw) in [0.75f32, -2.0, 3.0, 0.1].into_iter().enumerate() {
            let m = c.free(16.0 * (i + 1) as f64, Vec3::ZERO, &aim(yaw), &cd, flat).0;
            assert!(wrap_pi(heading(&m) - yaw).abs() < 1e-4, "{yaw}: {}", heading(&m));
            let eye: Vec3 = m.translation.into();
            assert!((eye.truncate().length() - cd.backup_dist).abs() < 1e-3);
        }
    }

    #[test]
    fn free_camera_orbits_up_and_down() {
        let cd = CameraParams::default();
        let flat = |_: f32, _: f32| Some(0.0);
        let cam = |p: f32| free_view(Vec3::ZERO, 0.3, p, cd.float_height, DEFAULT_HALF_VFOV, &cd, flat);
        let (level, _) = cam(0.0);
        let pivot = Vec3::Z * cd.focus_up_dist;
        // The pivot's angle below the view centre.
        let below = |m: &Affine3A| look_pitch(m) - (pivot - Vec3::from(m.translation)).normalize().z.asin();
        for p in [FREE_PITCH_RANGE.0, -0.2, 0.2, 0.4] {
            let (m, shown) = cam(p);
            // The view pitches by as much, its heading stays.
            assert!((look_pitch(&m) - (look_pitch(&level) + p)).abs() < 1e-4 && shown == p, "{p}: {}", look_pitch(&m));
            assert!((heading(&m) - 0.3).abs() < 1e-4);
            // The eye orbits the pivot above the tank: same distance, lower
            // when looking up, higher when looking down; the tank keeps its
            // place on screen.
            let (eye, eye0): (Vec3, Vec3) = (m.translation.into(), level.translation.into());
            assert!(((eye - pivot).length() - (eye0 - pivot).length()).abs() < 1e-3);
            assert!(if p > 0.0 { eye.z < eye0.z - 0.5 } else { eye.z > eye0.z + 0.5 }, "{p}: {} vs {}", eye.z, eye0.z);
            assert!((below(&m) - below(&level)).abs() < 1e-3, "{p}");
        }
        // All the way up: the eye stops level with the pivot, the view looks
        // well above the horizon, and turns only as far as keeps the tank
        // in the frame (more with a taller view).
        let (m, shown) = cam(FREE_PITCH_RANGE.1);
        let eye: Vec3 = m.translation.into();
        assert!((eye.z - pivot.z.max(FREE_CAM_CLEARANCE)).abs() < 1e-3, "{eye}");
        assert!(look_pitch(&m) > 0.15 && shown < FREE_PITCH_RANGE.1, "{} {shown}", look_pitch(&m));
        assert!((below(&m) - (DEFAULT_HALF_VFOV - FREE_TANK_MARGIN)).abs() < 1e-3);
        let (tall, _) = free_view(Vec3::ZERO, 0.3, FREE_PITCH_RANGE.1, cd.float_height, 0.64, &cd, flat);
        assert!(look_pitch(&tall) > look_pitch(&m) + 0.1);
    }

    #[test]
    fn camera_stays_at_the_mission_floor() {
        // A tank 20 m down in the pit under floor 95: the eye stays at 95.
        let cd = CameraParams { floor: 95.0, ..CameraParams::default() };
        let pit = |_: f32, _: f32| Some(75.0);
        assert_eq!(free_eye_height(Vec3::new(0.0, 0.0, 75.0), 0.0, &cd, pit), 20.0);
        let m = ChaseCam::default().update(0.0, Vec3::new(0.0, 0.0, 75.0), Mat3::IDENTITY, &cd, pit);
        assert_eq!(m.translation.z, 95.0);
    }

    #[test]
    fn free_camera_stays_above_the_terrain() {
        let cd = CameraParams::default();
        // A ridge behind the tank (the camera looks along +y): the ground
        // rises 1 m per metre behind y = -2.
        let ridge = |_: f32, y: f32| Some((-2.0 - y).max(0.0));
        let h = free_eye_height(Vec3::ZERO, 0.0, &cd, ridge);
        let (m0, _) = free_view(Vec3::ZERO, 0.0, 0.0, h, DEFAULT_HALF_VFOV, &cd, ridge);
        let base = look_pitch(&m0);
        let pivot = Vec3::Z * cd.focus_up_dist;
        let mut prev: Option<Vec3> = None;
        for i in 0..=60 {
            let p = FREE_PITCH_RANGE.0 + (FREE_PITCH_RANGE.1 - FREE_PITCH_RANGE.0) * i as f32 / 60.0;
            let (m, _) = free_view(Vec3::ZERO, 0.0, p, h, DEFAULT_HALF_VFOV, &cd, ridge);
            let eye: Vec3 = m.translation.into();
            let g = ridge(eye.x, eye.y).unwrap();
            assert!(eye.z >= g + FREE_CAM_CLEARANCE - 1e-3, "{p}: eye {eye} over ground {g}");
            // Looking down pitches by as much; looking up looks higher than
            // level, with the tank still in the frame.
            if p <= 0.0 {
                assert!((look_pitch(&m) - (base + p)).abs() < 1e-3, "{p}");
            } else {
                assert!(look_pitch(&m) > base, "{p}: {} <= {base}", look_pitch(&m));
                let below = look_pitch(&m) - (pivot - eye).normalize().z.asin();
                assert!(below <= DEFAULT_HALF_VFOV - FREE_TANK_MARGIN + 1e-3, "{p}: tank at {below}");
            }
            if let Some(q) = prev {
                assert!((eye - q).length() < 0.8, "{p}: {q} -> {eye}");
            }
            prev = Some(eye);
        }
    }
}
