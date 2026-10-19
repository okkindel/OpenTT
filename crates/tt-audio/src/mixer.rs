//! The [`Mixer`] owns all sources and renders them to interleaved stereo
//! `f32` at its output rate. It does no I/O on its own: [`Mixer::render`] is
//! called either by the real-time output (feature `output`) or directly
//! (offline rendering, tests).
//!
//! Gain of a source = source gain (initially the description's `volume`,
//! changed by `alxSourcef(AL_GAIN)`) × channel volume of the description's
//! `type` (`alxSetChannelVolume`) × listener gain (`alxListenerf
//! (AL_GAIN_LINEAR)`) × distance attenuation × cone attenuation (3D only).
//!
//! Mono 3D sources are panned by the angle to the listener's right vector
//! (`at × up`); like OpenAL, multi-channel buffers are not spatialized.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::db::{AudioDb, SoundDesc, NUM_AUDIO_TYPES};
use crate::decode::{self, OggStream, Pcm};
use crate::music::MusicPlayer;
use crate::{Error, Result};

/// Source handle as returned to scripts (`0` = none, like `NULL_AUDIOHANDLE`).
pub type Handle = u32;
pub const NULL_HANDLE: Handle = 0;

/// A mixer shared between the game thread and the audio callback.
pub type SharedMixer = Arc<Mutex<Mixer>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlParam {
    /// `AL_GAIN`: treated as linear (see crate docs on uncertainty).
    Gain,
    GainLinear,
    Pitch,
    Looping,
    ReferenceDistance,
    MaxDistance,
    RolloffFactor,
    ConeInnerAngle,
    ConeOuterAngle,
    ConeOuterGain,
    Position,
    Direction,
    Velocity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DistanceModel {
    /// OpenAL `AL_INVERSE_DISTANCE_CLAMPED` (the OpenAL default).
    #[default]
    InverseClamped,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    pub position: [f32; 3],
    /// Forward vector (`AL_ORIENTATION` "at").
    pub at: [f32; 3],
    pub up: [f32; 3],
    /// Master volume (`alxListenerf(AL_GAIN_LINEAR, ...)`).
    pub gain: f32,
}

impl Default for Listener {
    fn default() -> Self {
        Listener { position: [0.0; 3], at: [0.0, 1.0, 0.0], up: [0.0, 0.0, 1.0], gain: 1.0 }
    }
}

struct StreamData {
    dec: OggStream,
    /// Interleaved samples, converted to the source's channel count.
    buf: VecDeque<f32>,
    /// The decoder hit the end and no continuation exists.
    finished: bool,
}

enum Data {
    Static(Arc<Pcm>),
    Stream(Box<StreamData>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Initial,
    Playing,
}

struct Source {
    desc: SoundDesc,
    gain: f32,
    pitch: f32,
    looping: bool,
    rolloff: f32,
    position: [f32; 3],
    direction: [f32; 3],
    data: Data,
    rate: u32,
    channels: u16,
    /// Frame position (static: absolute; stream: relative to buffer front).
    cursor: f64,
    state: State,
    is_music: bool,
}

/// The mixer: all `alx*` state plus the music player.
pub struct Mixer {
    db: AudioDb,
    sample_rate: u32,
    sources: BTreeMap<Handle, Source>,
    next_handle: Handle,
    buffers: HashMap<PathBuf, Arc<Pcm>>,
    channel_volume: [f32; NUM_AUDIO_TYPES],
    listener: Listener,
    distance_model: DistanceModel,
    music: MusicPlayer,
    music_handle: Handle,
    errors: Vec<String>,
    /// Wall-clock time up to which sources have been advanced by
    /// [`Mixer::update`] (headless clock).
    clock: Option<Instant>,
    /// When [`Mixer::render`] last ran (an output device is pulling audio).
    last_render: Option<Instant>,
}

/// If nothing has rendered for this long, [`Mixer::update`] assumes there is
/// no output device and advances sources by wall-clock time itself.
const RENDER_TIMEOUT: Duration = Duration::from_millis(500);

const MAX_ERRORS: usize = 64;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
fn normalize(a: [f32; 3]) -> Option<[f32; 3]> {
    let l = len(a);
    (l > 1e-6).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

/// Distance attenuation for a 3D source (0 when culled).
pub fn distance_gain(model: DistanceModel, dist: f32, reference: f32, max: f32, rolloff: f32) -> f32 {
    if dist >= max {
        return 0.0;
    }
    let reference = reference.max(0.0);
    match model {
        DistanceModel::InverseClamped => {
            let d = dist.clamp(reference, max.max(reference));
            let denom = reference + rolloff * (d - reference);
            if denom <= 0.0 {
                1.0
            } else {
                (reference / denom).clamp(0.0, 1.0)
            }
        }
        DistanceModel::Linear => {
            if dist < reference || max <= reference {
                1.0
            } else {
                1.0 - (dist - reference) / (max - reference)
            }
        }
    }
}

/// (left, right) multipliers for a pan value in `-1..=1` (right positive):
/// unity in the centre, the far channel fading out at the sides.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    ((1.0 - p).sqrt().min(1.0), (1.0 + p).sqrt().min(1.0))
}

impl Mixer {
    pub fn new(sample_rate: u32, db: AudioDb) -> Mixer {
        Mixer {
            db,
            sample_rate: sample_rate.max(1),
            sources: BTreeMap::new(),
            next_handle: 1,
            buffers: HashMap::new(),
            channel_volume: [1.0; NUM_AUDIO_TYPES],
            listener: Listener::default(),
            distance_model: DistanceModel::default(),
            music: MusicPlayer::default(),
            music_handle: NULL_HANDLE,
            errors: Vec::new(),
            clock: None,
            last_render: None,
        }
    }

    pub fn shared(self) -> SharedMixer {
        Arc::new(Mutex::new(self))
    }

    pub fn db(&self) -> &AudioDb {
        &self.db
    }

    /// The audio objects, for adding or replacing them while running
    /// ([`AudioDb::set_object`]).
    pub fn db_mut(&mut self) -> &mut AudioDb {
        &mut self.db
    }

    /// Changes the output rate (sources resample on the fly).
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate.max(1);
    }

    pub fn music(&self) -> &MusicPlayer {
        &self.music
    }

    /// Errors that happened while rendering (e.g. a music segment that
    /// failed to open), oldest first.
    pub fn take_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.errors)
    }

    fn log(&mut self, e: impl Into<String>) {
        if self.errors.len() < MAX_ERRORS {
            self.errors.push(e.into());
        }
    }

    // ---- buffers ---------------------------------------------------------

    fn resolve(&self, filename: &str) -> Result<PathBuf> {
        self.db.resolve_file(filename).ok_or_else(|| Error(format!("audio file not found: {filename}")))
    }

    /// Decodes (and caches) a whole sound file named as in the scripts.
    pub fn load_buffer(&mut self, filename: &str) -> Result<Arc<Pcm>> {
        let path = self.resolve(filename)?;
        self.load_path(&path)
    }

    fn load_path(&mut self, path: &Path) -> Result<Arc<Pcm>> {
        if let Some(b) = self.buffers.get(path) {
            return Ok(b.clone());
        }
        let pcm = Arc::new(decode::decode_file(path)?);
        self.buffers.insert(path.to_path_buf(), pcm.clone());
        Ok(pcm)
    }

    /// Decodes every profile marked `preload` (all of the game's are);
    /// returns the failures.
    pub fn preload_profiles(&mut self) -> Vec<(String, Error)> {
        let files: Vec<(String, String, bool)> =
            self.db.profiles().iter().filter(|p| p.preload).map(|p| (p.name.clone(), p.filename.clone(), self.db.profile_description(p).is_streaming)).collect();
        let mut failed = Vec::new();
        for (name, file, streaming) in files {
            if streaming {
                continue;
            }
            if let Err(e) = self.load_buffer(&file) {
                failed.push((name, e));
            }
        }
        failed
    }

    /// Decodes profile `name`'s file if it is marked `preload` (and not
    /// streaming), as [`Mixer::preload_profiles`] does for every profile.
    pub fn preload_profile(&mut self, name: &str) -> Result<()> {
        let Some(p) = self.db.profile(name).filter(|p| p.preload).cloned() else { return Ok(()) };
        if self.db.profile_description(&p).is_streaming {
            return Ok(());
        }
        self.load_buffer(&p.filename).map(drop)
    }

    /// `alxGetWaveLen(filename)`: length in milliseconds.
    pub fn wave_len_ms(&mut self, filename: &str) -> Result<u32> {
        Ok(self.load_buffer(filename)?.duration_ms())
    }

    // ---- sources ---------------------------------------------------------

    fn alloc_handle(&mut self) -> Handle {
        let h = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        h
    }

    fn insert_source(&mut self, desc: SoundDesc, data: Data, rate: u32, channels: u16, pos: Option<[f32; 3]>) -> Handle {
        let h = self.alloc_handle();
        let src = Source {
            gain: desc.volume,
            pitch: 1.0,
            looping: desc.is_looping,
            rolloff: 1.0,
            position: pos.unwrap_or(self.listener.position),
            direction: desc.cone_vector,
            desc,
            data,
            rate,
            channels: channels.max(1),
            cursor: 0.0,
            state: State::Initial,
            is_music: false,
        };
        self.sources.insert(h, src);
        h
    }

    /// Creates a (stopped) source for an in-memory buffer.
    pub fn create_source_pcm(&mut self, desc: SoundDesc, pcm: Arc<Pcm>, pos: Option<[f32; 3]>) -> Handle {
        let (rate, ch) = (pcm.sample_rate, pcm.channels);
        self.insert_source(desc, Data::Static(pcm), rate, ch, pos)
    }

    /// Creates a (stopped) source playing a resolved file path; streaming
    /// descriptions stream Ogg files instead of decoding them up front.
    pub fn create_source_path(&mut self, desc: SoundDesc, path: &Path, pos: Option<[f32; 3]>) -> Result<Handle> {
        let is_ogg = std::fs::File::open(path)
            .and_then(|mut f| {
                let mut magic = [0u8; 4];
                std::io::Read::read_exact(&mut f, &mut magic).map(|_| &magic == b"OggS")
            })
            .unwrap_or(false);
        if desc.is_streaming && is_ogg {
            let dec = OggStream::open(path)?;
            let (rate, ch) = (dec.sample_rate, dec.channels);
            let data = Data::Stream(Box::new(StreamData { dec, buf: VecDeque::new(), finished: false }));
            Ok(self.insert_source(desc, data, rate, ch, pos))
        } else {
            let pcm = self.load_path(path)?;
            Ok(self.create_source_pcm(desc, pcm, pos))
        }
    }

    /// `alxCreateSource(description, filename[, x, y, z])` with a description
    /// value.
    pub fn create_source_with(&mut self, desc: SoundDesc, filename: &str, pos: Option<[f32; 3]>) -> Result<Handle> {
        let path = self.resolve(filename)?;
        self.create_source_path(desc, &path, pos)
    }

    /// `alxCreateSource(profile[, x, y, z])`.
    pub fn create_source(&mut self, profile: &str, pos: Option<[f32; 3]>) -> Result<Handle> {
        let p = self.db.profile(profile).ok_or_else(|| Error(format!("unknown sound {profile}")))?;
        let desc = self.db.profile_description(p);
        let file = p.filename.clone();
        self.create_source_with(desc, &file, pos)
    }

    /// `alxCreateSource(description, filename[, x, y, z])`.
    pub fn create_source_desc(&mut self, description: &str, filename: &str, pos: Option<[f32; 3]>) -> Result<Handle> {
        let desc = self.db.description(description).cloned().ok_or_else(|| Error(format!("unknown sound description {description}")))?;
        self.create_source_with(desc, filename, pos)
    }

    /// `alxPlay(handle)`: starts a created source; returns the handle, or
    /// `NULL_HANDLE` if it does not exist.
    pub fn play(&mut self, h: Handle) -> Handle {
        match self.sources.get_mut(&h) {
            Some(s) => {
                s.state = State::Playing;
                h
            }
            None => NULL_HANDLE,
        }
    }

    /// `alxPlay(profile[, x, y, z])`.
    pub fn play_profile(&mut self, profile: &str, pos: Option<[f32; 3]>) -> Result<Handle> {
        let h = self.create_source(profile, pos)?;
        Ok(self.play(h))
    }

    /// `alxStop(handle)`: stops and frees the source.
    pub fn stop(&mut self, h: Handle) {
        self.sources.remove(&h);
        if h == self.music_handle {
            self.music_handle = NULL_HANDLE;
        }
    }

    pub fn stop_all(&mut self) {
        self.sources.clear();
        self.music_handle = NULL_HANDLE;
    }

    /// `alxIsPlaying(handle)`.
    pub fn is_playing(&self, h: Handle) -> bool {
        self.sources.get(&h).is_some_and(|s| s.state == State::Playing)
    }

    /// Handles of all existing sources.
    pub fn handles(&self) -> Vec<Handle> {
        self.sources.keys().copied().collect()
    }

    /// `alxSourcef(handle, param, value)`.
    pub fn source_f(&mut self, h: Handle, param: AlParam, v: f32) {
        let Some(s) = self.sources.get_mut(&h) else { return };
        match param {
            AlParam::Gain | AlParam::GainLinear => s.gain = v.max(0.0),
            AlParam::Pitch => s.pitch = v.clamp(0.01, 16.0),
            AlParam::Looping => s.looping = v != 0.0,
            AlParam::ReferenceDistance => s.desc.reference_distance = v,
            AlParam::MaxDistance => s.desc.max_distance = v,
            AlParam::RolloffFactor => s.rolloff = v.max(0.0),
            AlParam::ConeInnerAngle => s.desc.cone_inside_angle = v,
            AlParam::ConeOuterAngle => s.desc.cone_outside_angle = v,
            AlParam::ConeOuterGain => s.desc.cone_outside_volume = v,
            AlParam::Position | AlParam::Direction | AlParam::Velocity => {}
        }
    }

    /// `alxSource3f(handle, param, x, y, z)` (position, direction).
    pub fn source_3f(&mut self, h: Handle, param: AlParam, v: [f32; 3]) {
        let Some(s) = self.sources.get_mut(&h) else { return };
        match param {
            AlParam::Position => s.position = v,
            AlParam::Direction => s.direction = v,
            _ => {}
        }
    }

    /// `alxSetChannelVolume(channel, volume)`: returns false for an invalid
    /// channel, like the engine.
    pub fn set_channel_volume(&mut self, channel: u32, v: f32) -> bool {
        match self.channel_volume.get_mut(channel as usize) {
            Some(c) => {
                *c = v.clamp(0.0, 1.0);
                true
            }
            None => false,
        }
    }

    /// `alxGetChannelVolume(channel)`.
    pub fn channel_volume(&self, channel: u32) -> f32 {
        self.channel_volume.get(channel as usize).copied().unwrap_or(0.0)
    }

    /// `alxListenerf(param, value)` (only gain is meaningful).
    pub fn listener_f(&mut self, param: AlParam, v: f32) {
        if matches!(param, AlParam::Gain | AlParam::GainLinear) {
            self.listener.gain = v.clamp(0.0, 1.0);
        }
    }

    pub fn set_listener_position(&mut self, p: [f32; 3]) {
        self.listener.position = p;
    }

    /// `AL_ORIENTATION`: forward and up vectors.
    pub fn set_listener_orientation(&mut self, at: [f32; 3], up: [f32; 3]) {
        if let (Some(at), Some(up)) = (normalize(at), normalize(up)) {
            self.listener.at = at;
            self.listener.up = up;
        }
    }

    pub fn set_listener_transform(&mut self, pos: [f32; 3], forward: [f32; 3], up: [f32; 3]) {
        self.set_listener_position(pos);
        self.set_listener_orientation(forward, up);
    }

    // ---- music -------------------------------------------------------------

    /// `MusicPlayer.setTrack(name)`: stops the current music and starts the
    /// named `MusicDef` track on a streaming `AudioMusic` source. An empty
    /// name just stops the music.
    pub fn music_set_track(&mut self, name: &str) -> Result<()> {
        self.music_stop();
        if name.is_empty() {
            return Ok(());
        }
        let track = self.db.music(name).cloned().ok_or_else(|| Error(format!("music: '{name}' is not a music track")))?;
        let file = self.music.set_track(track);
        let mut desc = self.db.description("AudioMusic").cloned().unwrap_or_else(SoundDesc::music);
        desc.is_streaming = true;
        let h = self.create_source_with(desc, &file, None)?;
        if let Some(s) = self.sources.get_mut(&h) {
            s.is_music = true;
        }
        self.music_handle = self.play(h);
        Ok(())
    }

    /// `MusicPlayer.stop()`.
    pub fn music_stop(&mut self) {
        if self.music_handle != NULL_HANDLE {
            let h = self.music_handle;
            self.stop(h);
        }
        self.music.stop();
    }

    pub fn music_handle(&self) -> Handle {
        self.music_handle
    }

    /// Per-frame update: music mood smoothing, and — when no output device
    /// is pulling audio (nothing called [`render`](Self::render) for
    /// 500 ms) — advancing all playing sources by the wall-clock time since
    /// the previous call, so finished one-shots are released headless too.
    /// With a device, `render` advances and releases sources itself.
    pub fn update(&mut self) {
        self.music.update();
        let now = Instant::now();
        let since = self.clock.replace(now);
        let rendering = self.last_render.is_some_and(|t| now.duration_since(t) < RENDER_TIMEOUT);
        if let (Some(since), false) = (since, rendering) {
            // Do not re-advance time an output already rendered.
            let from = self.last_render.map_or(since, |t| t.max(since));
            self.advance(now.saturating_duration_since(from).as_secs_f32());
        }
    }

    /// Advances every playing source by `seconds` without producing audio,
    /// releasing those that finish (the headless counterpart of `render`).
    pub fn advance(&mut self, seconds: f32) {
        let frames = (seconds.max(0.0) as f64 * self.sample_rate as f64).round() as usize;
        if frames == 0 {
            return;
        }
        let mut scratch = Vec::new();
        let mut sources = std::mem::take(&mut self.sources);
        sources.retain(|&h, src| {
            if src.state != State::Playing {
                return true;
            }
            let alive = match src.data {
                Data::Static(_) => self.mix_source(src, (0.0, 0.0), &mut [], frames),
                Data::Stream(_) => {
                    // Streams must be decoded to move on; cap the work so a
                    // long stall does not decode minutes of music at once.
                    let mut left = frames.min(self.sample_rate as usize * 2);
                    let mut alive = true;
                    while alive && left > 0 {
                        let n = left.min(1024);
                        scratch.resize(n * 2, 0.0);
                        alive = self.mix_source(src, (0.0, 0.0), &mut scratch, n);
                        left -= n;
                    }
                    alive
                }
            };
            if !alive && h == self.music_handle {
                self.music_handle = NULL_HANDLE;
            }
            alive
        });
        self.sources = sources;
    }

    // ---- rendering ---------------------------------------------------------

    fn source_gains(&self, s: &Source) -> (f32, f32) {
        let ch = s.desc.channel as usize;
        let base = s.gain * self.channel_volume.get(ch).copied().unwrap_or(1.0) * self.listener.gain;
        if !s.desc.is_3d {
            return (base, base);
        }
        let rel = sub(s.position, self.listener.position);
        let dist = len(rel);
        let mut g = base * distance_gain(self.distance_model, dist, s.desc.reference_distance, s.desc.max_distance, s.rolloff);
        let (inner, outer) = (s.desc.cone_inside_angle, s.desc.cone_outside_angle);
        if (inner < 360.0 || outer < 360.0) && dist > 1e-6 {
            if let Some(dir) = normalize(s.direction) {
                let to_listener = [-rel[0] / dist, -rel[1] / dist, -rel[2] / dist];
                let angle = dot(dir, to_listener).clamp(-1.0, 1.0).acos().to_degrees();
                let cone = if angle <= inner / 2.0 {
                    1.0
                } else if angle >= outer / 2.0 {
                    s.desc.cone_outside_volume
                } else {
                    let t = (angle - inner / 2.0) / ((outer - inner) / 2.0).max(1e-6);
                    1.0 + t * (s.desc.cone_outside_volume - 1.0)
                };
                g *= cone;
            }
        }
        if s.channels != 1 || dist <= 1e-4 {
            return (g, g);
        }
        let right = normalize(cross(self.listener.at, self.listener.up)).unwrap_or([1.0, 0.0, 0.0]);
        let (l, r) = pan_gains(dot(rel, right) / dist);
        (g * l, g * r)
    }

    /// Mixes all playing sources into `out` (interleaved stereo; the buffer
    /// is overwritten). Finished non-looping sources are freed.
    pub fn render(&mut self, out: &mut [f32]) {
        self.last_render = Some(Instant::now());
        out.fill(0.0);
        let frames = out.len() / 2;
        // Take the map out so sources can be mixed with `&mut self` (music,
        // logging) without per-source map removals and re-insertions.
        let mut sources = std::mem::take(&mut self.sources);
        sources.retain(|&h, src| {
            if src.state != State::Playing {
                return true;
            }
            let gains = self.source_gains(src);
            let alive = self.mix_source(src, gains, &mut out[..frames * 2], frames);
            if !alive && h == self.music_handle {
                self.music_handle = NULL_HANDLE;
            }
            alive
        });
        self.sources = sources;
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }

    /// Renders `seconds` of audio offline (interleaved stereo at the mixer
    /// rate).
    pub fn render_seconds(&mut self, seconds: f32) -> Vec<f32> {
        let frames = (seconds.max(0.0) * self.sample_rate as f32).round() as usize;
        let mut out = vec![0.0; frames * 2];
        // Render in blocks like a device callback would (gains are updated
        // per block).
        for chunk in out.chunks_mut(1024) {
            self.render(chunk);
        }
        out
    }

    /// Returns false when the source finished.
    /// `out` holds `frames` stereo frames, or is empty for a silent static
    /// source (only the cursor moves).
    fn mix_source(&mut self, s: &mut Source, (gl, gr): (f32, f32), out: &mut [f32], frames: usize) -> bool {
        let step = s.rate as f64 * s.pitch as f64 / self.sample_rate as f64;
        let ch = s.channels as usize;
        match &mut s.data {
            Data::Static(pcm) => {
                let pcm = pcm.clone();
                let n = pcm.frames();
                if n == 0 {
                    return false;
                }
                let silent = (gl == 0.0 && gr == 0.0) || out.len() < frames * 2;
                if silent {
                    s.cursor += step * frames as f64;
                    if s.cursor >= n as f64 {
                        if !s.looping {
                            return false;
                        }
                        s.cursor %= n as f64;
                    }
                    return true;
                }
                let sample = |frame: usize, c: usize, looping: bool| -> f32 {
                    let f = if frame >= n {
                        if looping {
                            frame % n
                        } else {
                            return 0.0;
                        }
                    } else {
                        frame
                    };
                    pcm.samples[f * ch + c.min(ch - 1)]
                };
                for i in 0..frames {
                    let pos = s.cursor;
                    let idx = pos as usize;
                    let frac = (pos - idx as f64) as f32;
                    let l = sample(idx, 0, s.looping) * (1.0 - frac) + sample(idx + 1, 0, s.looping) * frac;
                    let r = if ch == 1 { l } else { sample(idx, 1, s.looping) * (1.0 - frac) + sample(idx + 1, 1, s.looping) * frac };
                    out[i * 2] += l * gl;
                    out[i * 2 + 1] += r * gr;
                    s.cursor += step;
                    if s.cursor >= n as f64 {
                        if !s.looping {
                            return false;
                        }
                        s.cursor -= n as f64;
                    }
                }
                true
            }
            Data::Stream(_) => self.mix_stream(s, (gl, gr), out, step),
        }
    }

    fn mix_stream(&mut self, s: &mut Source, (gl, gr): (f32, f32), out: &mut [f32], step: f64) -> bool {
        let frames = out.len() / 2;
        let ch = s.channels as usize;
        for i in 0..frames {
            let idx = s.cursor as usize;
            if !self.fill_stream(s, idx + 2) {
                // Nothing left beyond what is buffered.
                let Data::Stream(st) = &s.data else { return false };
                if st.buf.len() / ch <= idx {
                    return false;
                }
            }
            let Data::Stream(st) = &mut s.data else { return false };
            let avail = st.buf.len() / ch;
            let get = |f: usize, c: usize| if f < avail { st.buf[f * ch + c.min(ch - 1)] } else { 0.0 };
            let frac = (s.cursor - idx as f64) as f32;
            let l = get(idx, 0) * (1.0 - frac) + get(idx + 1, 0) * frac;
            let r = if ch == 1 { l } else { get(idx, 1) * (1.0 - frac) + get(idx + 1, 1) * frac };
            out[i * 2] += l * gl;
            out[i * 2 + 1] += r * gr;
            s.cursor += step;
            let drop = (s.cursor as usize).min(avail);
            if drop > 0 {
                st.buf.drain(..drop * ch);
                s.cursor -= drop as f64;
            }
        }
        true
    }

    /// Decodes until the stream buffer holds `frames` frames; at the end of
    /// a file continues with the next music segment, rewinds a looping
    /// stream, or marks the stream finished. Returns false if it could not.
    fn fill_stream(&mut self, s: &mut Source, frames: usize) -> bool {
        let ch = s.channels as usize;
        let mut transitions = 0;
        loop {
            let Data::Stream(st) = &mut s.data else { return false };
            if st.buf.len() / ch >= frames {
                return true;
            }
            if st.finished {
                return false;
            }
            let mut tmp = VecDeque::new();
            match st.dec.decode_packet(&mut tmp) {
                Ok(true) => {
                    append_converted(&mut st.buf, tmp, st.dec.channels as usize, ch);
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    let msg = e.to_string();
                    st.finished = true;
                    self.log(msg);
                    return false;
                }
            }
            // End of the current file.
            transitions += 1;
            if transitions > 4 {
                st.finished = true;
                return false;
            }
            let current = st.dec.path().to_path_buf();
            let next_path = if s.is_music {
                match self.music.on_segment_end() {
                    Some(file) => match self.db.resolve_file(&file) {
                        Some(p) => Some(p),
                        None => {
                            self.log(format!("music segment not found: {file}"));
                            Some(current)
                        }
                    },
                    None if self.music.track().is_some() => Some(current),
                    None => s.looping.then_some(current),
                }
            } else {
                s.looping.then_some(current)
            };
            let Data::Stream(st) = &mut s.data else { return false };
            match next_path.map(|p| OggStream::open(&p)) {
                Some(Ok(dec)) => {
                    if dec.sample_rate != s.rate {
                        let msg = format!("{}: sample rate {} differs from stream rate {}", dec.path().display(), dec.sample_rate, s.rate);
                        st.dec = dec;
                        self.log(msg);
                    } else {
                        st.dec = dec;
                    }
                }
                Some(Err(e)) => {
                    st.finished = true;
                    self.log(e.to_string());
                    return false;
                }
                None => {
                    st.finished = true;
                    return false;
                }
            }
        }
    }
}

