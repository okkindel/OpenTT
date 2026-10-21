//! The prefs of the client and the server, with typed accessors for what the
//! menu edits.
//!
//! Setters store strings, as the engine's menu backend does today.

use crate::settings::Settings;
use crate::vars::{atof, num, VarFile, VarValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMode {
    Windowed,
    Fullscreen,
    Borderless,
}

const CLIENT_DEFAULTS: &[(&str, &str)] = &[
    ("player.name", "Bait"),
    ("player.tank", "0"),
    ("input.link_mouse_sensitivity", "0.5"),
    ("input.mouse_sensitivity", "0.25"),
    ("input.mouse_scale", "0.25"),
    // Off by default.
    ("input.reverse_steering", "0"),
    ("input.aim_mode", "1"),
    ("video.disable_vertical_sync", "0"),
    ("video.sim_resolution", "800 600 32"),
    ("video.shell_resolution", "800 600 32"),
    ("video.full_screen", "1"),
    ("video.detail_level", "2"),
    ("audio.sound_volume", "1"),
    ("audio.music_volume", "0.8"),
];

/// `mouseSensitivity` is OpenTT's mouse slider.
pub const PORT_DEFAULTS: &[(&str, &str)] = &[
    ("graphics.glass_jar", "1"),
    ("graphics.hdr", "1"),
    ("graphics.bloom", "0.7"),
    ("graphics.normal_maps", "1"),
    ("graphics.normal_strength", "0.2"),
    ("graphics.ssao", "1"),
    ("graphics.ao_strength", "0.5"),
    ("graphics.dynamic_lights", "1"),
    ("graphics.dynamic_light_strength", "1"),
    ("graphics.shadows", "1"),
    ("graphics.show_fps", "1"),
    ("graphics.hud_scale", "0.75"),
    ("input.aim_mode", "1"),
    ("input.invert_mouse", "0"),
    ("input.reverse_steering", "0"),
    ("input.mouse_sensitivity", "0.25"),
    ("video.disable_vertical_sync", "0"),
    ("video.detail_level", "2"),
    ("video.max_fps", "0"),
    // Hosting asks the router to forward the port (UPnP / NAT-PMP).
    ("net.port_mapping", "1"),
];

pub fn migrate_mouse_sensitivity(old: f64) -> f64 {
    0.25 + 0.75 * old.clamp(0.0, 1.0)
}

/// Defaults of the graphics extras (the [`PORT_DEFAULTS`] values).
pub struct GraphicsDefaults {
    pub glass_jar: bool,
    pub hdr: bool,
    pub bloom: f32,
    pub normal_maps: bool,
    /// "Surface detail": normal-map strength 0..2 (1 = 100 %).
    pub normal_strength: f32,
    pub ssao: u8,
    pub show_fps: bool,
    pub dynamic_lights: bool,
    /// AO strength 0..2.
    pub ao_strength: f32,
    pub shadows: u8,
}

pub const GRAPHICS_DEFAULTS: GraphicsDefaults = GraphicsDefaults { glass_jar: true, hdr: true, bloom: 0.7, normal_maps: true, normal_strength: 0.2, ssao: 1, show_fps: true, dynamic_lights: true, ao_strength: 0.5, shadows: 1 };

/// OpenTT's mouse slider pref (replaces `LinkMouseSensitivity`).
pub const MOUSE_SENSITIVITY: &str = "input.mouse_sensitivity";

/// The volume setting of audio channel `channel`: 2 (effects) is
/// `audio.sound_volume`, 4 (music) `audio.music_volume`, the others
/// `audio.channel_volume.<channel>`.
pub fn channel_volume_key(channel: u32) -> String {
    match channel {
        2 => "audio.sound_volume".into(),
        4 => "audio.music_volume".into(),
        c => format!("audio.channel_volume.{c}"),
    }
}

/// Default HUD size.
pub const HUD_SCALE_DEFAULT: f32 = 0.75;

/// Saved as [`Settings`] (`settings.ron`).
///
/// A pref the game first sets at run time ([`Prefs::set_value`]) is named
/// in lower case, as the settings files of the earlier builds have it.
#[derive(Debug, Clone, Default)]
pub struct Prefs {
    vars: VarFile,
}

impl PartialEq for Prefs {
    fn eq(&self, other: &Prefs) -> bool {
        self.vars == other.vars
    }
}

/// `dAtob`: "true" or a non-zero number.
fn atob(s: &str) -> bool {
    s.trim().eq_ignore_ascii_case("true") || atof(s) != 0.0
}

impl Prefs {
    pub fn defaults() -> Prefs {
        let mut p = Prefs::default();
        for (k, v) in CLIENT_DEFAULTS {
            p.vars.set(k, v);
        }
        p
    }

    /// The defaults, then the saved settings.
    pub fn with_settings(s: &Settings) -> Prefs {
        let mut p = Prefs::defaults();
        p.vars.merge(&s.to_vars());
        p
    }

    /// The typed form, for `settings.ron`.
    pub fn settings(&self) -> Settings {
        Settings::from_vars(&self.vars)
    }

    /// Every pref.
    pub fn vars(&self) -> &VarFile {
        &self.vars
    }

    /// Declares every variable of `vars` (names as written).
    pub fn merge(&mut self, vars: &VarFile) {
        for n in vars.names() {
            if let Some(v) = vars.value(n) {
                self.declare(n, v.clone());
            }
        }
    }

    pub fn value(&self, name: &str) -> Option<&VarValue> {
        self.vars.value(name)
    }

    pub fn is_set(&self, name: &str) -> bool {
        self.vars.is_set(name)
    }

    pub fn get(&self, name: &str) -> String {
        self.vars.get(name)
    }

    pub fn set(&mut self, name: &str, value: &str) {
        self.set_value(name, VarValue::Str(value.to_string()));
    }

    /// Sets a pref at run time (a new name is lower case, see [`Prefs`]).
    pub fn set_value(&mut self, name: &str, value: VarValue) {
        if self.vars.is_set(name) {
            self.vars.set_value(name, value);
        } else {
            self.vars.set_value(&name.to_ascii_lowercase(), value);
        }
    }

    /// Sets a pref with its name as written.
    pub fn declare(&mut self, name: &str, value: VarValue) {
        self.vars.set_value(name, value);
    }

    fn num_or(&self, name: &str, default: f32) -> f32 {
        if self.vars.is_set(name) {
            atof(&self.vars.get(name)) as f32
        } else {
            default
        }
    }

    fn flag_or(&self, name: &str, default: bool) -> bool {
        if self.vars.is_set(name) {
            atob(&self.vars.get(name))
        } else {
            default
        }
    }

    fn set_num(&mut self, name: &str, v: f32) {
        self.set(name, &num(v as f64));
    }

    fn set_flag(&mut self, name: &str, v: bool) {
        self.set(name, if v { "1" } else { "0" });
    }

    pub fn resolution(&self) -> (u32, u32) {
        let v: Vec<u32> = self.get("video.sim_resolution").split_whitespace().filter_map(|t| t.parse().ok()).collect();
        if v.len() >= 2 { (v[0], v[1]) } else { (800, 600) }
    }

    /// Sets both the sim and the shell resolution (the new menu is
    /// resolution independent).
    pub fn set_resolution(&mut self, w: u32, h: u32) {
        let r = format!("{w} {h} 32");
        self.set("video.sim_resolution", &r);
        self.set("video.shell_resolution", &r);
    }

    pub fn window_mode(&self) -> WindowMode {
        let full = self.flag_or("video.full_screen", true);
        match self.get("video.window_mode").to_ascii_lowercase().as_str() {
            "borderless" if full => WindowMode::Borderless,
            _ if full => WindowMode::Fullscreen,
            _ => WindowMode::Windowed,
        }
    }

    pub fn set_window_mode(&mut self, m: WindowMode) {
        self.set_flag("video.full_screen", m != WindowMode::Windowed);
        let label = match m {
            WindowMode::Windowed => "windowed",
            WindowMode::Fullscreen => "fullscreen",
            WindowMode::Borderless => "borderless",
        };
        self.set("video.window_mode", label);
    }

    pub fn vsync(&self) -> bool {
        !self.flag_or("video.disable_vertical_sync", false)
    }

    pub fn set_vsync(&mut self, on: bool) {
        self.set_flag("video.disable_vertical_sync", !on);
    }

    /// 0..2 (apply with `setThinkTanksDetailLevel`).
    pub fn detail_level(&self) -> u8 {
        (self.num_or("video.detail_level", 2.0) as i32).clamp(0, 2) as u8
    }

    pub fn set_detail_level(&mut self, l: u8) {
        self.set("video.detail_level", &l.min(2).to_string());
    }

    pub fn master_volume(&self) -> f32 {
        self.num_or("audio.master_volume", 1.0).clamp(0.0, 1.0)
    }

    pub fn set_master_volume(&mut self, v: f32) {
        self.set_num("audio.master_volume", v.clamp(0.0, 1.0));
    }

    pub fn music_volume(&self) -> f32 {
        self.num_or("audio.music_volume", 0.8).clamp(0.0, 1.0)
    }

    pub fn set_music_volume(&mut self, v: f32) {
        self.set_num("audio.music_volume", v.clamp(0.0, 1.0));
    }

    pub fn sound_volume(&self) -> f32 {
        self.num_or("audio.sound_volume", 1.0).clamp(0.0, 1.0)
    }

    pub fn set_sound_volume(&mut self, v: f32) {
        self.set_num("audio.sound_volume", v.clamp(0.0, 1.0));
    }

    pub fn mouse_sensitivity(&self) -> f32 {
        self.num_or(MOUSE_SENSITIVITY, 0.25).clamp(0.0, 1.0)
    }

    pub fn set_mouse_sensitivity(&mut self, v: f32) {
        self.set_num(MOUSE_SENSITIVITY, v.clamp(0.0, 1.0));
    }

    pub fn invert_mouse(&self) -> bool {
        self.flag_or("input.invert_mouse", false)
    }

    pub fn set_invert_mouse(&mut self, v: bool) {
        self.set_flag("input.invert_mouse", v);
    }

    /// Negate the player's turn input while the throttle is reversed.
    pub fn reverse_steering(&self) -> bool {
        self.flag_or("input.reverse_steering", false)
    }

    pub fn set_reverse_steering(&mut self, v: bool) {
        self.set_flag("input.reverse_steering", v);
    }

    /// 0 classic, 1 camera aim (default).
    pub fn aim_mode(&self) -> u8 {
        (self.num_or("input.aim_mode", 1.0) as i32).clamp(0, 1) as u8
    }

    pub fn set_aim_mode(&mut self, m: u8) {
        self.set("input.aim_mode", &m.min(1).to_string());
    }

    /// 0..3 (Light, Medium, Heavy, Baby Boss).
    pub fn tank_selection(&self) -> u8 {
        (self.num_or("player.tank", 0.0) as i32).clamp(0, 3) as u8
    }

    pub fn player_name(&self) -> String {
        self.get("player.name")
    }

    pub fn set_player_name(&mut self, name: &str) {
        self.set("player.name", name);
    }

    /// "" | "GreenTeam" | "BlueTeam".
    pub fn team(&self) -> String {
        self.get("player.team")
    }

    pub fn set_team(&mut self, team: &str) {
        self.set("player.team", team);
    }

    // Graphics extras.

    /// Glass brain jars (Fresnel, highlight, refraction).
    pub fn glass_jar(&self) -> bool {
        self.flag_or("graphics.glass_jar", GRAPHICS_DEFAULTS.glass_jar)
    }

    pub fn set_glass_jar(&mut self, v: bool) {
        self.set_flag("graphics.glass_jar", v);
    }

    /// HDR, tonemap, emissive glow and sun glow.
    pub fn hdr(&self) -> bool {
        self.flag_or("graphics.hdr", GRAPHICS_DEFAULTS.hdr)
    }

    pub fn set_hdr(&mut self, v: bool) {
        self.set_flag("graphics.hdr", v);
    }

    /// Bloom strength 0..1 (with HDR).
    pub fn bloom(&self) -> f32 {
        self.num_or("graphics.bloom", GRAPHICS_DEFAULTS.bloom).clamp(0.0, 1.0)
    }

    pub fn set_bloom(&mut self, v: f32) {
        self.set_num("graphics.bloom", v.clamp(0.0, 1.0));
    }

    /// Normal maps on terrain and shapes.
    pub fn normal_maps(&self) -> bool {
        self.flag_or("graphics.normal_maps", GRAPHICS_DEFAULTS.normal_maps)
    }

    pub fn set_normal_maps(&mut self, v: bool) {
        self.set_flag("graphics.normal_maps", v);
    }

    /// normal-map strength ("Surface detail") 0..2.
    pub fn normal_strength(&self) -> f32 {
        self.num_or("graphics.normal_strength", GRAPHICS_DEFAULTS.normal_strength).clamp(0.0, 2.0)
    }

    pub fn set_normal_strength(&mut self, v: f32) {
        self.set_num("graphics.normal_strength", v.clamp(0.0, 2.0));
    }

    /// explosions, shots and projectiles light their surroundings.
    pub fn dynamic_lights(&self) -> bool {
        self.flag_or("graphics.dynamic_lights", GRAPHICS_DEFAULTS.dynamic_lights)
    }

    pub fn set_dynamic_lights(&mut self, v: bool) {
        self.set_flag("graphics.dynamic_lights", v);
    }

    pub fn shadows(&self) -> u8 {
        (self.num_or("graphics.shadows", GRAPHICS_DEFAULTS.shadows as f32) as i32).clamp(0, 2) as u8
    }

    pub fn set_shadows(&mut self, v: u8) {
        self.set("graphics.shadows", &v.min(2).to_string());
    }

    /// Ambient occlusion strength 0..2.
    pub fn ao_strength(&self) -> f32 {
        self.num_or("graphics.ao_strength", GRAPHICS_DEFAULTS.ao_strength).clamp(0.0, 2.0)
    }

    pub fn set_ao_strength(&mut self, v: f32) {
        self.set_num("graphics.ao_strength", v.clamp(0.0, 2.0));
    }

    /// ("FPS limit"): 0 = the monitor's refresh rate, -1 = unlimited
    /// (without vsync), else frames per second.
    pub fn max_fps(&self) -> i32 {
        self.num_or("video.max_fps", 0.0) as i32
    }

    pub fn set_max_fps(&mut self, v: i32) {
        self.set("video.max_fps", &v.to_string());
    }

    /// HUD size 0.5..1.5 (1 = the default layout).
    pub fn hud_scale(&self) -> f32 {
        self.num_or("graphics.hud_scale", HUD_SCALE_DEFAULT).clamp(0.5, 1.5)
    }

    pub fn set_hud_scale(&mut self, v: f32) {
        self.set_num("graphics.hud_scale", v.clamp(0.5, 1.5));
    }

    /// Dynamic light strength 0..2.
    pub fn dynamic_light_strength(&self) -> f32 {
        self.num_or("graphics.dynamic_light_strength", 1.0).clamp(0.0, 2.0)
    }

    pub fn set_dynamic_light_strength(&mut self, v: f32) {
        self.set_num("graphics.dynamic_light_strength", v.clamp(0.0, 2.0));
    }

    /// The frame-rate counter in a corner.
    pub fn show_fps(&self) -> bool {
        self.flag_or("graphics.show_fps", GRAPHICS_DEFAULTS.show_fps)
    }

    pub fn set_show_fps(&mut self, v: bool) {
        self.set_flag("graphics.show_fps", v);
    }

    /// Ambient occlusion 0 off, 1 low, 2 high.
    pub fn ssao(&self) -> u8 {
        (self.num_or("graphics.ssao", GRAPHICS_DEFAULTS.ssao as f32) as i32).clamp(0, 2) as u8
    }

    pub fn set_ssao(&mut self, v: u8) {
        self.set("graphics.ssao", &v.min(2).to_string());
    }

    pub fn unlock_baby_boss(&self) -> bool {
        self.flag_or("player.unlock_baby_boss", true)
    }

    /// `isParental()`: the lock does not validate, or decodes to non-zero.
    pub fn is_parental(&self) -> bool {
        let code = self.get("player.parental_lock");
        tt_game::obfuscation::strfrap_decode(crate::IDLE_TEXT, &code).is_none_or(|v| atof(&v) != 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_accessors() {
        let mut p = Prefs::defaults();
        assert_eq!(p.resolution(), (800, 600));
        assert_eq!(p.window_mode(), WindowMode::Fullscreen);
        assert!(p.vsync(), "VSync on by default");
        assert_eq!(p.music_volume(), 0.8);
        p.set_resolution(1920, 1080);
        p.set_window_mode(WindowMode::Borderless);
        p.set_vsync(true);
        p.set_music_volume(0.25);
        p.set_detail_level(1);
        let q = Prefs::with_settings(&Settings::from_ron(&p.settings().to_ron()).unwrap());
        assert_eq!(q.resolution(), (1920, 1080));
        assert_eq!(q.window_mode(), WindowMode::Borderless);
        assert!(q.vsync());
        assert_eq!(q.music_volume(), 0.25);
        assert_eq!(q.detail_level(), 1);
        assert_eq!(p.settings().video.shell_resolution, Some(crate::settings::Resolution(1920, 1080, 32)));
        assert!(!p.reverse_steering());
        assert!(!Prefs::default().reverse_steering());
        p.set_reverse_steering(true);
        assert!(Prefs::with_settings(&Settings::from_ron(&p.settings().to_ron()).unwrap()).reverse_steering());
        // The other defaults of 2026-10-17.
        let d = Prefs::defaults();
        assert_eq!((d.mouse_sensitivity(), d.hud_scale(), d.bloom(), d.normal_strength(), d.ao_strength()), (0.25, 0.75, 0.7, 0.2, 0.5));
        assert!(d.show_fps() && d.vsync() && d.glass_jar());
        assert_eq!((d.ssao(), d.shadows(), d.aim_mode(), d.detail_level(), d.max_fps()), (1, 1, 1, 2, 0));
        assert_eq!(Prefs::defaults().aim_mode(), 1);
        assert_eq!(Prefs::default().aim_mode(), 1);
        p.set_aim_mode(0);
        assert_eq!(Prefs::with_settings(&Settings::from_ron(&p.settings().to_ron()).unwrap()).aim_mode(), 0);
        p.set_aim_mode(7);
        assert_eq!(p.aim_mode(), 1);
    }
}
