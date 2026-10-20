//! Key bindings and the player's move: the action maps as typed data and
//! the move the bound actions build.
//!
//! The move keeps each value as the script variable held it ([`SVal`]):
//! the engine read the variables as text, so a mouse yaw summed in `F32`
//! reached the move through its `%g` text, and so it does here.

use serde::{Deserialize, Serialize};

use super::text::{atof, atoi, f32r, fs, lf, SVal};

/// An input device of a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Device {
    Keyboard,
    Mouse,
}

impl Device {
    pub fn name(self) -> &'static str {
        match self {
            Device::Keyboard => "keyboard",
            Device::Mouse => "mouse",
        }
    }
}

/// What a binding does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BindAction {
    EscapeFromGame,
    MoveLeft,
    MoveRight,
    MoveForward,
    MoveBackward,
    MoveUp,
    MoveDown,
    TurnLeft,
    TurnRight,
    PanUp,
    PanDown,
    /// Mouse x: turret (or camera) yaw.
    Yaw,
    /// Mouse y: pitch.
    Pitch,
    Fire,
    AltFire,
    ToggleFreeLook,
    ToggleFirstPerson,
    ToggleCamera,
    /// The chat input to everyone (`toggleMessageHud`).
    Chat,
    /// The chat input to the team (`teamMessageHud`).
    TeamChat,
    PageMessagesUp,
    PageMessagesDown,
    ResizeMessages,
    StartDemoRecording,
    StopDemoRecording,
    DropCameraAtPlayer,
    DropPlayerAtCamera,
    /// Respawn when dead (`sendSpawnCommand`).
    Spawn,
    CycleDebugRenderMode,
    ToggleConsole,
    ToggleFullScreen,
    /// F1: help / pause menu (`toggleHelpGui`).
    ToggleHelp,
    ToggleLogMode,
    ToggleScriptTrace,
    DoProfile,
    CycleRenderMode,
    CycleTank,
    TogglePlayerAi,
    Quit,
    Screenshot,
}

