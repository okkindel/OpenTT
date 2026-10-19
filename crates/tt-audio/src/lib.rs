//! Audio layer of the game.
//!
//! This crate reimplements that layer in software:
//!
//! * [`decode`]: Ogg Vorbis (lewton) and WAV (hound) to `f32` PCM, plus an
//!   Ogg packet streamer for music.
//! * [`db`]: `SoundDescDef`, `SoundDef` and the game-specific
//!   `MusicDef` objects, declared from the game's data files.
//! * [`mixer`]: an OpenAL-like mixer with the `alx*` script API (sources,
//!   handles, channel volumes, listener, inverse-distance attenuation,
//!   panning, looping, streaming) and offline rendering.
//! * [`music`]: the `MusicPlayer` segment sequencer, reconstructed from the
//!   game executable.
//! * `output` (feature `output`, on by default): real-time playback via cpal.
//!
//! Uncertain points: `alxSourcef(AL_GAIN)` is treated as linear like
//! `AL_GAIN_LINEAR`.

pub mod db;
pub mod decode;
pub mod mixer;
pub mod music;
#[cfg(feature = "output")]
pub mod output;

pub use db::{AudioDb, SoundDesc, SoundProfile, MusicTrack};
pub use decode::Pcm;
pub use mixer::{AlParam, DistanceModel, Handle, Mixer, SharedMixer, NULL_HANDLE};
pub use music::MusicPlayer;

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
