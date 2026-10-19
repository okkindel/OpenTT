//! Data files written with the names of earlier versions (a mod made
//! before the rename of the data vocabulary): [`migrate`] rewrites the old
//! class and field names to the current ones before the file is read, and
//! notes each one (the game logs them as deprecated).

/// Renamed classes (RON variant names): (old, new).
pub const CLASSES: &[(&str, &str)] = &[
    ("AudioDescription", "SoundDescDef"),
    ("ClientAudioDescription", "ClientSoundDesc"),
    ("ClientAudioProfile", "ClientSound"),
    ("MusicData", "MusicDef"),
    ("CameraData", "CameraDef"),
    ("MissionMarkerData", "MarkerDef"),
    ("AudioProfile", "SoundDef"),
    ("ExplosionData", "ExplosionDef"),
    ("ParticleData", "ParticleDef"),
    ("ParticleEmitterData", "EmitterDef"),
    ("ReticleData", "ReticleDef"),
    ("ProjectileData", "ProjectileDef"),
    ("PowerUpData", "PowerUpDef"),
    ("BoostData", "BoostDef"),
    ("VolcanoData", "VolcanoDef"),
    ("FlagData", "FlagDef"),
    ("TankAIData", "TankAiDef"),
    ("TankData", "TankDef"),
    ("TankCameraData", "TankCameraDef"),
    ("TankFxData", "TankFxDef"),
    ("TankMiscData", "TankMiscDef"),
    ("SimGroup", "Group"),
    ("ScriptObject", "Info"),
    ("TerrainBlock", "Terrain"),
    ("SpawnSphere", "SpawnPoint"),
    ("TSStatic", "Prop"),
];

