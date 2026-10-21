//! Transform strings and the scripts' `position`/`rotation` fields.

use glam::{Affine3A, Mat3, Quat, Vec3};
use tt_assets::mission::SceneObject;
use tt_game::fmt::format_g;

pub(super) fn floats(s: &str) -> Vec<f32> {
    s.split_whitespace().map(tt_game::fields::atof).collect()
}

/// A mission object's transform from its `position`/`rotation`/`scale` fields.
pub fn object_transform(o: &SceneObject) -> Affine3A {
    let f = |n: &str| floats(o.get(n).unwrap_or(""));
    tt_game::geom::mission_transform(&f("position"), &f("rotation"), &f("scale"))
}

pub fn transform_string(pos: Vec3, rot: Mat3) -> String {
    let q = Quat::from_mat3(&rot).normalize();
    let (axis, angle) = q.to_axis_angle();
    let (axis, angle) = if angle.abs() < 1e-6 { (Vec3::Z, 0.0) } else { (axis, -angle) };
    [pos.x, pos.y, pos.z, axis.x, axis.y, axis.z, angle].iter().map(|v| format_g(*v as f64)).collect::<Vec<_>>().join(" ")
}

pub fn parse_transform(s: &str) -> (Vec3, Option<Mat3>) {
    let v = floats(s);
    let pos = Vec3::new(v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0));
    let rot = (v.len() >= 7).then(|| {
        let axis = Vec3::new(v[3], v[4], v[5]);
        if axis.length_squared() < 1e-12 {
            Mat3::IDENTITY
        } else {
            Mat3::from_quat(Quat::from_axis_angle(axis.normalize(), -v[6]))
        }
    });
    (pos, rot)
}

/// `obj.setTransform(t)` on an object's `position` / `rotation` fields
/// (the angle stored in degrees).
pub fn set_transform_fields(f: &mut (String, String), t: &str) {
    let v: Vec<f64> = t.split_whitespace().map(tt_game::fmt::atof).collect();
    let fmt = |v: &[f64]| v.iter().map(|x| format_g(*x)).collect::<Vec<_>>().join(" ");
    if v.len() >= 3 {
        f.0 = fmt(&v[..3]);
    }
    if v.len() >= 7 {
        f.1 = fmt(&[v[3], v[4], v[5], v[6].to_degrees()]);
    }
}

/// `getTransform()` of an object with these `position` / `rotation`
/// fields (radians again).
pub fn fields_transform(position: &str, rotation: &str) -> String {
    let p: Vec<f64> = position.split_whitespace().map(tt_game::fmt::atof).collect();
    let r: Vec<f64> = rotation.split_whitespace().map(tt_game::fmt::atof).collect();
    let p = [p.first().copied().unwrap_or(0.0), p.get(1).copied().unwrap_or(0.0), p.get(2).copied().unwrap_or(0.0)];
    let (axis, deg) = match r.as_slice() {
        [x, y, z, a, ..] if x * x + y * y + z * z > 0.0 => ([*x, *y, *z], *a),
        _ => ([1.0, 0.0, 0.0], 0.0),
    };
    [p[0], p[1], p[2], axis[0], axis[1], axis[2], deg.to_radians()].iter().map(|x| format_g(*x)).collect::<Vec<_>>().join(" ")
}

/// A tank's `position` / `rotation` fields as the tick writes them from
/// its pose.
pub fn tank_fields(pos: Vec3, rot: Mat3) -> (String, String) {
    let s = transform_string(pos, rot);
    let mut parts = s.splitn(4, ' ');
    let p = format!("{} {} {}", parts.next().unwrap_or("0"), parts.next().unwrap_or("0"), parts.next().unwrap_or("0"));
    let r: Vec<f64> = parts.next().unwrap_or("0 0 1 0").split(' ').map(tt_game::fmt::atof).collect();
    let r = if r.len() == 4 { format!("{} {} {} {}", format_g(r[0]), format_g(r[1]), format_g(r[2]), format_g(r[3].to_degrees())) } else { "1 0 0 0".into() };
    (p, r)
}

/// A tank's `getTransform()` (its synced fields read back).
pub fn tank_script_transform(pos: Vec3, rot: Mat3) -> String {
    let (p, r) = tank_fields(pos, rot);
    fields_transform(&p, &r)
}

/// case-insensitive "red" 0, "green" 1, "blue" 2, "purple" 3; any other
/// string leaves the team unchanged (`None`).
pub fn team_index(color: &str) -> Option<i32> {
    match color.to_ascii_lowercase().as_str() {
        "red" => Some(0),
        "green" => Some(1),
        "blue" => Some(2),
        "purple" => Some(3),
        _ => None,
    }
}

/// `rotation` field (degrees) as the radians of a transform string.
pub fn rotation_rad(r: &str) -> String {
    let v: Vec<f64> = r.split_whitespace().map(tt_game::fmt::atof).collect();
    match v.as_slice() {
        [x, y, z, a] => format!("{x} {y} {z} {}", a.to_radians()),
        _ => "0 0 1 0".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_strings_roundtrip() {
        let rot = Mat3::from_quat(Quat::from_axis_angle(Vec3::Z, -0.7));
        let s = transform_string(Vec3::new(1.0, 2.0, 3.0), rot);
        let (p, r) = parse_transform(&s);
        assert_eq!(p, Vec3::new(1.0, 2.0, 3.0));
        assert!((r.unwrap().y_axis - rot.y_axis).length() < 1e-4);
        let (_, r) = parse_transform("0 0 0 0 0 1 0.7");
        assert!(r.unwrap().y_axis.x > 0.6);
    }
}
