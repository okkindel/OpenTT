//! The key bindings as an earlier build saved them in `binds.ron`. The
//! bindings file is now tt-gameplay's typed `Binds` (docs/layout.md § User
//! data); tt-engine's start-up migration converts a file of this format
//! through [`UserBinds::to_config_cs`].

use serde::{Deserialize, Serialize};

/// One binding of an action map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Binding {
    /// `map.bind(device, key, [flags, values...,] command)`: `args` are the
    /// arguments after the key, as written (the command last).
    Bind { device: String, key: String, args: Vec<String> },
    /// `map.bindCmd(device, key, makeCmd, breakCmd)`.
    BindCmd { device: String, key: String, make: String, brk: String },
}

/// The bindings of one action map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindMap {
    /// The action map (`moveMap`).
    pub name: String,
    /// The map is recreated before the bindings (`if (isObject(m)) m.delete(); new ActionMap(m);`).
    #[serde(default)]
    pub replace: bool,
    pub bindings: Vec<Binding>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UserBinds {
    pub maps: Vec<BindMap>,
}

/// The arguments of `name(...)` in `line` (after `prefix`), split on commas
/// outside quotes; quoted ones unescaped.
fn call_args(rest: &str) -> Option<Vec<String>> {
    let inner = rest.trim().strip_prefix('(')?;
    let close = {
        let (mut q, mut esc) = (false, false);
        let mut end = None;
        for (i, c) in inner.char_indices() {
            if esc {
                esc = false;
            } else if q && c == '\\' {
                esc = true;
            } else if c == '"' {
                q = !q;
            } else if c == ')' && !q {
                end = Some(i);
            }
        }
        end?
    };
    let mut args = Vec::new();
    let (mut cur, mut q, mut esc, mut quoted) = (String::new(), false, false, false);
    for c in inner[..close].chars() {
        if esc {
            cur.push('\\');
            cur.push(c);
            esc = false;
        } else if q && c == '\\' {
            esc = true;
        } else if c == '"' {
            q = !q;
            quoted = true;
        } else if c == ',' && !q {
            args.push(if quoted { crate::vars::collapse_escape(&cur) } else { cur.trim().to_string() });
            (cur, quoted) = (String::new(), false);
        } else if q || !c.is_whitespace() {
            cur.push(c);
        }
    }
    if quoted || !cur.trim().is_empty() || !args.is_empty() {
        args.push(if quoted { crate::vars::collapse_escape(&cur) } else { cur.trim().to_string() });
    }
    Some(args)
}

impl UserBinds {
    fn map(&mut self, name: &str) -> &mut BindMap {
        let i = match self.maps.iter().position(|m| m.name.eq_ignore_ascii_case(name)) {
            Some(i) => i,
            None => {
                self.maps.push(BindMap { name: name.to_string(), replace: false, bindings: Vec::new() });
                self.maps.len() - 1
            }
        };
        &mut self.maps[i]
    }

    pub fn parse_config_cs(src: &str) -> UserBinds {
        let mut b = UserBinds::default();
        for line in src.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("new ActionMap(") {
                let name = rest.split(')').next().unwrap_or("").trim();
                if !name.is_empty() {
                    b.map(name).replace = true;
                }
                continue;
            }
            let Some((obj, rest)) = line.split_once('.') else { continue };
            if obj.contains(char::is_whitespace) || obj.contains('(') {
                continue;
            }
            let (method, rest) = match rest.find('(') {
                Some(i) => (&rest[..i], &rest[i..]),
                None => continue,
            };
            let Some(args) = call_args(rest) else { continue };
            let binding = match method.to_ascii_lowercase().as_str() {
                "bind" if args.len() >= 3 => Binding::Bind { device: args[0].clone(), key: args[1].clone(), args: args[2..].to_vec() },
                "bindcmd" if args.len() == 4 => Binding::BindCmd { device: args[0].clone(), key: args[1].clone(), make: args[2].clone(), brk: args[3].clone() },
                _ => continue,
            };
            b.map(obj).bindings.push(binding);
        }
        b
    }

    pub fn to_ron(&self) -> String {
        crate::save::pretty(self, &format!("{} key bindings.", tt_assets::product::NAME))
    }

    pub fn to_config_cs(&self) -> String {
        let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        let mut out = String::new();
        for m in &self.maps {
            for b in &m.bindings {
                let (method, args): (&str, Vec<&str>) = match b {
                    Binding::Bind { device, key, args } => ("bind", [device.as_str(), key.as_str()].into_iter().chain(args.iter().map(String::as_str)).collect()),
                    Binding::BindCmd { device, key, make, brk } => ("bindCmd", vec![device, key, make, brk]),
                };
                let args: Vec<String> = args.iter().map(|a| q(a)).collect();
                out += &format!("{}.{method}({});\n", m.name, args.join(", "));
            }
        }
        out
    }

    pub fn from_ron(src: &str) -> Result<UserBinds, String> {
        ron::from_str(src).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_cs() {
        let src = "if (isObject(moveMap)) moveMap.delete();\r\nnew ActionMap(moveMap);\r\nmoveMap.bind(keyboard, \"w\", moveforward);\r\nmoveMap.bind(mouse0, \"xaxis\", S, 0.5, yaw);\r\nmoveMap.bindCmd(keyboard, \"escape\", \"\", \"escapeFromGame(\\\"x, y\\\");\");\r\nGlobalActionMap.bind(keyboard, \"tilde\", toggleConsole);\r\n";
        let b = UserBinds::parse_config_cs(src);
        assert_eq!(b.maps.len(), 2);
        assert!(b.maps[0].replace && !b.maps[1].replace);
        assert_eq!(b.maps[0].bindings[0], Binding::Bind { device: "keyboard".into(), key: "w".into(), args: vec!["moveforward".into()] });
        assert_eq!(b.maps[0].bindings[1], Binding::Bind { device: "mouse0".into(), key: "xaxis".into(), args: vec!["S".into(), "0.5".into(), "yaw".into()] });
        assert_eq!(b.maps[0].bindings[2], Binding::BindCmd { device: "keyboard".into(), key: "escape".into(), make: String::new(), brk: "escapeFromGame(\"x, y\");".into() });
        assert_eq!(UserBinds::from_ron(&b.to_ron()).unwrap(), b);
        let again = UserBinds::parse_config_cs(&b.to_config_cs());
        assert_eq!(again.maps.iter().map(|m| &m.bindings).collect::<Vec<_>>(), b.maps.iter().map(|m| &m.bindings).collect::<Vec<_>>());
    }
}