/// Renamed fields: (class, old, new); class "" = a struct without a
/// name (a mission file, a solo level).
pub const FIELDS: &[(&str, &str, &str)] = &[
    ("SoundDescDef", "kind", "channel"),
    ("ClientSoundDesc", "kind", "channel"),
    ("SoundDef", "file_name", "file"),
    ("ClientSound", "file_name", "file"),
    ("ProjectileDef", "num_bounce", "bounces"),
    ("ProjectileDef", "splash_area", "blast_radius"),
    ("ProjectileDef", "count", "rounds"),
    ("ProjectileDef", "save_old_cartridge", "keep_previous_weapon"),
    ("ProjectileDef", "can_save", "can_be_kept"),
    ("ProjectileDef", "reload_time", "reload_ms"),
    ("ProjectileDef", "burst_delay", "volley_delay_ms"),
    ("ProjectileDef", "burst_count", "volley_size"),
    ("ProjectileDef", "synch_time", "sync_ms"),
    ("ProjectileDef", "synch_before", "sync_lead_ms"),
    ("ProjectileDef", "size_scale", "sprite_scale"),
    ("ProjectileDef", "max_life", "max_life_ms"),
    ("ExplosionDef", "start_scale", "start_size"),
    ("ExplosionDef", "end_scale", "end_size"),
    ("ExplosionDef", "duration_scale", "duration_ms"),
    ("ParticleDef", "texture_name", "texture"),
    ("ParticleDef", "inherited_vel_factor", "inherit_velocity"),
    ("ParticleDef", "use_inv_alpha", "inverse_alpha"),
    ("ReticleDef", "bmp", "image"),
    ("ReticleDef", "anim_bmp", "anim_image"),
    ("ReticleDef", "spot_bmp", "spot_image"),
    ("ReticleDef", "ring_bmp", "ring_image"),
    ("ReticleDef", "do_anim", "animate"),
    ("ReticleDef", "do_spot", "show_spot"),
    ("PowerUpDef", "start_on", "starts_on"),
    ("PowerUpDef", "min_off", "min_off_ms"),
    ("PowerUpDef", "max_off", "max_off_ms"),
    ("PowerUpDef", "userseq1", "activate_sequence"),
    ("PowerUpDef", "class_name", "behavior"),
    ("PowerUpDef", "pup_projectile", "test_projectile"),
    ("PowerUpDef", "pup_sound", "test_sound"),
    ("FlagDef", "shape_file", "shape"),
    ("FlagDef", "hard_coll_sound", "hard_collision_sound"),
    ("FlagDef", "hard_coll_vel", "hard_collision_speed"),
    ("FlagDef", "soft_coll_sound", "soft_collision_sound"),
    ("FlagDef", "soft_coll_vel", "soft_collision_speed"),
    ("FlagDef", "coll_isi", "collision_interval_ms"),
    ("FlagDef", "snag_flag_sound", "snag_sound"),
    ("MarkerDef", "shape_file", "shape"),
    ("TankDef", "shape_file", "shape"),
    ("TankDef", "brain_shape_file", "brain_shape"),
    ("TankDef", "deccel_rate", "decel_rate"),
    ("TankDef", "coast_deccel_rate", "coast_decel_rate"),
    ("TankDef", "turret_ver_center", "turret_pitch_center"),
    ("TankDef", "turret_hor_range", "turret_yaw_range"),
    ("TankDef", "turret_ver_range", "turret_pitch_range"),
    ("TankFxDef", "hard_tank_coll_sound", "hard_tank_collision_sound"),
    ("TankFxDef", "hard_tank_coll_vel", "hard_tank_collision_speed"),
    ("TankFxDef", "hard_static_coll_sound", "hard_static_collision_sound"),
    ("TankFxDef", "hard_static_coll_vel", "hard_static_collision_speed"),
    ("TankFxDef", "hard_ground_coll_sound", "hard_ground_collision_sound"),
    ("TankFxDef", "hard_ground_coll_vel", "hard_ground_collision_speed"),
    ("TankFxDef", "soft_tank_coll_sound", "soft_tank_collision_sound"),
    ("TankFxDef", "soft_tank_coll_vel", "soft_tank_collision_speed"),
    ("TankFxDef", "soft_static_coll_sound", "soft_static_collision_sound"),
    ("TankFxDef", "soft_static_coll_vel", "soft_static_collision_speed"),
    ("TankFxDef", "soft_ground_coll_sound", "soft_ground_collision_sound"),
    ("TankFxDef", "soft_ground_coll_vel", "soft_ground_collision_speed"),
    ("TankFxDef", "tank_coll_isi", "tank_collision_interval_ms"),
    ("TankFxDef", "static_coll_isi", "static_collision_interval_ms"),
    ("TankFxDef", "ground_coll_isi", "ground_collision_interval_ms"),
    ("", "gotolevel", "goto_level"),
    ("SpawnPoint", "data_block", "def"),
    ("PowerUp", "data_block", "def"),
    ("Boost", "data_block", "def"),
    ("Volcano", "data_block", "def"),
    ("Flag", "data_block", "def"),
    ("Prop", "shape_name", "shape"),
    ("", "datablocks", "defs"),
    ("Other", "datablock", "def"),
];

fn channel_of_variable(v: &str) -> Option<&'static str> {
    Some(match v.trim_start_matches('$').to_ascii_lowercase().as_str() {
        "defaultaudiotype" => "default",
        "guiaudiotype" => "gui",
        "simaudiotype" => "sim",
        "messageaudiotype" => "message",
        "musicaudiotype" => "music",
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Ident,
    Str,
    Open(u8),
    Close,
    Colon,
    Other,
}

/// The tokens of a RON text: (kind, byte range). Comments, the
/// `#![enable(..)]` lines and white space are left out.
fn tokens(text: &str) -> Vec<(Tok, usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c.is_ascii_whitespace() {
            i += 1;
        } else if text[i..].starts_with("//") || c == b'#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if text[i..].starts_with("/*") {
            i = text[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
        } else if c == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(b.len());
            out.push((Tok::Str, start, i));
        } else if c == b'r' && matches!(b.get(i + 1), Some(b'"') | Some(b'#')) {
            let hashes = text[i + 1..].bytes().take_while(|&h| h == b'#').count();
            let close = format!("\"{}", "#".repeat(hashes));
            let body = i + 1 + hashes + 1;
            i = text.get(body..).and_then(|t| t.find(&close)).map_or(b.len(), |e| body + e + close.len());
            out.push((Tok::Str, start, i));
        } else if c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != b'\'' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(b.len());
            out.push((Tok::Other, start, i));
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push((Tok::Ident, start, i));
        } else if matches!(c, b'(' | b'[' | b'{') {
            i += 1;
            out.push((Tok::Open(c), start, i));
        } else if matches!(c, b')' | b']' | b'}') {
            i += 1;
            out.push((Tok::Close, start, i));
        } else if c == b':' {
            i += 1;
            out.push((Tok::Colon, start, i));
        } else {
            // Numbers, commas, signs: one token up to the next delimiter.
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'.' | b'_' | b'+' | b'-')) {
                i += 1;
            }
            out.push((Tok::Other, start, i));
        }
    }
    out
}

