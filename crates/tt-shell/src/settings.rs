//! The typed form of the prefs: `settings.ron` (plan D3: the saves are RON;
//! older saves are imported once, [`crate::legacy`]).
//!
//! Every setting has a key `section.field` (`video.max_fps`,
//! `server.max_players`; indexed ones `server.bot.3`): the name the
//! console's `set` / `get`, `--cmd` and the game code use. The settings the
//! game itself uses are typed fields in sections ([`Settings::player`],
//! [`Settings::video`], ...; a field's key is its section and field name,
//! the tables below), so a [`Prefs`](crate::Prefs) store converts to
//! settings and back without loss: a value becomes a typed field only when
//! the field writes it back as the same text ([`Settings::from_vars`]);
//! anything else — engine tuning nobody edits, values a typed field would
//! not reproduce — stays in [`Settings::other`] under its key. Unset fields
//! are absent, so the defaults still apply to them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::vars::{VarFile, VarValue};

/// The sections of the settings' keys (`misc` holds keys of no other
/// section). A key whose first part is one of these is a setting
/// ([`is_key`]); the game's run-time variables use other namespaces
/// (`game.`, `session.`, `mission.`, `sim.`, ...).
pub const SECTIONS: &[&str] = &[
    "player", "video", "graphics", "audio", "input", "host", "net", "server", "opengl", "shapes", "terrain", "lighting", "shell", "demo", "unix", "tank", "particle", "interior", "hud", "client", "game_data", "mods", "misc",
];

/// Whether `name` is a settings key (`section.field`, see [`SECTIONS`]).
pub fn is_key(name: &str) -> bool {
    name.split_once('.').is_some_and(|(s, f)| !f.is_empty() && SECTIONS.iter().any(|x| x.eq_ignore_ascii_case(s)))
}

/// A value of [`Settings::other`]: what the console variable held.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrefValue {
    Int(i32),
    Float(f32),
    Str(String),
}

impl From<&VarValue> for PrefValue {
    fn from(v: &VarValue) -> Self {
        match v {
            VarValue::Int(i) => PrefValue::Int(*i),
            VarValue::Float(f) => PrefValue::Float(*f),
            VarValue::Str(s) => PrefValue::Str(s.clone()),
        }
    }
}

impl From<&PrefValue> for VarValue {
    fn from(v: &PrefValue) -> Self {
        match v {
            PrefValue::Int(i) => VarValue::Int(*i),
            PrefValue::Float(f) => VarValue::Float(*f),
            PrefValue::Str(s) => VarValue::Str(s.clone()),
        }
    }
}

/// A screen resolution pref: "width height bpp".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution(pub u32, pub u32, pub u32);

/// A typed field's conversion from and to the console value.
pub trait PrefKind: Sized {
    fn from_var(v: &VarValue) -> Option<Self>;
    fn to_var(&self) -> VarValue;
}

impl PrefKind for String {
    fn from_var(v: &VarValue) -> Option<Self> {
        Some(v.as_string())
    }
    fn to_var(&self) -> VarValue {
        VarValue::Str(self.clone())
    }
}

impl PrefKind for bool {
    fn from_var(v: &VarValue) -> Option<Self> {
        match v.as_string().trim() {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        }
    }
    fn to_var(&self) -> VarValue {
        VarValue::Int(*self as i32)
    }
}

macro_rules! int_kind {
    ($($t:ty),*) => {$(
        impl PrefKind for $t {
            fn from_var(v: &VarValue) -> Option<Self> {
                v.as_string().trim().parse::<$t>().ok()
            }
            fn to_var(&self) -> VarValue {
                VarValue::Int(*self as i32)
            }
        }
    )*};
}
int_kind!(u8, u16, u32, i32);

impl PrefKind for f32 {
    fn from_var(v: &VarValue) -> Option<Self> {
        let s = v.as_string();
        s.trim().parse::<f64>().ok().map(|_| crate::vars::atof(&s) as f32)
    }
    fn to_var(&self) -> VarValue {
        VarValue::Float(*self)
    }
}