impl BindAction {
    /// Every action.
    pub const ALL: &'static [BindAction] = &[
        BindAction::EscapeFromGame,
        BindAction::MoveLeft,
        BindAction::MoveRight,
        BindAction::MoveForward,
        BindAction::MoveBackward,
        BindAction::MoveUp,
        BindAction::MoveDown,
        BindAction::TurnLeft,
        BindAction::TurnRight,
        BindAction::PanUp,
        BindAction::PanDown,
        BindAction::Yaw,
        BindAction::Pitch,
        BindAction::Fire,
        BindAction::AltFire,
        BindAction::ToggleFreeLook,
        BindAction::ToggleFirstPerson,
        BindAction::ToggleCamera,
        BindAction::Chat,
        BindAction::TeamChat,
        BindAction::PageMessagesUp,
        BindAction::PageMessagesDown,
        BindAction::ResizeMessages,
        BindAction::StartDemoRecording,
        BindAction::StopDemoRecording,
        BindAction::DropCameraAtPlayer,
        BindAction::DropPlayerAtCamera,
        BindAction::Spawn,
        BindAction::CycleDebugRenderMode,
        BindAction::ToggleConsole,
        BindAction::ToggleFullScreen,
        BindAction::ToggleHelp,
        BindAction::ToggleLogMode,
        BindAction::ToggleScriptTrace,
        BindAction::DoProfile,
        BindAction::CycleRenderMode,
        BindAction::CycleTank,
        BindAction::TogglePlayerAi,
        BindAction::Quit,
        BindAction::Screenshot,
    ];

    /// The console function the action map calls (the scripts' name).
    pub fn function(self) -> &'static str {
        use BindAction::*;
        match self {
            EscapeFromGame => "escapeFromGame",
            MoveLeft => "moveleft",
            MoveRight => "moveright",
            MoveForward => "moveforward",
            MoveBackward => "movebackward",
            MoveUp => "moveup",
            MoveDown => "movedown",
            TurnLeft => "turnLeft",
            TurnRight => "turnRight",
            PanUp => "panUp",
            PanDown => "panDown",
            Yaw => "yaw",
            Pitch => "pitch",
            Fire => "mouseFire",
            AltFire => "altTrigger",
            ToggleFreeLook => "toggleFreeLook",
            ToggleFirstPerson => "toggleFirstPerson",
            ToggleCamera => "toggleCamera",
            Chat => "toggleMessageHud",
            TeamChat => "teamMessageHud",
            PageMessagesUp => "pageMessageHudUp",
            PageMessagesDown => "pageMessageHudDown",
            ResizeMessages => "resizeMessageHud",
            StartDemoRecording => "startRecordingDemo",
            StopDemoRecording => "stopRecordingDemo",
            DropCameraAtPlayer => "dropCameraAtPlayer",
            DropPlayerAtCamera => "dropPlayerAtCamera",
            Spawn => "sendSpawnCommand",
            CycleDebugRenderMode => "cycleDebugRenderMode",
            ToggleConsole => "toggleConsole",
            ToggleFullScreen => "toggleFullScreen",
            ToggleHelp => "toggleHelpGui",
            ToggleLogMode => "toggleLogMode",
            ToggleScriptTrace => "toggleScriptTrace",
            DoProfile => "doProfile",
            CycleRenderMode => "cycleTTRenderMode",
            CycleTank => "cycleTank",
            TogglePlayerAi => "clientTogglePlayerAI",
            Quit => "ttQuit2",
            Screenshot => "doScreenShot",
        }
    }

    /// Runs on the key press only, without a value (`bindCmd`).
    pub fn press_only(self) -> bool {
        matches!(self, BindAction::EscapeFromGame | BindAction::ToggleFullScreen | BindAction::ToggleHelp | BindAction::CycleTank)
    }

    /// The action of a bound script function, numbered copies
    /// included.
    pub fn from_function(name: &str) -> Option<BindAction> {
        let n = name.trim().to_ascii_lowercase();
        let base = match n.as_str() {
            "moveleft2" | "moveleft3" => "moveleft",
            "moveright2" | "moveright3" => "moveright",
            "moveforward2" | "moveforward3" => "moveforward",
            "movebackward2" | "movebackward3" | "movebackward4" => "movebackward",
            "togglemessagehud2" | "togglemessagehud3" | "togglemessagehud4" => "togglemessagehud",
            "ttquit" => "ttquit2",
            other => other,
        };
        BindAction::ALL.iter().copied().find(|a| a.function().eq_ignore_ascii_case(base))
    }
}

/// When a binding of the defaults exists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum When {
    #[default]
    Always,
    /// Developer builds only (`!isRelease()`).
    Debug,
    /// Developer builds or the attract-mode demo.
    DebugOrDemo,
    /// On macOS / elsewhere.
    Mac,
    NotMac,
}

/// One binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub device: Device,
    /// The key with modifiers, as the action map names it ("w", "ctrl q",
    /// "button0", "xaxis").
    pub key: String,
    pub action: BindAction,
    #[serde(default, skip_serializing_if = "is_default")]
    pub when: When,
    /// Axis scale (`bind(..., "S", scale, ...)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    /// Inverted axis (`"I"`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub invert: bool,
}

fn is_default(w: &When) -> bool {
    *w == When::Always
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// An action map and its bindings, in binding order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindMap {
    /// `GlobalActionMap` (always active) or `MoveMap` (in the game).
    pub name: String,
    pub binds: Vec<Binding>,
}

/// All action maps.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Binds {
    pub maps: Vec<BindMap>,
}

/// Where a binding applies (`release`: no developer bindings).
pub fn applies(w: When, release: bool, demo: bool, mac: bool) -> bool {
    match w {
        When::Always => true,
        When::Debug => !release,
        When::DebugOrDemo => !release || demo,
        When::Mac => mac,
        When::NotMac => !mac,
    }
}

