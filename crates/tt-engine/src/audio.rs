//! Audio plug-in point. The game's sounds (the game's `alx*`, `OpenAL*` and
//! `MusicPlayer` calls) go to an [`Audio`] implementation; [`MixerAudio`]
//! drives the tt-audio software mixer (with a cpal output when a device is
//! available), [`NullAudio`] only counts.

use std::path::Path;
use tt_audio::{AlParam, AudioDb, Handle, Mixer, SharedMixer, NULL_HANDLE};

pub trait Audio {
    /// `OpenALInitDriver()`.
    fn init_driver(&mut self) -> bool;
    /// `OpenALShutdownDriver()`.
    fn shutdown_driver(&mut self);
    /// `alGetString(AL_VENDOR | AL_VERSION | AL_RENDERER | AL_EXTENSIONS)`.
    fn al_string(&self, name: &str) -> String;
    /// `alxPlay(profile[, x, y, z])`.
    fn play_profile(&mut self, profile: &str, pos: Option<[f32; 3]>) -> Handle;
    /// `alxPlay(handle)`.
    fn play(&mut self, h: Handle) -> Handle;
    /// `alxCreateSource(profile[, pos])` or `alxCreateSource(description, file[, pos])`.
    fn create_source(&mut self, profile_or_desc: &str, file: Option<&str>, pos: Option<[f32; 3]>) -> Handle;
    fn stop(&mut self, h: Handle);
    fn stop_all(&mut self);
    fn source_f(&mut self, h: Handle, param: AlParam, v: f32);
    fn source_3f(&mut self, h: Handle, param: AlParam, v: [f32; 3]);
    fn listener_f(&mut self, param: AlParam, v: f32);
    fn set_channel_volume(&mut self, channel: u32, v: f32) -> bool;
    fn channel_volume(&self, channel: u32) -> f32;
    /// `alxGetWaveLen(file)` in ms (0 if unknown).
    fn wave_len_ms(&mut self, file: &str) -> u32;
    /// Camera transform for 3D sounds.
    fn set_listener(&mut self, pos: [f32; 3], forward: [f32; 3], up: [f32; 3]);
    fn music_set_track(&mut self, name: &str) -> bool;
    fn music_stop(&mut self);
    /// Per-tick update; returns errors to log.
    fn update(&mut self) -> Vec<String>;
    /// Short status line for reports.
    fn status(&self) -> String;
    /// Number of live sources (leak checks).
    fn source_count(&self) -> usize {
        0
    }
    /// An `SoundDescDef` / `SoundDef` / `MusicDef` was declared
    /// or changed: its class, name and field values (array fields flat,
    /// `mood10`).
    fn set_object(&mut self, _class: &str, _name: &str, _fields: &[(String, String)]) {}
    /// The declared name of the sound profile or description `name` names
    /// (case-insensitive), if there is one.
    fn profile_name(&self, _name: &str) -> Option<String> {
        None
    }
    /// The declared name of the music track `name` names, if there is one.
    fn music_name(&self, _name: &str) -> Option<String> {
        None
    }
}

/// The declared names of the sound objects (for [`Audio::profile_name`]
/// where there is no mixer database).
#[derive(Debug, Default, Clone)]
pub struct SoundNames {
    profiles: Vec<String>,
    music: Vec<String>,
}

impl SoundNames {
    pub fn set(&mut self, class: &str, name: &str) {
        let list = if class.eq_ignore_ascii_case("MusicDef") { &mut self.music } else { &mut self.profiles };
        if !list.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            list.push(name.to_string());
        }
    }
    pub fn profile(&self, name: &str) -> Option<String> {
        self.profiles.iter().find(|n| n.eq_ignore_ascii_case(name)).cloned()
    }
    pub fn music(&self, name: &str) -> Option<String> {
        self.music.iter().find(|n| n.eq_ignore_ascii_case(name)).cloned()
    }
}

/// Classes whose objects the audio database mirrors.
pub const AUDIO_CLASSES: [&str; 3] = ["SoundDescDef", "SoundDef", "MusicDef"];