impl PrefKind for Resolution {
    fn from_var(v: &VarValue) -> Option<Self> {
        let s = v.as_string();
        let w: Vec<u32> = s.split(' ').map(|t| t.parse().ok()).collect::<Option<_>>()?;
        match w[..] {
            [a, b, c] => Some(Resolution(a, b, c)),
            _ => None,
        }
    }
    fn to_var(&self) -> VarValue {
        VarValue::Str(format!("{} {} {}", self.0, self.1, self.2))
    }
}

/// A tab-separated list.
impl PrefKind for Vec<String> {
    fn from_var(v: &VarValue) -> Option<Self> {
        let s = v.as_string();
        Some(if s.is_empty() { Vec::new() } else { s.split('\t').map(String::from).collect() })
    }
    fn to_var(&self) -> VarValue {
        VarValue::Str(self.join("\t"))
    }
}

/// Takes `name` out of `vars` as a `T` when `T` writes it back as the same
/// text (else it stays for [`Settings::other`]).
fn take<T: PrefKind>(vars: &mut VarFile, name: &str) -> Option<T> {
    let v = vars.value(name)?;
    let t = T::from_var(v)?;
    if t.to_var().as_string() != v.as_string() {
        return None;
    }
    vars.remove(name);
    Some(t)
}

macro_rules! section {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident : $ty:ty = $pref:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct $name {
            $($(#[$fm])* #[serde(skip_serializing_if = "Option::is_none")] pub $field: Option<$ty>,)*
        }

        impl $name {
            fn take(vars: &mut VarFile) -> $name {
                $name { $($field: take::<$ty>(vars, $pref),)* }
            }

            fn put(&self, vars: &mut VarFile) {
                $(if let Some(v) = &self.$field {
                    vars.set_value($pref, v.to_var());
                })*
            }

            pub fn is_empty(&self) -> bool {
                *self == $name::default()
            }
        }
    };
}

section! {
    /// The current player and the shell.
    PlayerSettings {
        name: String = "player.name",
        /// 0 Light, 1 Medium, 2 Heavy, 3 Baby Boss.
        tank: u8 = "player.tank",
        /// "" (auto), "GreenTeam", "BlueTeam".
        team: String = "player.team",
        unlock_baby_boss: bool = "player.unlock_baby_boss",
        play_count: u32 = "player.play_count",
        parental_lock: String = "player.parental_lock",
    }
}

section! {
    VideoSettings {
        resolution: Resolution = "video.resolution",
        sim_resolution: Resolution = "video.sim_resolution",
        shell_resolution: Resolution = "video.shell_resolution",
        full_screen: bool = "video.full_screen",
        /// "windowed", "fullscreen", "borderless".
        window_mode: String = "video.window_mode",
        disable_vertical_sync: bool = "video.disable_vertical_sync",
        /// 0..2.
        detail_level: u8 = "video.detail_level",
        render_scale: f32 = "video.render_scale",
        /// 0 = the monitor's refresh rate, -1 unlimited.
        max_fps: i32 = "video.max_fps",
        anisotropy: f32 = "video.anisotropy",
    }
}

section! {
    /// OpenTT's graphics extras (Options → Graphics).
    GraphicsSettings {
        glass_jar: bool = "graphics.glass_jar",
        hdr: bool = "graphics.hdr",
        bloom: f32 = "graphics.bloom",
        normal_maps: bool = "graphics.normal_maps",
        normal_strength: f32 = "graphics.normal_strength",
        ssao: u8 = "graphics.ssao",
        ao_strength: f32 = "graphics.ao_strength",
        shadows: u8 = "graphics.shadows",
        dynamic_lights: bool = "graphics.dynamic_lights",
        dynamic_light_strength: f32 = "graphics.dynamic_light_strength",
        show_fps: bool = "graphics.show_fps",
        hud_scale: f32 = "graphics.hud_scale",
    }
}

section! {
    AudioSettings {
        master_volume: f32 = "audio.master_volume",
        /// Audio channel 4.
        music_volume: f32 = "audio.music_volume",
        /// Audio channel 2 (Sim).
        sound_volume: f32 = "audio.sound_volume",
    }
}

section! {
    InputSettings {
        /// OpenTT's mouse slider.
        mouse_sensitivity: f32 = "input.mouse_sensitivity",
        link_mouse_sensitivity: f32 = "input.link_mouse_sensitivity",
        mouse_scale: f32 = "input.mouse_scale",
        invert_mouse: bool = "input.invert_mouse",
        reverse_steering: bool = "input.reverse_steering",
        /// 0 classic, 1 camera aim.
        aim_mode: u8 = "input.aim_mode",
        auto_turn: bool = "input.auto_turn",
    }
}

section! {
    /// The last hosted / custom game.
    HostSettings {
        mission: String = "host.mission",
        mode: u32 = "host.mode",
        minutes: u32 = "host.minutes",
    }
}

section! {
    NetSettings {
        master_server: String = "net.master_server",
        port_mapping: bool = "net.port_mapping",
        recent_servers: Vec<String> = "net.recent_servers",
        /// The default master servers already added to `master_server`
        /// (comma-separated; `Prefs::add_default_masters`).
        default_masters_added: String = "net.default_masters_added",
    }
}

section! {
    ServerSettings {
        name: String = "server.name",
        port: u16 = "server.port",
        max_players: u32 = "server.max_players",
        password: String = "server.password",
        /// "Always" or "Never".
        display_on_master: String = "server.display_on_master",
        max_bots: u32 = "server.max_bots",
        replace_bots: bool = "server.replace_bots",
        min_health_packs: u32 = "server.min_health_packs",
        /// The maps played in turn (mission ids; the first starts a hosted game).
        map_rotation: Vec<String> = "server.map_rotation",
        /// The rotation's maps come in a random order (off: in turn).
        map_rotation_random: bool = "server.map_rotation_random",
    }
}

section! {
    /// The mods (`docs/modding.md`).
    ModsSettings {
        /// The enabled mods' folder names in load order (later ones win).
        enabled: Vec<String> = "mods.enabled",
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BotSlot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tank: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
}

const BOT_FIELDS: [&str; 3] = ["server.bot.", "server.bot_name.", "server.bot_team."];

fn take_bots(vars: &mut VarFile) -> Vec<BotSlot> {
    let mut out = Vec::new();
    for i in 0.. {
        let names = BOT_FIELDS.map(|f| format!("{f}{i}"));
        if !names.iter().any(|n| vars.is_set(n)) {
            break;
        }
        let [tank, name, team] = names.map(|n| take::<String>(vars, &n));
        out.push(BotSlot { tank, name, team });
    }
    out
}

/// Every pref, typed: the RON form of a [`Prefs`](crate::Prefs) store.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(skip_serializing_if = "PlayerSettings::is_empty")]
    pub player: PlayerSettings,
    #[serde(skip_serializing_if = "VideoSettings::is_empty")]
    pub video: VideoSettings,
    #[serde(skip_serializing_if = "GraphicsSettings::is_empty")]
    pub graphics: GraphicsSettings,
    #[serde(skip_serializing_if = "AudioSettings::is_empty")]
    pub audio: AudioSettings,
    #[serde(skip_serializing_if = "InputSettings::is_empty")]
    pub input: InputSettings,
    #[serde(skip_serializing_if = "HostSettings::is_empty")]
    pub host: HostSettings,
    #[serde(skip_serializing_if = "NetSettings::is_empty")]
    pub net: NetSettings,
    #[serde(skip_serializing_if = "ServerSettings::is_empty")]
    pub server: ServerSettings,
    #[serde(skip_serializing_if = "ModsSettings::is_empty")]
    pub mods: ModsSettings,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bots: Vec<BotSlot>,
    /// Every other pref, by its console name (engine tuning, values the
    /// typed fields would not reproduce exactly).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub other: BTreeMap<String, PrefValue>,
}

impl Settings {
    /// The settings of a set of console variables (lossless, see the module docs).
    pub fn from_vars(vars: &VarFile) -> Settings {
        let mut v = vars.clone();
        let mut s = Settings {
            player: PlayerSettings::take(&mut v),
            video: VideoSettings::take(&mut v),
            graphics: GraphicsSettings::take(&mut v),
            audio: AudioSettings::take(&mut v),
            input: InputSettings::take(&mut v),
            host: HostSettings::take(&mut v),
            net: NetSettings::take(&mut v),
            server: ServerSettings::take(&mut v),
            mods: ModsSettings::take(&mut v),
            bots: take_bots(&mut v),
            other: BTreeMap::new(),
        };
        for n in v.names() {
            if let Some(val) = v.value(n) {
                s.other.insert(n.to_string(), val.into());
            }
        }
        s
    }

    /// The console variables (names as in the tables, or as in `other`).
    pub fn to_vars(&self) -> VarFile {
        let mut v = VarFile::new();
        self.player.put(&mut v);
        self.video.put(&mut v);
        self.graphics.put(&mut v);
        self.audio.put(&mut v);
        self.input.put(&mut v);
        self.host.put(&mut v);
        self.net.put(&mut v);
        self.server.put(&mut v);
        self.mods.put(&mut v);
        for (i, b) in self.bots.iter().enumerate() {
            for (f, val) in BOT_FIELDS.iter().zip([&b.tank, &b.name, &b.team]) {
                if let Some(val) = val {
                    v.set(&format!("{f}{i}"), val);
                }
            }
        }
        for (n, val) in &self.other {
            v.set_value(n, val.into());
        }
        v
    }

    pub fn to_ron(&self) -> String {
        crate::save::pretty(self, &format!("{} settings (keys: docs/layout.md § Settings).", tt_assets::product::NAME))
    }

    /// Reads `settings.ron`.
    pub fn from_ron(src: &str) -> Result<Settings, String> {
        ron::from_str(src).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_round_trip_is_lossless() {
        let src = "video.full_screen = \"1\";\r\nvideo.sim_resolution = \"1920 1080 32\";\r\naudio.master_volume = \"0.343125\";\r\ngraphics.normal_strength = \"0.1967264\";\r\nplayer.name = \"okkindel\";\r\nvideo.window_mode = \"windowed\";\r\nnet.recent_servers = \"10.0.0.1\t10.0.0.2\";\r\nmods.enabled = \"hd-skins\tnew-maps\";\r\nserver.bot.0 = \"GoldHeavyTank\";\r\nserver.bot_name.0 = \"Hum [Bot]\";\r\nserver.bot.1 = \"BronzeLightTank\";\r\nopengl.max_texture_dim = 512;\r\nvideo.disable_vertical_sync = \"true\";\r\nserver.port = 28000;\r\n";
        let mut vars = VarFile::new();
        for line in src.split("\r\n").filter(|l| !l.is_empty()) {
            let (n, v) = line.trim_end_matches(';').split_once(" = ").unwrap();
            let value = match v.strip_prefix('"') {
                Some(q) => VarValue::Str(q.trim_end_matches('"').into()),
                None => VarValue::Int(v.parse().unwrap()),
            };
            vars.set_value(n, value);
        }
        let s = Settings::from_vars(&vars);
        assert_eq!(s.video.full_screen, Some(true));
        assert_eq!(s.video.sim_resolution, Some(Resolution(1920, 1080, 32)));
        assert_eq!(s.audio.master_volume, Some(0.343125));
        assert_eq!(s.server.port, Some(28000));
        assert_eq!(s.net.recent_servers.as_deref(), Some(&["10.0.0.1".to_string(), "10.0.0.2".to_string()][..]));
        assert_eq!(s.mods.enabled.as_deref(), Some(&["hd-skins".to_string(), "new-maps".to_string()][..]));
        assert_eq!(s.bots.len(), 2);
        assert_eq!(s.bots[1], BotSlot { tank: Some("BronzeLightTank".into()), name: None, team: None });
        // Text a typed field would change stays as it was.
        assert_eq!(s.other.get("graphics.normal_strength"), Some(&PrefValue::Str("0.1967264".into())));
        assert_eq!(s.other.get("video.disable_vertical_sync"), Some(&PrefValue::Str("true".into())));
        assert_eq!(s.other.get("opengl.max_texture_dim"), Some(&PrefValue::Int(512)));
        let back = s.to_vars();
        for n in vars.names() {
            assert_eq!(back.get(n), vars.get(n), "{n}");
        }
        assert_eq!(back.names().count(), vars.names().count());
        let ron = s.to_ron();
        assert_eq!(Settings::from_ron(&ron).unwrap(), s, "{ron}");
    }
}
