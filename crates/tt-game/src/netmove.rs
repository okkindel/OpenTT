//! Network move quantisation.
//!
//! What the server simulates for a player is the *unpacked* move: x and y
//! collapse to −1/0/+1, yaw and pitch are truncated to a signed 16-bit
//! 2π/65536 grid, only `trigger[0]` survives, and z/roll are not sent.

use std::f32::consts::PI;

use crate::tank::Move;

/// 65536/2π and 2π/65536 as the f32 constants.
const TO_U16: f32 = 65536.0 / (2.0 * PI);
const FROM_U16: f32 = 2.0 * PI / 65536.0;

/// `clamp` of an angle: truncate towards zero, keep 16 bits.
pub fn angle_to_u16(a: f32) -> u16 {
    ((a as f64 * TO_U16 as f64).trunc() as i64 & 0xFFFF) as u16
}

/// `unclamp` of an angle: signed 16-bit value times 2π/65536.
pub fn u16_to_angle(v: u16) -> f32 {
    (v as i16) as f32 * FROM_U16
}

/// `clamp` of x/y: 0..=32 with 16 = centre.
pub fn axis_to_u5(v: f32) -> u8 {
    if v < -1.0 {
        0
    } else if v > 1.0 {
        32
    } else {
        ((v as f64 + 1.0) * 16.0).trunc() as u8
    }
}

/// What the client predicts with (`clamp` then `unclamp`).
pub fn client_move(mv: &Move) -> Move {
    let axis = |v: f32| (axis_to_u5(v) as f32 - 16.0) / 16.0;
    let mut out = Move::NULL;
    out.x = axis(mv.x);
    out.y = axis(mv.y);
    out.yaw = u16_to_angle(angle_to_u16(mv.yaw));
    out.pitch = u16_to_angle(angle_to_u16(mv.pitch));
    out.trigger = mv.trigger;
    out.z = mv.z;
    out.roll = mv.roll;
    out
}

/// What the server receives after `pack`/`unpack`: use this for player input.
pub fn server_move(mv: &Move) -> Move {
    let axis = |v: f32| match axis_to_u5(v) {
        16 => 0.0,
        p if p > 16 => 1.0,
        _ => -1.0,
    };
    let mut out = Move::NULL;
    out.x = axis(mv.x);
    out.y = axis(mv.y);
    out.yaw = u16_to_angle(angle_to_u16(mv.yaw));
    out.pitch = u16_to_angle(angle_to_u16(mv.pitch));
    out.trigger[0] = mv.trigger[0];
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_collapse_to_three_values() {
        let m = |x: f32, y: f32| server_move(&Move { x, y, ..Move::NULL });
        assert_eq!((m(0.5, -0.5).x, m(0.5, -0.5).y), (1.0, -1.0));
        assert_eq!(m(0.05, 0.0).x, 0.0); // (0.05+1)·16 = 16.8 → 16
        assert_eq!(m(-0.05, 0.0).x, -1.0); // 15.2 → 15
        assert_eq!(m(7.0, -7.0).x, 1.0);
        assert_eq!(client_move(&Move { x: 0.5, ..Move::NULL }).x, 0.5);
        assert_eq!(client_move(&Move { x: 0.53, ..Move::NULL }).x, 0.5);
    }

    #[test]
    fn angles_truncate_on_16_bit_grid() {
        let step = 2.0 * PI / 65536.0;
        let q = server_move(&Move { yaw: 2.5 * step, pitch: -2.5 * step, ..Move::NULL });
        assert!((q.yaw - 2.0 * step).abs() < 1e-9);
        assert!((q.pitch + 2.0 * step).abs() < 1e-9);
        // Signed: anything past ±π wraps.
        let q = server_move(&Move { yaw: 3.2, ..Move::NULL });
        assert!(q.yaw < 0.0);
        assert!((server_move(&Move { yaw: 1.0, ..Move::NULL }).yaw - 1.0).abs() < step);
    }

    #[test]
    fn only_trigger_zero_and_no_z_roll() {
        let q = server_move(&Move { z: 1.0, roll: 0.3, trigger: [true, true, true, false, false, true], ..Move::NULL });
        assert_eq!(q.trigger, [true, false, false, false, false, false]);
        assert_eq!((q.z, q.roll), (0.0, 0.0));
    }
}