pub struct MixerAudio {
    mixer: SharedMixer,
    want_output: bool,
    output: Option<tt_audio::output::Output>,
    device: String,
    errors: Vec<String>,
    /// `OpenALInitDriver` ran: profiles marked `preload` are decoded as
    /// they are declared.
    driver_init: bool,
}

impl MixerAudio {
    /// A mixer with an empty audio database: the engine fills it from the
    /// script objects as they are registered ([`Audio::set_object`]); sound
    /// files resolve under `game_root`. With `output` false the mixer runs
    /// silently (headless runs, screenshots).
    pub fn new(game_root: &Path, output: bool) -> MixerAudio {
        let mut db = AudioDb::new();
        db.game_root = Some(game_root.to_path_buf());
        MixerAudio {
            mixer: Mixer::new(44100, db).shared(),
            want_output: output,
            output: None,
            device: "none".into(),
            errors: Vec::new(),
            driver_init: false,
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Mixer) -> R) -> Option<R> {
        self.mixer.lock().ok().map(|mut m| f(&mut m))
    }

    fn ok(&mut self, r: Option<tt_audio::Result<Handle>>) -> Handle {
        match r {
            Some(Ok(h)) => h,
            Some(Err(e)) => {
                self.errors.push(e.to_string());
                NULL_HANDLE
            }
            None => NULL_HANDLE,
        }
    }
}

impl Audio for MixerAudio {
    fn init_driver(&mut self) -> bool {
        if self.want_output && self.output.is_none() {
            match tt_audio::output::Output::start(self.mixer.clone()) {
                Ok(o) => {
                    self.device = format!("{} ({} Hz, {} ch)", o.device_name, o.sample_rate, o.channels);
                    self.output = Some(o);
                }
                Err(e) => self.errors.push(format!("audio output unavailable, running silent: {e}")),
            }
        }
        let failed = self.with(|m| m.preload_profiles()).unwrap_or_default();
        for (name, e) in failed {
            self.errors.push(format!("preload {name}: {e}"));
        }
        self.driver_init = true;
        // Without a device the mixer still runs, so scripts see a working driver.
        true
    }

    fn shutdown_driver(&mut self) {
        self.with(|m| m.stop_all());
        self.output = None;
    }

    fn al_string(&self, name: &str) -> String {
        match name.trim().to_ascii_uppercase().as_str() {
            "AL_VENDOR" => "tt-audio".into(),
            "AL_VERSION" => "1.0 (software mixer)".into(),
            "AL_RENDERER" => format!("tt-audio mixer, output: {}", self.device),
            "AL_EXTENSIONS" => String::new(),
            _ => String::new(),
        }
    }

    fn play_profile(&mut self, profile: &str, pos: Option<[f32; 3]>) -> Handle {
        let r = self.with(|m| m.play_profile(profile, pos));
        self.ok(r)
    }

    fn play(&mut self, h: Handle) -> Handle {
        self.with(|m| m.play(h)).unwrap_or(NULL_HANDLE)
    }

    fn create_source(&mut self, name: &str, file: Option<&str>, pos: Option<[f32; 3]>) -> Handle {
        let r = self.with(|m| match file {
            Some(f) => m.create_source_desc(name, f, pos),
            None => m.create_source(name, pos),
        });
        self.ok(r)
    }

    fn stop(&mut self, h: Handle) {
        self.with(|m| m.stop(h));
    }

    fn stop_all(&mut self) {
        self.with(|m| m.stop_all());
    }

    fn source_f(&mut self, h: Handle, p: AlParam, v: f32) {
        self.with(|m| m.source_f(h, p, v));
    }

    fn source_3f(&mut self, h: Handle, p: AlParam, v: [f32; 3]) {
        self.with(|m| m.source_3f(h, p, v));
    }

    fn listener_f(&mut self, p: AlParam, v: f32) {
        self.with(|m| m.listener_f(p, v));
    }

    fn set_channel_volume(&mut self, channel: u32, v: f32) -> bool {
        self.with(|m| m.set_channel_volume(channel, v)).unwrap_or(false)
    }

    fn channel_volume(&self, channel: u32) -> f32 {
        self.with(|m| m.channel_volume(channel)).unwrap_or(0.0)
    }

