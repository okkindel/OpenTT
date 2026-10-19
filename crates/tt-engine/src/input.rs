//! Action maps: device/action bindings and input dispatch.
//!
//! The maps are named (`GlobalActionMap`, always active; `MoveMap`, active
//! in the game). Dispatch order is the engine's: `GlobalActionMap` first,
//! then (if the GUI did not consume the event) the active maps, most
//! recently pushed first. A key press calls the bound action with `1` and
//! the release with `0`; the release always goes to the binding that
//! handled the press, even if modifiers changed in between. Mouse axes call
//! the action with the (scaled) delta. A press-only binding (`bindCmd`)
//! runs its action on the press, without a value.

use std::collections::HashMap;
use tt_gameplay::client::binds::BindAction;

pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;

/// The engine's always-active map.
pub const GLOBAL_MAP: &str = "GlobalActionMap";
/// The game's map (pushed while `PlayGui` is up).
pub const MOVE_MAP: &str = "MoveMap";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Device {
    Keyboard,
    Mouse,
}

impl From<tt_gameplay::client::binds::Device> for Device {
    fn from(d: tt_gameplay::client::binds::Device) -> Device {
        match d {
            tt_gameplay::client::binds::Device::Keyboard => Device::Keyboard,
            tt_gameplay::client::binds::Device::Mouse => Device::Mouse,
        }
    }
}

pub fn canonical_key(s: &str) -> String {
    let k = s.to_ascii_lowercase();
    match k.as_str() {
        "enter" => "return",
        "esc" => "escape",
        "pgup" => "pageup",
        "pgdn" => "pagedown",
        "ctrl" | "control" => "lcontrol",
        "shift" => "lshift",
        "alt" => "lalt",
        "button" => "button0",
        "mouse0" | "lmb" => "button0",
        "mouse1" | "rmb" => "button1",
        "mouse2" | "mmb" => "button2",
        _ => return k,
    }
    .to_string()
}

fn modifier_bit(s: &str) -> Option<u8> {
    Some(match s.to_ascii_lowercase().as_str() {
        "shift" | "lshift" | "rshift" => SHIFT,
        "ctrl" | "control" | "lctrl" | "rctrl" | "lcontrol" | "rcontrol" => CTRL,
        "alt" | "lalt" | "ralt" => ALT,
        _ => return None,
    })
}

/// Parses an action string ("w", "shift a", "alt enter") into (modifiers, key).
pub fn parse_action(s: &str) -> Option<(u8, String)> {
    let words: Vec<&str> = s.split_whitespace().collect();
    let (key, mods) = words.split_last()?;
    let mut m = 0;
    for w in mods {
        m |= modifier_bit(w)?;
    }
    Some((m, canonical_key(key)))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `bind`: the action, called with the event value.
    Action { action: BindAction, inverted: bool, scale: f32 },
    /// `bindCmd`: the action on the press, without a value.
    Press(BindAction),
}