impl Binds {
    pub fn parse(text: &str) -> Result<Binds, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_ron(&self) -> String {
        let cfg = ron::ser::PrettyConfig::new().depth_limit(3).struct_names(false);
        ron::ser::to_string_pretty(self, cfg).unwrap_or_default()
    }

    pub fn map(&self, name: &str) -> Option<&BindMap> {
        self.maps.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }

    fn map_mut(&mut self, name: &str) -> &mut BindMap {
        if self.map(name).is_none() {
            self.maps.push(BindMap { name: name.to_string(), binds: Vec::new() });
        }
        self.maps.iter_mut().find(|m| m.name.eq_ignore_ascii_case(name)).unwrap()
    }

    /// `user`'s maps replace these (a map the user file lacks stays).
    pub fn overlay(&mut self, user: Binds) {
        for m in user.maps {
            let name = m.name.clone();
            *self.map_mut(&name) = m;
        }
    }

    /// Adds or replaces (same device and key) a binding of `map`.
    pub fn bind(&mut self, map: &str, b: Binding) {
        let m = self.map_mut(map);
        let k = normal_key(&b.key);
        m.binds.retain(|o| !(o.device == b.device && normal_key(&o.key) == k));
        m.binds.push(b);
    }

    /// Removes a binding (`bind(..., "")` / `unbind`).
    pub fn unbind(&mut self, map: &str, device: Device, key: &str) {
        let k = normal_key(key);
        self.map_mut(map).binds.retain(|o| !(o.device == device && normal_key(&o.key) == k));
    }

    /// Returns the lines that could not be imported.
    pub fn import_config_cs(&mut self, text: &str) -> Vec<String> {
        let mut skipped = Vec::new();
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with("//") {
                continue;
            }
            if !self.import_line(l) {
                skipped.push(l.to_string());
            }
        }
        skipped
    }

    fn import_line(&mut self, l: &str) -> bool {
        let Some(dot) = l.find('.') else { return false };
        let map = l[..dot].trim();
        let rest = &l[dot + 1..];
        let Some(open) = rest.find('(') else { return false };
        let method = rest[..open].trim().to_ascii_lowercase();
        let Some(close) = rest.rfind(')') else { return false };
        let args = split_args(&rest[open + 1..close]);
        let map = if map.eq_ignore_ascii_case("movemap") { "MoveMap" } else if map.eq_ignore_ascii_case("globalactionmap") { "GlobalActionMap" } else { map };
        let device = |s: &str| {
            let d = s.trim_end_matches(|c: char| c.is_ascii_digit()).to_ascii_lowercase();
            match d.as_str() {
                "keyboard" => Some(Device::Keyboard),
                "mouse" => Some(Device::Mouse),
                _ => None,
            }
        };
        match method.as_str() {
            "bind" if args.len() >= 3 => {
                let Some(dev) = device(&args[0]) else { return false };
                let func = args.last().unwrap();
                if func.is_empty() {
                    self.unbind(map, dev, &args[1]);
                    return true;
                }
                let Some(action) = BindAction::from_function(func) else { return false };
                let (mut scale, mut invert) = (None, false);
                if args.len() > 3 {
                    let mut params = args[3..args.len() - 1].iter();
                    for f in args[2].to_ascii_uppercase().chars() {
                        match f {
                            'I' => invert = true,
                            'S' => scale = params.next().map(|s| atof(s) as f32),
                            'D' => {
                                params.next();
                            }
                            _ => {}
                        }
                    }
                }
                self.bind(map, Binding { device: dev, key: args[1].clone(), action, when: When::Always, scale, invert });
                true
            }
            "bindcmd" if args.len() >= 3 => {
                let Some(dev) = device(&args[0]) else { return false };
                let make = args[2].trim().trim_end_matches(';').trim();
                let func = make.strip_suffix("()").or_else(|| make.split('(').next()).unwrap_or("");
                let Some(action) = BindAction::from_function(func) else { return false };
                self.bind(map, Binding { device: dev, key: args[1].clone(), action, when: When::Always, scale: None, invert: false });
                true
            }
            _ => false,
        }
    }
}