/// Appends interleaved samples converting `from` channels to `to` channels.
fn append_converted(buf: &mut VecDeque<f32>, src: VecDeque<f32>, from: usize, to: usize) {
    if from == to {
        buf.extend(src);
        return;
    }
    let from = from.max(1);
    let src: Vec<f32> = src.into();
    for frame in src.chunks(from) {
        let mono = frame.iter().sum::<f32>() / frame.len() as f32;
        for c in 0..to {
            buf.push_back(if from == 1 { frame[0] } else if c < frame.len() && to > 1 { frame[c] } else { mono });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, channels: u16, seconds: f32, amp: f32) -> Arc<Pcm> {
        let n = (rate as f32 * seconds) as usize;
        let mut samples = Vec::with_capacity(n * channels as usize);
        for i in 0..n {
            let v = amp * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin();
            for _ in 0..channels {
                samples.push(v);
            }
        }
        Arc::new(Pcm { sample_rate: rate, channels, samples })
    }

    fn dc(rate: u32, frames: usize, v: f32) -> Arc<Pcm> {
        Arc::new(Pcm { sample_rate: rate, channels: 1, samples: vec![v; frames] })
    }

    fn peak(buf: &[f32], ch: usize) -> f32 {
        buf.chunks(2).fold(0.0f32, |m, f| m.max(f[ch].abs()))
    }

    fn d3(reference: f32, max: f32) -> SoundDesc {
        SoundDesc { is_3d: true, reference_distance: reference, max_distance: max, ..Default::default() }
    }

    #[test]
    fn inverse_distance_clamped() {
        let m = DistanceModel::InverseClamped;
        assert_eq!(distance_gain(m, 0.0, 10.0, 60.0, 1.0), 1.0);
        assert_eq!(distance_gain(m, 10.0, 10.0, 60.0, 1.0), 1.0);
        assert!((distance_gain(m, 20.0, 10.0, 60.0, 1.0) - 0.5).abs() < 1e-6);
        assert!((distance_gain(m, 59.0, 10.0, 60.0, 1.0) - 10.0 / 59.0).abs() < 1e-6);
        assert_eq!(distance_gain(m, 60.0, 10.0, 60.0, 1.0), 0.0, "culled at maxDistance");
        let l = DistanceModel::Linear;
        assert!((distance_gain(l, 35.0, 10.0, 60.0, 1.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn plays_2d_at_full_volume_and_finishes() {
        let mut m = Mixer::new(22050, AudioDb::new());
        let h = m.create_source_pcm(SoundDesc::default(), dc(22050, 2205, 0.5), None);
        assert!(!m.is_playing(h));
        assert_eq!(m.play(h), h);
        let out = m.render_seconds(0.05);
        assert!((out[0] - 0.5).abs() < 1e-6 && (out[1] - 0.5).abs() < 1e-6);
        // 0.1 s source: still playing after 0.05 s, gone after 0.2 s.
        assert!(m.is_playing(h));
        let out = m.render_seconds(0.15);
        assert!(!m.is_playing(h));
        assert_eq!(*out.last().unwrap(), 0.0);
    }

    #[test]
    fn looping_keeps_playing() {
        let mut m = Mixer::new(44100, AudioDb::new());
        let desc = SoundDesc { is_looping: true, ..Default::default() };
        let h = m.create_source_pcm(desc, dc(44100, 100, 0.25), None);
        m.play(h);
        let out = m.render_seconds(1.0);
        assert!(m.is_playing(h));
        assert!(out.iter().all(|&s| (s - 0.25).abs() < 1e-6));
        m.stop(h);
        assert!(!m.is_playing(h));
        assert!(m.render_seconds(0.1).iter().all(|&s| s == 0.0));
    }

    #[test]
    fn channel_and_master_volume() {
        let mut m = Mixer::new(8000, AudioDb::new());
        let desc = SoundDesc { volume: 0.5, channel: 2, is_looping: true, ..Default::default() };
        let h = m.create_source_pcm(desc, dc(8000, 800, 1.0), None);
        m.play(h);
        assert!(m.set_channel_volume(2, 0.5));
        assert!(!m.set_channel_volume(99, 0.5));
        m.listener_f(AlParam::GainLinear, 0.5);
        let out = m.render_seconds(0.01);
        assert!((out[0] - 0.125).abs() < 1e-6, "{}", out[0]);
        m.source_f(h, AlParam::Gain, 1.0);
        let out = m.render_seconds(0.01);
        assert!((out[0] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn panning_follows_listener_orientation() {
        let mut m = Mixer::new(8000, AudioDb::new());
        // Source 5 m to the right (+x) of a listener looking along +y.
        let h = m_create(&mut m, d3(10.0, 100.0), [5.0, 0.0, 0.0]);
        m.play(h);
        let out = m.render_seconds(0.05);
        assert!(peak(&out, 1) > 0.99 && peak(&out, 0) < 0.01, "right only: {} {}", peak(&out, 0), peak(&out, 1));
        // Turn around: now it is on the left.
        m.set_listener_orientation([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
        let out = m.render_seconds(0.05);
        assert!(peak(&out, 0) > 0.99 && peak(&out, 1) < 0.01);
        // Straight ahead: centred at unity.
        m.source_3f(h, AlParam::Position, [0.0, -5.0, 0.0]);
        let out = m.render_seconds(0.05);
        assert!((peak(&out, 0) - peak(&out, 1)).abs() < 1e-3 && peak(&out, 0) > 0.99);
    }

    fn m_create(m: &mut Mixer, desc: SoundDesc, pos: [f32; 3]) -> Handle {
        let desc = SoundDesc { is_looping: true, ..desc };
        m.create_source_pcm(desc, tone(8000, 1, 0.5, 1.0), Some(pos))
    }

    #[test]
    fn attenuation_and_culling() {
        let mut m = Mixer::new(8000, AudioDb::new());
        let h = m_create(&mut m, d3(10.0, 60.0), [0.0, 20.0, 0.0]);
        m.play(h);
        let out = m.render_seconds(0.05);
        assert!((peak(&out, 0) - 0.5).abs() < 0.01, "{}", peak(&out, 0));
        m.source_3f(h, AlParam::Position, [0.0, 70.0, 0.0]);
        assert!(m.render_seconds(0.05).iter().all(|&s| s == 0.0));
        assert!(m.is_playing(h), "culled loops stay alive");
        m.set_listener_position([0.0, 65.0, 0.0]);
        assert!(peak(&m.render_seconds(0.05), 0) > 0.99);
    }

    #[test]
    fn cone() {
        let mut m = Mixer::new(8000, AudioDb::new());
        let desc = SoundDesc { cone_inside_angle: 90.0, cone_outside_angle: 180.0, cone_outside_volume: 0.25, cone_vector: [0.0, 1.0, 0.0], ..d3(10.0, 100.0) };
        // Source in front of the listener pointing away from it.
        let h = m_create(&mut m, desc, [0.0, 5.0, 0.0]);
        m.play(h);
        assert!((peak(&m.render_seconds(0.05), 0) - 0.25).abs() < 0.01);
        m.source_3f(h, AlParam::Direction, [0.0, -1.0, 0.0]);
        assert!(peak(&m.render_seconds(0.05), 0) > 0.99);
    }

    #[test]
    fn pitch_and_resampling() {
        // A 22050 Hz, 0.1 s source rendered at 44100 Hz lasts 0.1 s; with
        // pitch 2 it lasts 0.05 s.
        let mut m = Mixer::new(44100, AudioDb::new());
        let h = m.create_source_pcm(SoundDesc::default(), dc(22050, 2205, 1.0), None);
        m.play(h);
        let out = m.render_seconds(0.2);
        let played = out.chunks(2).filter(|f| f[0] != 0.0).count();
        assert!((played as i64 - 4410).abs() <= 2, "{played}");
        assert!(!m.is_playing(h));
        let h = m.create_source_pcm(SoundDesc::default(), dc(22050, 2205, 1.0), None);
        m.play(h);
        m.source_f(h, AlParam::Pitch, 2.0);
        let out = m.render_seconds(0.2);
        let played = out.chunks(2).filter(|f| f[0] != 0.0).count();
        assert!((played as i64 - 2205).abs() <= 2, "{played}");
    }

    #[test]
    fn stop_all_and_unknowns() {
        let mut m = Mixer::new(8000, AudioDb::new());
        let a = m.create_source_pcm(SoundDesc::default(), dc(8000, 8000, 0.1), None);
        m.play(a);
        let b = m.create_source_pcm(SoundDesc::default(), dc(8000, 8000, 0.1), None);
        m.play(b);
        assert_ne!(a, b);
        assert_eq!(m.handles().len(), 2);
        m.stop_all();
        assert!(m.handles().is_empty());
        assert_eq!(m.play(12345), NULL_HANDLE);
        assert!(m.play_profile("NoSuchProfile", None).is_err());
        assert!(m.music_set_track("NoSuchTrack").is_err());
    }

    #[test]
    fn stereo_buffers_are_not_panned() {
        let mut m = Mixer::new(8000, AudioDb::new());
        let desc = SoundDesc { is_looping: true, ..d3(10.0, 100.0) };
        let h = m.create_source_pcm(desc, tone(8000, 2, 0.5, 1.0), Some([5.0, 0.0, 0.0]));
        m.play(h);
        let out = m.render_seconds(0.05);
        assert!((peak(&out, 0) - peak(&out, 1)).abs() < 1e-3);
    }

    fn fire_shots(m: &mut Mixer, n: usize) {
        let pcm = dc(22050, 2205, 0.001); // 0.1 s one-shot
        let before = m.handles().len();
        for i in 0..n {
            // Mix of 2D and 3D (some culled beyond max_distance) one-shots.
            let desc = if i % 2 == 0 { SoundDesc::default() } else { d3(10.0, 60.0) };
            let h = m.create_source_pcm(desc, pcm.clone(), Some([0.0, (i % 100) as f32, 0.0]));
            m.play(h);
        }
        assert_eq!(m.handles().len(), before + n);
    }

    #[test]
    fn one_shots_are_released_with_output() {
        // The output callback path: render() pulls audio.
        let mut m = Mixer::new(44100, AudioDb::new());
        let looping = m.create_source_pcm(SoundDesc { is_looping: true, ..Default::default() }, dc(44100, 441, 0.1), None);
        m.play(looping);
        fire_shots(&mut m, 1000);
        m.render_seconds(0.05);
        assert_eq!(m.handles().len(), 1001, "still playing halfway");
        m.render_seconds(0.1);
        assert_eq!(m.handles(), vec![looping], "only the loop survives");
    }

    #[test]
    fn one_shots_are_released_headless() {
        let mut m = Mixer::new(44100, AudioDb::new());
        fire_shots(&mut m, 1000);
        m.advance(0.05);
        assert_eq!(m.handles().len(), 1000);
        m.advance(0.06);
        assert!(m.handles().is_empty());

        // The engine only calls update() once per tick: wall-clock driven.
        fire_shots(&mut m, 1000);
        m.update();
        std::thread::sleep(Duration::from_millis(150));
        m.update();
        assert!(m.handles().is_empty(), "{} left", m.handles().len());
    }

    #[test]
    fn update_does_not_double_advance_while_rendering() {
        let mut m = Mixer::new(8000, AudioDb::new());
        let h = m.create_source_pcm(SoundDesc::default(), dc(8000, 8000, 0.1), None);
        m.play(h);
        m.update();
        std::thread::sleep(Duration::from_millis(30));
        m.render_seconds(0.01);
        m.update();
        assert!(m.is_playing(h));
        // Exactly 0.99 s left: 0.98 s more keeps it alive, render-driven only.
        m.render_seconds(0.98);
        m.update();
        assert!(m.is_playing(h));
        m.render_seconds(0.02);
        assert!(!m.is_playing(h));
    }

    #[test]
    fn channel_conversion() {
        let mut buf = VecDeque::new();
        append_converted(&mut buf, VecDeque::from(vec![0.5, -0.5]), 1, 2);
        assert_eq!(Vec::from(buf.clone()), vec![0.5, 0.5, -0.5, -0.5]);
        buf.clear();
        append_converted(&mut buf, VecDeque::from(vec![1.0, 0.0]), 2, 1);
        assert_eq!(Vec::from(buf), vec![0.5]);
    }
}