impl Command {
    pub fn action(&self) -> BindAction {
        match self {
            Command::Action { action, .. } | Command::Press(action) => *action,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub device: Device,
    pub modifiers: u8,
    pub key: String,
    /// The key with its modifiers as bound (for the menus).
    pub action: String,
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    /// Keyboard key or mouse button (`key` = canonical name, e.g. "a", "button0").
    Button { device: Device, key: String, pressed: bool },
    /// Mouse axis motion ("xaxis", "yaxis", "zaxis").
    Axis { device: Device, axis: String, delta: f32 },
}

#[derive(Default)]
pub struct Input {
    /// Bindings by map (lowercase map name).
    pub maps: HashMap<String, Vec<Binding>>,
    /// The active maps besides `GlobalActionMap`, in push order (lowercase).
    pub active: Vec<String>,
    /// Pressed button -> binding that handled its make event.
    held: HashMap<(Device, String), Binding>,
    pub modifiers: u8,
}

fn fmt_f(v: f32) -> String {
    tt_game::fmt::format_g(v as f64)
}

impl Input {
    /// The map exists (it has been bound to or created).
    pub fn has_map(&self, map: &str) -> bool {
        self.maps.contains_key(&map.to_ascii_lowercase())
    }

    /// `new ActionMap(map)`: an empty map (an existing one is replaced and
    /// leaves the active set).
    pub fn new_map(&mut self, map: &str) {
        let k = map.to_ascii_lowercase();
        self.active.retain(|m| *m != k);
        self.maps.insert(k, Vec::new());
    }

    /// `map.push()`: active, most recent first (nothing if no such map).
    pub fn push_map(&mut self, map: &str) {
        let k = map.to_ascii_lowercase();
        if !self.maps.contains_key(&k) {
            return;
        }
        self.active.retain(|m| *m != k);
        self.active.push(k);
    }

    /// `map.pop()`.
    pub fn pop_map(&mut self, map: &str) {
        let k = map.to_ascii_lowercase();
        self.active.retain(|m| *m != k);
    }

    /// The active maps' names (for reports).
    pub fn active_maps(&self) -> Vec<String> {
        self.active.clone()
    }

    /// Binds `action` (a key with modifiers: "w", "ctrl q", "xaxis") of
    /// `device` in `map` to `command`, replacing what it had.
    pub fn bind(&mut self, map: &str, device: Device, action: &str, command: Command) -> Result<(), String> {
        let (modifiers, key) = parse_action(action).ok_or_else(|| format!("ActionMap::bind: bad action {action:?}"))?;
        let list = self.maps.entry(map.to_ascii_lowercase()).or_default();
        list.retain(|o| !(o.device == device && o.modifiers == modifiers && o.key == key));
        list.push(Binding { device, modifiers, key, action: action.to_string(), command });
        Ok(())
    }

    /// The bindings of a map, in binding order.
    pub fn bindings(&self, map: &str) -> &[Binding] {
        self.maps.get(&map.to_ascii_lowercase()).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Tracks modifier state from raw key names.
    pub fn update_modifiers(&mut self, key: &str, pressed: bool) {
        let bit = match key {
            "lshift" | "rshift" => SHIFT,
            "lcontrol" | "rcontrol" => CTRL,
            "lalt" | "ralt" => ALT,
            _ => return,
        };
        if pressed {
            self.modifiers |= bit;
        } else {
            self.modifiers &= !bit;
        }
    }

    /// The maps an event goes through: `GlobalActionMap` (`global`) and/or
    /// the active maps, most recent first (`active`).
    pub fn dispatch_maps(&self, global: bool, active: bool) -> Vec<String> {
        let g = GLOBAL_MAP.to_ascii_lowercase();
        let mut maps: Vec<String> = Vec::new();
        if global && self.maps.contains_key(&g) {
            maps.push(g.clone());
        }
        if active {
            maps.extend(self.active.iter().rev().filter(|m| **m != g).cloned());
        }
        maps
    }

    /// Dispatches an event through `maps` (in priority order). Returns the
    /// actions to run with their values ("" for a press-only action; empty
    /// if nothing was bound).
    pub fn dispatch(&mut self, maps: &[String], ev: &InputEvent) -> Vec<(BindAction, String)> {
        match ev {
            InputEvent::Button { device, key, pressed } => {
                let key = canonical_key(key);
                if *device == Device::Keyboard {
                    self.update_modifiers(&key, *pressed);
                }
                if !*pressed {
                    let Some(b) = self.held.remove(&(*device, key.clone())) else { return Vec::new() };
                    return match b.command {
                        Command::Action { action, .. } => vec![(action, "0".into())],
                        Command::Press(_) => Vec::new(),
                    };
                }
                if self.held.contains_key(&(*device, key.clone())) {
                    return Vec::new(); // auto-repeat
                }
                // Modifier keys themselves match with their own bit cleared.
                let mods = match key.as_str() {
                    "lshift" | "rshift" => self.modifiers & !SHIFT,
                    "lcontrol" | "rcontrol" => self.modifiers & !CTRL,
                    "lalt" | "ralt" => self.modifiers & !ALT,
                    _ => self.modifiers,
                };
                for m in maps {
                    let Some(list) = self.maps.get(m) else { continue };
                    if let Some(b) = list.iter().find(|b| b.device == *device && b.key == key && b.modifiers == mods) {
                        let b = b.clone();
                        self.held.insert((*device, key), b.clone());
                        return match b.command {
                            Command::Action { action, .. } => vec![(action, "1".into())],
                            Command::Press(action) => vec![(action, String::new())],
                        };
                    }
                }
                Vec::new()
            }
            InputEvent::Axis { device, axis, delta } => {
                let axis = canonical_key(axis);
                for m in maps {
                    let Some(list) = self.maps.get(m) else { continue };
                    if let Some(b) = list.iter().find(|b| b.device == *device && b.key == axis && b.modifiers == self.modifiers) {
                        if let Command::Action { action, inverted, scale } = &b.command {
                            let mut v = *delta * scale;
                            if *inverted {
                                v = -v;
                            }
                            return vec![(*action, fmt_f(v))];
                        }
                        return Vec::new();
                    }
                }
                Vec::new()
            }
        }
    }
}

pub fn key_name(code: winit::keyboard::KeyCode) -> Option<&'static str> {
    use winit::keyboard::KeyCode::*;
    Some(match code {
        KeyA => "a",
        KeyB => "b",
        KeyC => "c",
        KeyD => "d",
        KeyE => "e",
        KeyF => "f",
        KeyG => "g",
        KeyH => "h",
        KeyI => "i",
        KeyJ => "j",
        KeyK => "k",
        KeyL => "l",
        KeyM => "m",
        KeyN => "n",
        KeyO => "o",
        KeyP => "p",
        KeyQ => "q",
        KeyR => "r",
        KeyS => "s",
        KeyT => "t",
        KeyU => "u",
        KeyV => "v",
        KeyW => "w",
        KeyX => "x",
        KeyY => "y",
        KeyZ => "z",
        Digit0 => "0",
        Digit1 => "1",
        Digit2 => "2",
        Digit3 => "3",
        Digit4 => "4",
        Digit5 => "5",
        Digit6 => "6",
        Digit7 => "7",
        Digit8 => "8",
        Digit9 => "9",
        F1 => "f1",
        F2 => "f2",
        F3 => "f3",
        F4 => "f4",
        F5 => "f5",
        F6 => "f6",
        F7 => "f7",
        F8 => "f8",
        F9 => "f9",
        F10 => "f10",
        F11 => "f11",
        F12 => "f12",
        Escape => "escape",
        Backquote => "tilde",
        Enter => "return",
        NumpadEnter => "numpadenter",
        Space => "space",
        Tab => "tab",
        Backspace => "backspace",
        ArrowUp => "up",
        ArrowDown => "down",
        ArrowLeft => "left",
        ArrowRight => "right",
        PageUp => "pageup",
        PageDown => "pagedown",
        Home => "home",
        End => "end",
        Insert => "insert",
        Delete => "delete",
        Numpad0 => "numpad0",
        Numpad1 => "numpad1",
        Numpad2 => "numpad2",
        Numpad3 => "numpad3",
        Numpad4 => "numpad4",
        Numpad5 => "numpad5",
        Numpad6 => "numpad6",
        Numpad7 => "numpad7",
        Numpad8 => "numpad8",
        Numpad9 => "numpad9",
        NumpadAdd => "add",
        NumpadSubtract => "subtract",
        NumpadMultiply => "multiply",
        NumpadDivide => "divide",
        NumpadDecimal => "decimal",
        Minus => "minus",
        Equal => "equals",
        BracketLeft => "lbracket",
        BracketRight => "rbracket",
        Backslash => "backslash",
        Semicolon => "semicolon",
        Quote => "apostrophe",
        Comma => "comma",
        Period => "period",
        Slash => "slash",
        ShiftLeft => "lshift",
        ShiftRight => "rshift",
        ControlLeft => "lcontrol",
        ControlRight => "rcontrol",
        AltLeft => "lalt",
        AltRight => "ralt",
        CapsLock => "capslock",
        PrintScreen => "print",
        Pause => "pause",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn key(k: &str, pressed: bool) -> InputEvent {
        InputEvent::Button { device: Device::Keyboard, key: k.into(), pressed }
    }

    #[test]
    fn parse_actions() {
        assert_eq!(parse_action("w"), Some((0, "w".into())));
        assert_eq!(parse_action("shift a"), Some((SHIFT, "a".into())));
        assert_eq!(parse_action("alt enter"), Some((ALT, "return".into())));
        assert_eq!(parse_action("ctrl alt F1"), Some((CTRL | ALT, "f1".into())));
        assert_eq!(parse_action("foo a"), None);
    }

    fn act(action: BindAction) -> Command {
        Command::Action { action, inverted: false, scale: 1.0 }
    }

    #[test]
    fn make_break_and_priority() {
        let mut inp = Input::default();
        inp.bind("a", Device::Keyboard, "w", act(BindAction::MoveForward)).unwrap();
        inp.bind("b", Device::Keyboard, "w", act(BindAction::MoveBackward)).unwrap();
        inp.bind("a", Device::Keyboard, "escape", Command::Press(BindAction::EscapeFromGame)).unwrap();
        assert!(inp.bind("a", Device::Keyboard, "foo q", act(BindAction::MoveForward)).is_err());
        let maps = s(&["b", "a"]);
        assert_eq!(inp.dispatch(&maps, &key("w", true)), vec![(BindAction::MoveBackward, "1".into())]);
        // Auto-repeat is ignored; the break goes to the same binding.
        assert!(inp.dispatch(&maps, &key("w", true)).is_empty());
        assert_eq!(inp.dispatch(&s(&["a"]), &key("w", false)), vec![(BindAction::MoveBackward, "0".into())]);
        assert_eq!(inp.dispatch(&maps, &key("escape", true)), vec![(BindAction::EscapeFromGame, String::new())]);
        assert!(inp.dispatch(&maps, &key("escape", false)).is_empty());
        assert!(inp.dispatch(&maps, &key("q", true)).is_empty());
    }

    #[test]
    fn modifiers_axes_and_maps() {
        let mut inp = Input::default();
        inp.bind("a", Device::Keyboard, "alt enter", Command::Press(BindAction::ToggleFullScreen)).unwrap();
        inp.bind("a", Device::Mouse, "xaxis", Command::Action { action: BindAction::Yaw, inverted: false, scale: 2.0 }).unwrap();
        inp.bind("a", Device::Mouse, "yaxis", Command::Action { action: BindAction::Pitch, inverted: true, scale: 1.0 }).unwrap();
        let a = s(&["a"]);
        assert!(inp.dispatch(&a, &key("return", true)).is_empty());
        inp.dispatch(&a, &key("return", false));
        inp.dispatch(&a, &key("lalt", true));
        assert_eq!(inp.dispatch(&a, &key("return", true)), vec![(BindAction::ToggleFullScreen, String::new())]);
        inp.dispatch(&a, &key("return", false));
        inp.dispatch(&a, &key("lalt", false));
        let ax = |x: &str, d| InputEvent::Axis { device: Device::Mouse, axis: x.into(), delta: d };
        assert_eq!(inp.dispatch(&a, &ax("xaxis", 1.5)), vec![(BindAction::Yaw, "3".into())]);
        assert_eq!(inp.dispatch(&a, &ax("yaxis", 2.0)), vec![(BindAction::Pitch, "-2".into())]);
        assert_eq!(inp.bindings("A").iter().map(|b| b.action.as_str()).collect::<Vec<_>>(), ["alt enter", "xaxis", "yaxis"]);
        // The active set: most recently pushed first, global first of all.
        inp.new_map(GLOBAL_MAP);
        inp.new_map(MOVE_MAP);
        inp.push_map("a");
        inp.push_map(MOVE_MAP);
        inp.push_map("nosuch");
        assert_eq!(inp.dispatch_maps(true, true), s(&["globalactionmap", "movemap", "a"]));
        inp.pop_map(MOVE_MAP);
        assert_eq!(inp.dispatch_maps(false, true), s(&["a"]));
    }
}