/// Rewrites the names of earlier versions in a data file's text to the
/// current ones ([`CLASSES`], [`FIELDS`]; a sound description's old
/// `kind: (global: "$...AudioType")` becomes its `channel` name), with a
/// note for each name rewritten.
pub fn migrate(text: &str) -> (std::borrow::Cow<'_, str>, Vec<String>) {
    let toks = tokens(text);
    // (byte range, replacement)
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut note = |n: String| {
        if !notes.contains(&n) {
            notes.push(n);
        }
    };
    // The open brackets: the class of a `Class(` (resolved to its new
    // name), "" for an unnamed `(`, None for `[` / `{`.
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let (t, s, e) = toks[i];
        let next = toks.get(i + 1).map(|x| x.0);
        match t {
            Tok::Ident if next == Some(Tok::Open(b'(')) => {
                let name = &text[s..e];
                let class = match CLASSES.iter().find(|(old, _)| *old == name) {
                    Some((old, new)) => {
                        edits.push((s, e, new.to_string()));
                        note(format!("class `{old}` is now `{new}`"));
                        new.to_string()
                    }
                    None => name.to_string(),
                };
                stack.push(Some(class));
                i += 2;
                continue;
            }
            Tok::Ident if next == Some(Tok::Colon) => {
                let ctx = match stack.last() {
                    Some(Some(c)) => c.as_str(),
                    _ => "",
                };
                let name = &text[s..e];
                if let Some((_, old, new)) = FIELDS.iter().find(|(c, old, _)| *c == ctx && *old == name) {
                    edits.push((s, e, new.to_string()));
                    let place = if ctx.is_empty() { String::new() } else { format!(" (in {ctx})") };
                    note(format!("field `{old}`{place} is now `{new}`"));
                    // > `channel: "sim"`.
                    if *new == "channel" && toks.get(i + 2).map(|x| x.0) == Some(Tok::Open(b'(')) {
                        let open = i + 2;
                        let mut depth = 0;
                        let mut end = open;
                        let mut var = None;
                        for (j, (tk, ts, te)) in toks.iter().enumerate().skip(open) {
                            match tk {
                                Tok::Open(_) => depth += 1,
                                Tok::Close => depth -= 1,
                                Tok::Str => var = Some(text[ts + 1..te - 1].to_string()),
                                _ => {}
                            }
                            if depth == 0 {
                                end = j;
                                break;
                            }
                        }
                        if let Some(ch) = var.as_deref().and_then(channel_of_variable) {
                            edits.push((toks[open].1, toks[end].2, format!("{ch:?}")));
                            i = end + 1;
                            continue;
                        }
                    }
                }
            }
            Tok::Open(b'(') => stack.push(Some(String::new())),
            Tok::Open(_) => stack.push(None),
            Tok::Close => {
                stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
    if edits.is_empty() {
        return (std::borrow::Cow::Borrowed(text), notes);
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (s, e, r) in edits {
        out.push_str(&text[at..s]);
        out.push_str(&r);
        at = e;
    }
    out.push_str(&text[at..]);
    (std::borrow::Cow::Owned(out), notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::defs::{Def, SoundsFile};
    use crate::data::mission::{Mission, MissionObject};

    #[test]
    fn old_names_load_as_the_new_ones() {
        let old = r#"// a mod's weapons, written before the rename
#![enable(implicit_some)]
[
  ProjectileData(
    name: "Mortar", // "splash_area: 1" in a comment stays
    num_bounce: 2,
    splash_area: 11,
    count: 6,
    reload_time: 1800,
    burst_delay: 1500,
    burst_count: 3,
    size_scale: 0.7,
    resource: "count: 3",
  ),
  PowerUpData(name: "Heal", userseq1: "activate", kind: "health", min_off: 9),
]
"#;
        let new = r#"[
  ProjectileDef(name: "Mortar", bounces: 2, blast_radius: 11, rounds: 6, reload_ms: 1800, volley_delay_ms: 1500, volley_size: 3, sprite_scale: 0.7, resource: "count: 3"),
  PowerUpDef(name: "Heal", activate_sequence: "activate", kind: "health", min_off_ms: 9),
]"#;
        let (a, notes): (Vec<Def>, _) = crate::data::from_ron_noted(old).unwrap();
        let (b, none): (Vec<Def>, _) = crate::data::from_ron_noted(new).unwrap();
        assert_eq!(a, b);
        assert!(none.is_empty(), "{none:?}");
        assert!(notes.contains(&"class `ProjectileData` is now `ProjectileDef`".to_string()), "{notes:?}");
        assert!(notes.contains(&"field `splash_area` (in ProjectileDef) is now `blast_radius`".to_string()), "{notes:?}");
        // `kind` is renamed in a sound description only.
        assert!(!notes.iter().any(|n| n.contains("`kind`")), "{notes:?}");
        assert_eq!(notes.len(), 11, "{notes:?}");
    }

    #[test]
    fn old_sound_channels_and_missions() {
        let sounds = r#"(game: [AudioDescription(name: "A", kind: (global: "$SimAudioType"), volume: 1)],
  interface: [ClientAudioDescription(name: "G", kind: (
        global: "$GuiAudioType",
      )), ClientAudioProfile(name: "S", file_name: "a.ogg", description: "G")],
  music: [])"#;
        let (f, notes): (SoundsFile, _) = crate::data::from_ron_noted(sounds).unwrap();
        let text = crate::data::to_ron("", &f).unwrap();
        assert!(text.contains("channel: \"sim\"") && text.contains("channel: \"gui\"") && text.contains("file: \"a.ogg\""), "{text}");
        assert_eq!(notes.len(), 6, "{notes:?}");
        let mission = r#"(datablocks: [PowerUpData(name: "P", pup_sound: "x")],
  objects: [SimGroup(name: "MissionGroup", children: [
    ScriptObject(name: "MissionInfo", display_name: "Map"),
    TerrainBlock(terrain_file: "t.ter"),
    SpawnSphere(data_block: "SpawnSphereMarker"),
    TSStatic(shape_name: "models/props/green/treegreen01.dts", light_boost: 0),
  ])])"#;
        let (m, notes): (Mission, _) = crate::data::from_ron_noted(mission).unwrap();
        assert_eq!(m.defs.len(), 1);
        let MissionObject::Group(g) = &m.objects[0] else { panic!() };
        assert!(matches!(&g.children[3], MissionObject::Prop(p) if p.shape.as_deref() == Some("models/props/green/treegreen01.dts")));
        assert!(matches!(&g.children[2], MissionObject::SpawnPoint(p) if p.def.as_deref() == Some("SpawnSphereMarker")));
        assert!(notes.iter().any(|n| n == "field `datablocks` is now `defs`"), "{notes:?}");
    }

    /// No old name is a current name in the same place (a current file
    /// never changes, gets no note).
    #[test]
    fn old_names_are_not_current_names() {
        for (old, new) in CLASSES {
            assert!(!CLASSES.iter().any(|(o, _)| o == new), "{old} -> {new}");
        }
        for (c, old, _) in FIELDS {
            assert!(!FIELDS.iter().any(|(c2, _, n)| c2 == c && n == old), "{c}.{old}");
        }
    }
}
