//! The sound defs of the data files: `SoundDescDef` (how a sound plays),
//! `SoundDef` (a sound file) and the `MusicDef` tracks.
//!
//! The engine fills the database from the declarations of the data files
//! as they are made ([`AudioDb::set_object`]: class, name and field
//! values).

use std::path::PathBuf;

/// Number of audio channels ("types") the mixer keeps volumes for.
pub const NUM_AUDIO_TYPES: usize = 32;

/// The mixer channels the sound descriptions name (`channel`).
pub const DEFAULT_AUDIO_TYPE: u32 = 0;
pub const GUI_AUDIO_TYPE: u32 = 1;
pub const SIM_AUDIO_TYPE: u32 = 2;
pub const MESSAGE_AUDIO_TYPE: u32 = 3;
pub const MUSIC_AUDIO_TYPE: u32 = 4;

/// The mixer channel a sound description names (`channel: "sim"`; or a
/// channel number), case-insensitive.
pub fn channel(name: &str) -> Option<u32> {
    let n = name.trim().to_ascii_lowercase();
    Some(match n.as_str() {
        "default" => DEFAULT_AUDIO_TYPE,
        "gui" => GUI_AUDIO_TYPE,
        "sim" => SIM_AUDIO_TYPE,
        "message" => MESSAGE_AUDIO_TYPE,
        "music" => MUSIC_AUDIO_TYPE,
        _ => return n.parse::<u32>().ok().map(|c| c.min(NUM_AUDIO_TYPES as u32 - 1)),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundDesc {
    pub name: String,
    pub volume: f32,
    pub is_looping: bool,
    pub is_streaming: bool,
    pub is_3d: bool,
    pub reference_distance: f32,
    pub max_distance: f32,
    pub cone_inside_angle: f32,
    pub cone_outside_angle: f32,
    pub cone_outside_volume: f32,
    pub cone_vector: [f32; 3],
    /// Channel index for `alxSetChannelVolume` (`type` field).
    pub channel: u32,
}

impl Default for SoundDesc {
    fn default() -> Self {
        SoundDesc {
            name: String::new(),
            volume: 1.0,
            is_looping: false,
            is_streaming: false,
            is_3d: false,
            reference_distance: 1.0,
            max_distance: 100.0,
            cone_inside_angle: 360.0,
            cone_outside_angle: 360.0,
            cone_outside_volume: 1.0,
            cone_vector: [0.0, 0.0, 1.0],
            channel: DEFAULT_AUDIO_TYPE,
        }
    }
}

impl SoundDesc {
    /// What the engine falls back to for music when `AudioMusic` is missing.
    pub fn music() -> Self {
        SoundDesc {
            name: "AudioMusic".into(),
            is_looping: true,
            is_streaming: true,
            channel: MUSIC_AUDIO_TYPE,
            ..Default::default()
        }
    }

    fn from_fields(name: &str, f: &Fields) -> Self {
        let d = SoundDesc::default();
        SoundDesc {
            name: name.to_string(),
            volume: f.float("volume").unwrap_or(d.volume),
            is_looping: f.bool("is_looping").unwrap_or(d.is_looping),
            is_streaming: f.bool("is_streaming").unwrap_or(d.is_streaming),
            is_3d: f.bool("is_3d").unwrap_or(d.is_3d),
            reference_distance: f.float("reference_distance").unwrap_or(d.reference_distance),
            max_distance: f.float("max_distance").unwrap_or(d.max_distance),
            cone_inside_angle: f.float("cone_inside_angle").unwrap_or(d.cone_inside_angle),
            cone_outside_angle: f.float("cone_outside_angle").unwrap_or(d.cone_outside_angle),
            cone_outside_volume: f.float("cone_outside_volume").unwrap_or(d.cone_outside_volume),
            cone_vector: f.vec3("cone_vector").unwrap_or(d.cone_vector),
            channel: f.get("channel").and_then(channel).unwrap_or(d.channel),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundProfile {
    pub name: String,
    /// File name as written in the script (e.g. `sound/sfx/explode.ogg`).
    pub filename: String,
    /// Name of the `SoundDescDef` (looked up case-insensitively).
    pub description: String,
    pub preload: bool,
}

impl SoundProfile {
    fn from_fields(name: &str, f: &Fields) -> Self {
        SoundProfile {
            name: name.to_string(),
            filename: f.get("file").unwrap_or_default().to_string(),
            description: f.get("description").unwrap_or_default().to_string(),
            preload: f.bool("preload").unwrap_or(false),
        }
    }
}

/// ThinkTanks `MusicDef`: a track made of short Ogg segments chained by
/// sequences.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MusicTrack {
    pub name: String,
    pub num_moods: i32,
    /// `moods[m][i]` = field `mood<m+1>[i]`.
    pub moods: [[String; 3]; 3],
    pub urgent_mood: String,
    /// `sequences[m][i]` = field `sequences<m+1>[i]`.
    pub sequences: [[String; 3]; 3],
    pub k: [f32; 3],
    pub smooth: [f32; 3],
    pub mood_smooth: f32,
    pub mood_scale: f32,
}

impl MusicTrack {
    fn from_fields(name: &str, f: &Fields) -> Self {
        let mut m = MusicTrack { name: name.to_string(), num_moods: f.int("num_moods").unwrap_or(0), ..Default::default() };
        for mood in 0..3 {
            for i in 0..3 {
                m.moods[mood][i] = f.indexed(&format!("mood{}", mood + 1), i).unwrap_or_default().to_string();
                m.sequences[mood][i] = f.indexed(&format!("sequences{}", mood + 1), i).unwrap_or_default().to_string();
            }
            m.k[mood] = f.float(&format!("k{}", mood + 1)).unwrap_or(0.0);
            m.smooth[mood] = f.float(&format!("smooth{}", mood + 1)).unwrap_or(0.0);
        }
        m.urgent_mood = f.get("urgent_mood").unwrap_or_default().to_string();
        m.mood_smooth = f.float("mood_smooth").unwrap_or(0.0);
        m.mood_scale = f.float("mood_scale").unwrap_or(0.0);
        m
    }

    /// Number of non-empty sequences of mood `mood` (1-based), as the engine
    /// counts them when a track is selected.
    pub fn sequence_count(&self, mood: usize) -> usize {
        self.sequences.get(mood.wrapping_sub(1)).map_or(0, |s| s.iter().filter(|s| !s.is_empty()).count())
    }

    /// Every segment file this track can play.
    pub fn files(&self) -> Vec<&str> {
        self.moods.iter().flatten().chain(std::iter::once(&self.urgent_mood)).filter(|s| !s.is_empty()).map(|s| s.as_str()).collect()
    }
}

/// Field values of one object: (name, value); the last of a name counts.
struct Fields<'a>(&'a [(String, String)]);

impl Fields<'_> {
    fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    /// An array field's element, stored flat (`mood10` for `mood1[0]`).
    fn indexed(&self, name: &str, idx: usize) -> Option<&str> {
        self.get(&format!("{name}{idx}"))
    }
    fn float(&self, name: &str) -> Option<f32> {
        self.get(name).map(atof)
    }
    fn int(&self, name: &str) -> Option<i32> {
        self.get(name).map(|v| atof(v) as i32)
    }
    fn bool(&self, name: &str) -> Option<bool> {
        self.get(name).map(|v| v.eq_ignore_ascii_case("true") || atof(v) != 0.0)
    }
    fn vec3(&self, name: &str) -> Option<[f32; 3]> {
        let v = self.get(name)?;
        let mut it = v.split_whitespace().map(atof);
        Some([it.next().unwrap_or(0.0), it.next().unwrap_or(0.0), it.next().unwrap_or(0.0)])
    }
}

fn atof(s: &str) -> f32 {
    let s = s.trim_start();
    let end = s
        .char_indices()
        .take_while(|&(i, c)| c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && i == 0) || c == 'e' || c == 'E')
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    (0..=end).rev().find_map(|e| s[..e].parse::<f32>().ok()).unwrap_or(0.0)
}

/// All audio objects of the game.
#[derive(Debug, Clone, Default)]
pub struct AudioDb {
    /// The asset directory sound file names are relative to.
    pub game_root: Option<PathBuf>,
    descriptions: Vec<SoundDesc>,
    profiles: Vec<SoundProfile>,
    music: Vec<MusicTrack>,
}

impl AudioDb {
    pub fn new() -> Self {
        AudioDb::default()
    }

    /// Adds or replaces one audio object from its live field values (class
    /// `SoundDescDef`, `SoundDef` or `MusicDef`; anything else is
    /// ignored). Array fields are given flat (`mood10` for `mood1[0]`).
    /// Returns true if the object is an audio object.
    pub fn set_object(&mut self, class: &str, name: &str, fields: &[(String, String)]) -> bool {
        let f = Fields(fields);
        fn put<T>(v: &mut Vec<T>, item: T, name: impl Fn(&T) -> &str) {
            let n = name(&item).to_ascii_lowercase();
            match v.iter_mut().find(|x| name(x).eq_ignore_ascii_case(&n)) {
                Some(slot) => *slot = item,
                None => v.push(item),
            }
        }
        match class.to_ascii_lowercase().as_str() {
            "sounddescdef" => put(&mut self.descriptions, SoundDesc::from_fields(name, &f), |d| &d.name),
            "sounddef" => put(&mut self.profiles, SoundProfile::from_fields(name, &f), |p| &p.name),
            "musicdef" => put(&mut self.music, MusicTrack::from_fields(name, &f), |m| &m.name),
            _ => return false,
        }
        true
    }

    pub fn descriptions(&self) -> &[SoundDesc] {
        &self.descriptions
    }

    pub fn profiles(&self) -> &[SoundProfile] {
        &self.profiles
    }

    pub fn music_tracks(&self) -> &[MusicTrack] {
        &self.music
    }

    pub fn description(&self, name: &str) -> Option<&SoundDesc> {
        self.descriptions.iter().find(|d| d.name.eq_ignore_ascii_case(name))
    }

    pub fn profile(&self, name: &str) -> Option<&SoundProfile> {
        self.profiles.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn music(&self, name: &str) -> Option<&MusicTrack> {
        self.music.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }

    /// The description a profile plays with (engine defaults if it names a
    /// description that does not exist).
    pub fn profile_description(&self, profile: &SoundProfile) -> SoundDesc {
        self.description(&profile.description).cloned().unwrap_or_default()
    }

    pub fn resolve_file(&self, path: &str) -> Option<PathBuf> {
        let path = path.replace('\\', "/");
        let p = match &self.game_root {
            Some(root) => tt_assets::resolve(root, root, &path),
            None => PathBuf::from(&path),
        };
        if let Some(found) = tt_assets::find_ci(&p) {
            return Some(found);
        }
        let ext = p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
        let alt = match ext.as_deref() {
            Some("wav") => "ogg",
            Some("ogg") => "wav",
            _ => return None,
        };
        tt_assets::find_ci(&p.with_extension(alt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objects_from_live_fields() {
        let mut db = AudioDb::new();
        let f = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        assert!(db.set_object("SoundDescDef", "AudioGui", &f(&[("volume", "0.5"), ("channel", "gui")])));
        assert!(db.set_object("SoundDef", "Boom", &f(&[("file", "sound/sfx/explode.ogg"), ("description", "AudioGui"), ("preload", "1")])));
        assert!(db.set_object("MusicDef", "Lush", &f(&[("num_moods", "1"), ("mood10", "sound/music/a.ogg"), ("sequences10", "1 1")])));
        assert!(!db.set_object("Level", "X", &[]));
        assert_eq!(db.description("audiogui").unwrap().channel, GUI_AUDIO_TYPE);
        let p = db.profile("boom").unwrap();
        assert!(p.preload);
        assert_eq!(db.profile_description(p).volume, 0.5);
        let m = db.music("lush").unwrap();
        assert_eq!(m.moods[0][0], "sound/music/a.ogg");
        assert_eq!(m.sequence_count(1), 1);
        db.set_object("SoundDescDef", "audiogui", &f(&[("volume", "0.25")]));
        assert_eq!(db.descriptions().len(), 1);
        assert_eq!(db.description("AudioGui").unwrap().volume, 0.25);
    }

    #[test]
    fn atof_like_c() {
        assert_eq!(atof("0.6"), 0.6);
        assert_eq!(atof("  12abc"), 12.0);
        assert_eq!(atof("-3"), -3.0);
        assert_eq!(atof("x"), 0.0);
    }
}