    fn wave_len_ms(&mut self, file: &str) -> u32 {
        match self.with(|m| m.wave_len_ms(file)) {
            Some(Ok(ms)) => ms,
            Some(Err(e)) => {
                self.errors.push(e.to_string());
                0
            }
            None => 0,
        }
    }

    fn set_listener(&mut self, pos: [f32; 3], forward: [f32; 3], up: [f32; 3]) {
        self.with(|m| m.set_listener_transform(pos, forward, up));
    }

    fn music_set_track(&mut self, name: &str) -> bool {
        match self.with(|m| m.music_set_track(name)) {
            Some(Ok(())) => true,
            Some(Err(e)) => {
                self.errors.push(e.to_string());
                false
            }
            None => false,
        }
    }

    fn music_stop(&mut self) {
        self.with(|m| m.music_stop());
    }

    fn update(&mut self) -> Vec<String> {
        let mut errs = std::mem::take(&mut self.errors);
        if let Some(e) = self.with(|m| {
            m.update();
            m.take_errors()
        }) {
            errs.extend(e);
        }
        errs
    }

    fn source_count(&self) -> usize {
        self.with(|m| m.handles().len()).unwrap_or(0)
    }

    fn set_object(&mut self, class: &str, name: &str, fields: &[(String, String)]) {
        let preload = self.driver_init && class.eq_ignore_ascii_case("SoundDef");
        let r = self.with(|m| {
            m.db_mut().set_object(class, name, fields);
            if preload {
                m.preload_profile(name)
            } else {
                Ok(())
            }
        });
        if let Some(Err(e)) = r {
            self.errors.push(format!("preload {name}: {e}"));
        }
    }

    fn status(&self) -> String {
        let n = self.with(|m| m.handles().len()).unwrap_or(0);
        format!("tt-audio mixer, output {}, {n} sources", self.device)
    }

    fn profile_name(&self, name: &str) -> Option<String> {
        self.with(|m| {
            let db = m.db();
            db.profile(name).map(|p| p.name.clone()).or_else(|| db.description(name).map(|d| d.name.clone()))
        })
        .flatten()
    }

    fn music_name(&self, name: &str) -> Option<String> {
        self.with(|m| m.db().music(name).map(|t| t.name.clone())).flatten()
    }
}

/// Audio that does nothing (used when the game's audio data cannot be loaded).
#[derive(Default)]
pub struct NullAudio {
    next: Handle,
    names: SoundNames,
}

impl Audio for NullAudio {
    fn init_driver(&mut self) -> bool {
        true
    }
    fn shutdown_driver(&mut self) {}
    fn al_string(&self, _: &str) -> String {
        "none".into()
    }
    fn play_profile(&mut self, _: &str, _: Option<[f32; 3]>) -> Handle {
        self.next += 1;
        self.next
    }
    fn play(&mut self, h: Handle) -> Handle {
        h
    }
    fn create_source(&mut self, _: &str, _: Option<&str>, _: Option<[f32; 3]>) -> Handle {
        self.next += 1;
        self.next
    }
    fn stop(&mut self, _: Handle) {}
    fn stop_all(&mut self) {}
    fn source_f(&mut self, _: Handle, _: AlParam, _: f32) {}
    fn source_3f(&mut self, _: Handle, _: AlParam, _: [f32; 3]) {}
    fn listener_f(&mut self, _: AlParam, _: f32) {}
    fn set_channel_volume(&mut self, _: u32, _: f32) -> bool {
        true
    }
    fn channel_volume(&self, _: u32) -> f32 {
        1.0
    }
    fn wave_len_ms(&mut self, _: &str) -> u32 {
        0
    }
    fn set_listener(&mut self, _: [f32; 3], _: [f32; 3], _: [f32; 3]) {}
    fn music_set_track(&mut self, _: &str) -> bool {
        true
    }
    fn music_stop(&mut self) {}
    fn update(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn status(&self) -> String {
        "null audio".into()
    }
    fn set_object(&mut self, class: &str, name: &str, _: &[(String, String)]) {
        self.names.set(class, name);
    }
    fn profile_name(&self, name: &str) -> Option<String> {
        self.names.profile(name)
    }
    fn music_name(&self, name: &str) -> Option<String> {
        self.names.music(name)
    }
}