/// Key text compared as the action map does (modifier order kept, case-insensitive).
fn normal_key(k: &str) -> String {
    k.split_whitespace().map(|w| w.to_ascii_lowercase()).collect::<Vec<_>>().join(" ")
}

/// Arguments of a script call: comma separated, `"..."` quoted (with `\"`).
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// One of the eight move "action" values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveAxis {
    Left,
    Right,
    Forward,
    Backward,
    Up,
    Down,
}

/// A keyboard turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnKey {
    Left,
    Right,
    Up,
    Down,
}

/// The script's aim limit (the literal `3.14`, not π).
#[allow(clippy::approx_constant)]
const AIM_LIMIT: f64 = 3.14;

/// The local player's move in the making.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MoveState {
    pub left: SVal,
    pub right: SVal,
    pub forward: SVal,
    pub backward: SVal,
    pub up: SVal,
    pub down: SVal,
    pub yaw_left_speed: SVal,
    pub yaw_right_speed: SVal,
    pub pitch_up_speed: SVal,
    pub pitch_down_speed: SVal,
    pub yaw: SVal,
    pub pitch: SVal,
    pub roll: SVal,
    pub triggers: [SVal; 2],
    pub free_look: bool,
    pub movement_speed: SVal,
}

pub const MOUSE_SENSITIVITY_PREF: &str = "input.mouse_sensitivity";
/// 25 %: the game's lowest setting.
pub const DEFAULT_MOUSE_SENSITIVITY: f64 = 0.25;

/// The game's `getMouseAdjustAmount` curve (radians per count / fov
/// factor) at its `LinkMouseSensitivity` value `old` (0..1).
fn original_factor(old: f64) -> f64 {
    if 0.5 > old {
        f32r(0.0065 + 0.01 * old)
    } else {
        f32r(0.023 * old)
    }
}

/// Radians per mouse count (at a 90° field of view) for the sensitivity
/// slider `s` (0..1): from 25 % up the game's curve over its whole range
/// (25 % = its minimum 0.0065, 100 % = its maximum 0.023); below 25 %
/// down linearly to a tenth of its minimum at 0 %.
pub fn mouse_factor(s: f64) -> f64 {
    if s >= DEFAULT_MOUSE_SENSITIVITY {
        original_factor((s - DEFAULT_MOUSE_SENSITIVITY) / (1.0 - DEFAULT_MOUSE_SENSITIVITY))
    } else {
        original_factor(0.0) * (0.1 + 0.9 * s / DEFAULT_MOUSE_SENSITIVITY)
    }
}

/// The mouse settings an axis action reads (the prefs, as script values
/// read with `LOADVAR_FLT`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseSettings {
    /// The sensitivity slider, 0..1 ([`MOUSE_SENSITIVITY_PREF`]; see
    /// [`mouse_factor`]).
    pub sensitivity: f64,
    pub camera_fov: f64,
    pub invert: f64,
}

impl MoveState {
    fn axis(&mut self, a: MoveAxis) -> &mut SVal {
        match a {
            MoveAxis::Left => &mut self.left,
            MoveAxis::Right => &mut self.right,
            MoveAxis::Forward => &mut self.forward,
            MoveAxis::Backward => &mut self.backward,
            MoveAxis::Up => &mut self.up,
            MoveAxis::Down => &mut self.down,
        }
    }

    pub fn set_axis(&mut self, a: MoveAxis, val: &str) {
        *self.axis(a) = SVal::Str(val.to_string());
    }

    /// The throttle and turn of the next moves directly (`forward`, `right`).
    pub fn set_drive(&mut self, forward: f32, right: f32) {
        self.forward = SVal::Str(forward.to_string());
        self.right = SVal::Str(right.to_string());
    }

