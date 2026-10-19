//! The HUD itself is drawn natively (tt-menu `hud`, filled by the engine's `hud.rs`); tt-gui no longer draws
//! these controls.

use crate::geom::{Color, Point};

pub fn shape_name_opacity(dist: f32, visible_distance: f32, distance_fade: f32) -> f32 {
    let fade_start = visible_distance * distance_fade;
    if dist < fade_start {
        1.0
    } else {
        (1.0 - (dist - fade_start) / (visible_distance - fade_start).max(1e-6)).clamp(0.0, 1.0)
    }
}

/// Returns the height above the tank origin. The engine also skips the
/// control object's own tank and tanks whose line of sight from the camera
/// is blocked.
pub fn shape_name_height(box_height: f32, dist: f32, visible_distance: f32, min_offset: f32, max_offset: f32) -> f32 {
    box_height + min_offset + dist / visible_distance.max(1e-6) * (max_offset - min_offset)
}

/// One radar blip.
#[derive(Debug, Clone, PartialEq)]
pub struct RadarBlip {
    /// Position relative to the player in world units (x east, y north).
    pub dx: f32,
    pub dy: f32,
    pub color: Color,
    /// Drawn with `bigDotBitmap` at 1.5x the dot size.
    pub big: bool,
    /// Only drawn in the "on" half of a 600 ms blink cycle (`(time / 300) & 1`).
    pub blink: bool,
}

impl RadarBlip {
    /// Another (living) tank, in its team colour.
    pub fn tank(dx: f32, dy: f32, team_color: Color) -> RadarBlip {
        RadarBlip { dx, dy, color: team_color, big: false, blink: false }
    }
    /// The scrum ball (`Flag`): white, blinking.
    pub fn flag(dx: f32, dy: f32) -> RadarBlip {
        RadarBlip { dx, dy, color: Color::WHITE, big: false, blink: true }
    }
    /// A goal power-up (`type` starting with "g"): (1, 0, 0.8), big.
    pub fn goal(dx: f32, dy: f32, blink: bool) -> RadarBlip {
        RadarBlip { dx, dy, color: Color::rgba(255, 0, 204, 255), big: true, blink }
    }
    /// An ammo power-up (`type` "ammo"): (0.21, 0.777, 0.879), big.
    pub fn ammo(dx: f32, dy: f32, blink: bool) -> RadarBlip {
        RadarBlip { dx, dy, color: Color::rgba(54, 198, 224, 255), big: true, blink }
    }
    /// A health power-up (`type` "health"): red, big, blinking.
    pub fn health(dx: f32, dy: f32) -> RadarBlip {
        RadarBlip { dx, dy, color: Color::rgba(255, 0, 0, 255), big: true, blink: true }
    }
}

/// Radar parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadarParams {
    pub radius: f32,
    pub exponent: f32,
    pub inner_circle: f32,
    pub dot_offset: Point,
    pub dot_size: Point,
}

impl Default for RadarParams {
    fn default() -> Self {
        RadarParams { radius: 64.0, exponent: 1.0, inner_circle: 1.0, dot_offset: Point::new(4, 4), dot_size: Point::new(4, 4) }
    }
}

/// Blip position relative to the control's top left, for a control `extent` pixels
/// wide/high: distance `d = min(|(dx,dy)|, radius)`, screen radius `r = (d² +
/// 0.01)^(exponent/2) / radius^exponent * extent.x / 2 * innerCircle`, angle `a =
/// bearing(dx,dy) - heading`; the dot's top-left before centring is `(extent/2 -
/// dotOffset) + (r sin a, -r cos a)`.
pub fn radar_point(p: &RadarParams, extent: Point, heading_deg: f32, dx: f32, dy: f32) -> (f32, f32) {
    let d2 = (dx * dx + dy * dy).min(p.radius * p.radius);
    let norm = 1.0 / p.radius.powf(p.exponent);
    let r = (d2 + 0.01).powf(p.exponent * 0.5) * norm * extent.x as f32 * 0.5 * p.inner_circle;
    let bearing = dx.atan2(dy).to_degrees();
    let a = (bearing - heading_deg).to_radians();
    let x = (extent.x / 2 - p.dot_offset.x) as f32 + a.sin() * r;
    let y = (extent.y / 2 - p.dot_offset.y) as f32 - a.cos() * r;
    (x, y)
}

/// `GuiHudClock` text: `SS` under a minute unless `showMinutes`, `MM:SS`
/// under an hour unless `showHours`, else `HH:MM:SS`.
pub fn hud_clock_text(seconds: i32, show_minutes: bool, show_hours: bool) -> String {
    let t = seconds.max(0);
    let (h, m, s) = (t / 3600, (t % 3600) / 60, t % 60);
    if h == 0 && !show_hours {
        if m == 0 && !show_minutes {
            format!("{s:02}")
        } else {
            format!("{m:02}:{s:02}")
        }
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

/// `GuiHudClock` `showTime` rule: with `showTime >= 0` the clock is visible
/// only once `seconds <= showTime` (count-down) or `seconds >= showTime`
/// (count-up). `None` when `showTime < 0` (visibility left alone).
pub fn hud_clock_visible(seconds: i32, count_down: bool, show_time: i32) -> Option<bool> {
    (show_time >= 0).then_some(if count_down { seconds <= show_time } else { seconds >= show_time })
}

#[cfg(test)]
mod clock_tests {
    use super::*;

    #[test]
    fn clock_text() {
        assert_eq!(hud_clock_text(30, true, false), "00:30");
        assert_eq!(hud_clock_text(0, true, false), "00:00");
        assert_eq!(hud_clock_text(-3, true, false), "00:00");
        assert_eq!(hud_clock_text(61, true, false), "01:01");
        assert_eq!(hud_clock_text(900, true, false), "15:00");
        assert_eq!(hud_clock_text(3_725, true, false), "01:02:05");
        assert_eq!(hud_clock_text(30, false, false), "30");
        assert_eq!(hud_clock_text(30, false, true), "00:00:30");
    }
}