    pub fn turn(&mut self, k: TurnKey, val: &str, keyboard_turn_speed: &str) {
        let v = SVal::Str(if lf(val) != 0.0 { keyboard_turn_speed.to_string() } else { "0".into() });
        match k {
            TurnKey::Left => self.yaw_right_speed = v,
            TurnKey::Right => self.yaw_left_speed = v,
            TurnKey::Up => self.pitch_down_speed = v,
            TurnKey::Down => self.pitch_up_speed = v,
        }
    }

    /// `getMouseAdjustAmount(val)` as its returned text (with the
    /// rescaled sensitivity, [`mouse_factor`]).
    pub fn mouse_adjust(val: &str, m: &MouseSettings) -> String {
        fs(lf(val) * (m.camera_fov / 90.0) * mouse_factor(m.sensitivity))
    }

    fn add_clamped(v: &mut SVal, amount: f64) {
        let mut x = SVal::Float((v.float() + amount) as f32);
        if x.float() < -AIM_LIMIT {
            x = SVal::Float(-AIM_LIMIT as f32);
        } else if x.float() > AIM_LIMIT {
            x = SVal::Float(AIM_LIMIT as f32);
        }
        *v = x;
    }

    /// `yaw(val)`: mouse x.
    pub fn mouse_yaw(&mut self, val: &str, m: &MouseSettings) {
        let a = atof(&Self::mouse_adjust(val, m));
        Self::add_clamped(&mut self.yaw, a);
    }

    /// `pitch(val)`: mouse y.
    pub fn mouse_pitch(&mut self, val: &str, m: &MouseSettings) {
        let factor = if m.invert != 0.0 { f32r(-1.0) } else { 1.0 };
        let a = atof(&Self::mouse_adjust(val, m));
        Self::add_clamped(&mut self.pitch, a * factor);
    }

    pub fn trigger(&mut self, n: usize) {
        let t = &mut self.triggers[n];
        *t = SVal::Float((t.float() + 1.0) as f32);
    }

    /// `setSpeed(v)`.
    pub fn set_speed(&mut self, v: &str) {
        if lf(v) != 0.0 {
            self.movement_speed = SVal::Str(v.to_string());
        }
    }

    /// The mouse aim not yet in a move.
    pub fn aim(&self) -> (f32, f32) {
        (atof(&self.yaw.string()) as f32, atof(&self.pitch.string()) as f32)
    }

    /// The mouse aim taken by the free camera.
    pub fn clear_aim(&mut self) {
        self.yaw = SVal::Str("0".into());
        self.pitch = SVal::Str("0".into());
    }

    /// The player's move for a tick (TT `getNextMove`): the values read as
    /// the engine read the variables (their text), yaw/pitch/roll reset.
    /// `reverse_steering` (, docs/tank-physics.md §3.3): the turn is
    /// negated while backing up. `trigger[0]` fires on an odd count or a
    /// change since `last_trigger_count`.
    pub fn take_move(&mut self, reverse_steering: bool, last_trigger_count: &mut i32) -> tt_game::Move {
        let g = |v: &SVal| atof(&v.string()) as f32;
        let mut x = g(&self.right) - g(&self.left);
        let y = g(&self.forward) - g(&self.backward);
        if y < 0.0 && reverse_steering {
            x = -x;
        }
        let yaw = g(&self.yaw) + g(&self.yaw_left_speed) - g(&self.yaw_right_speed);
        let pitch = g(&self.pitch) + g(&self.pitch_up_speed) - g(&self.pitch_down_speed);
        for v in [&mut self.yaw, &mut self.pitch, &mut self.roll] {
            *v = SVal::Str("0".into());
        }
        let count = atoi(&self.triggers[0].string());
        let fire = count % 2 != 0 || count != *last_trigger_count;
        *last_trigger_count = count;
        let mut trigger = [false; 6];
        trigger[0] = fire;
        tt_game::Move { x, y, z: 0.0, yaw, pitch, roll: 0.0, trigger }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_names_round_trip() {
        for &a in BindAction::ALL {
            assert_eq!(BindAction::from_function(a.function()), Some(a), "{a:?}");
        }
    }

    /// The default bindings of the asset directory (game data).
    fn defaults() -> String {
        std::fs::read_to_string(tt_assets::layout::default_dir().join("data/binds.ron")).unwrap()
    }

    #[test]
    fn defaults_parse_and_round_trip() {
        tt_assets::require_game_data!();
        let b = Binds::parse(&defaults()).unwrap();
        let mv = b.map("MoveMap").unwrap();
        assert!(mv.binds.iter().any(|x| x.key == "w" && x.action == BindAction::MoveForward));
        assert_eq!(Binds::parse(&b.to_ron()).unwrap(), b);
    }

    #[test]
    fn config_cs_import() {
        tt_assets::require_game_data!();
        let mut b = Binds::parse(&defaults()).unwrap();
        let skipped = b.import_config_cs("moveMap.bind(keyboard, \"i\", moveforward2);\nmoveMap.bind(keyboard, \"w\", \"\");\nGlobalActionMap.bindCmd(keyboard, \"F2\", \"toggleHelpGui();\", \"\");\nmoveMap.bind(mouse0, \"xaxis\", S, 0.5, yaw);\nfoo();");
        assert_eq!(skipped, vec!["foo();".to_string()]);
        let mv = b.map("MoveMap").unwrap();
        assert!(mv.binds.iter().any(|x| x.key == "i" && x.action == BindAction::MoveForward));
        assert!(!mv.binds.iter().any(|x| x.key == "w"));
        assert_eq!(mv.binds.iter().find(|x| x.key == "xaxis").unwrap().scale, Some(0.5));
        assert!(b.map("GlobalActionMap").unwrap().binds.iter().any(|x| x.key == "F2" && x.action == BindAction::ToggleHelp));
    }

    #[test]
    fn sensitivity_scale() {
        // 25 % is the game's minimum, 100 % its maximum, exactly.
        assert_eq!(mouse_factor(0.25), original_factor(0.0));
        assert_eq!(mouse_factor(0.25), 0.0065f32 as f64);
        assert_eq!(mouse_factor(1.0), original_factor(1.0));
        assert_eq!(mouse_factor(1.0), 0.023f32 as f64);
        // 0 % is a tenth of the minimum; the scale rises throughout.
        assert!((mouse_factor(0.0) - 0.00065f32 as f64).abs() < 1e-9);
        let v: Vec<f64> = (0..=20).map(|i| mouse_factor(i as f64 / 20.0)).collect();
        assert!(v.windows(2).all(|w| w[1] > w[0]), "{v:?}");
        for old in [0.0, 0.2, 0.5, 1.0] {
            assert_eq!(mouse_factor(0.25 + 0.75 * old), original_factor(old), "{old}");
        }
    }

    #[test]
    fn moves_like_the_variables() {
        let mut m = MoveState::default();
        let mut last = 0;
        m.set_axis(MoveAxis::Forward, "1");
        m.set_axis(MoveAxis::Left, "0.5");
        let s = MouseSettings { sensitivity: 1.0, camera_fov: 90.0, invert: 0.0 };
        m.mouse_yaw("3", &s);
        let mv = m.take_move(true, &mut last);
        assert_eq!((mv.x, mv.y), (-0.5, 1.0));
        assert_eq!(mv.yaw, atof(&fs(3.0 * 0.023f32 as f64)) as f32);
        assert_eq!(m.yaw, SVal::Str("0".into()));
        assert!(!mv.trigger[0]);
        m.trigger(0);
        assert!(m.take_move(true, &mut last).trigger[0]);
        m.set_axis(MoveAxis::Forward, "0");
        m.set_axis(MoveAxis::Backward, "1");
        assert_eq!(m.take_move(true, &mut last).x, 0.5);
        m.turn(TurnKey::Left, "1", "0.1");
        assert_eq!(m.take_move(false, &mut last).yaw, -0.1);
        for _ in 0..100 {
            m.mouse_yaw("1000", &s);
        }
        #[allow(clippy::approx_constant)]
        let expected = SVal::Float(3.14);
        assert_eq!(m.yaw, expected);
    }
}
