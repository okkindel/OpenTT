//! Decoding of the game's sound files (Ogg Vorbis and RIFF WAV) to `f32` PCM.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::{Path, PathBuf};

use lewton::inside_ogg::OggStreamReader;

use crate::{Error, Result};

/// Decoded audio: interleaved `f32` samples in `[-1, 1]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    /// Length in seconds.
    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.sample_rate.max(1) as f64
    }

    /// Length in milliseconds, as `alxGetWaveLen` reports it.
    pub fn duration_ms(&self) -> u32 {
        (self.duration() * 1000.0).round() as u32
    }

    /// Peak absolute sample value.
    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Ogg,
    Wav,
}

/// Detects the container from the file's magic bytes (extensions in the game
/// data lie: profiles name `.wav` files that only exist as `.ogg`).
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"OggS") {
        Some(Format::Ogg)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Some(Format::Wav)
    } else {
        None
    }
}

/// Decodes a whole Ogg Vorbis or WAV file held in memory.
pub fn decode(bytes: &[u8]) -> Result<Pcm> {
    match sniff(bytes) {
        Some(Format::Ogg) => decode_ogg(Cursor::new(bytes)),
        Some(Format::Wav) => decode_wav(Cursor::new(bytes)),
        None => Err(Error("unknown audio format (expected Ogg Vorbis or RIFF WAVE)".into())),
    }
}

pub fn decode_file(path: &Path) -> Result<Pcm> {
    let bytes = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    decode(&bytes).map_err(|e| Error(format!("{}: {e}", path.display())))
}

fn decode_ogg<R: Read + Seek>(r: R) -> Result<Pcm> {
    let mut rd = OggStreamReader::new(r).map_err(|e| Error(format!("vorbis: {e}")))?;
    let channels = rd.ident_hdr.audio_channels as u16;
    let sample_rate = rd.ident_hdr.audio_sample_rate;
    let mut samples = Vec::new();
    while let Some(pkt) = rd.read_dec_packet_itl().map_err(|e| Error(format!("vorbis: {e}")))? {
        samples.extend(pkt.iter().map(|&s| s as f32 / 32768.0));
    }
    Ok(Pcm { sample_rate, channels, samples })
}

fn decode_wav<R: Read>(r: R) -> Result<Pcm> {
    let mut rd = hound::WavReader::new(r).map_err(|e| Error(format!("wav: {e}")))?;
    let spec = rd.spec();
    let samples: std::result::Result<Vec<f32>, _> = match spec.sample_format {
        hound::SampleFormat::Float => rd.samples::<f32>().collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            if spec.bits_per_sample <= 8 {
                // hound maps unsigned 8-bit to signed i8 already.
                rd.samples::<i8>().map(|s| s.map(|v| v as f32 * scale)).collect()
            } else if spec.bits_per_sample <= 16 {
                rd.samples::<i16>().map(|s| s.map(|v| v as f32 * scale)).collect()
            } else {
                rd.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect()
            }
        }
    };
    let samples = samples.map_err(|e| Error(format!("wav: {e}")))?;
    Ok(Pcm { sample_rate: spec.sample_rate, channels: spec.channels, samples })
}

/// Incremental Ogg Vorbis decoder for streamed sources (music): decodes one
/// Vorbis packet at a time from the open file.
pub struct OggStream {
    path: PathBuf,
    reader: OggStreamReader<BufReader<File>>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl OggStream {
    pub fn open(path: &Path) -> Result<OggStream> {
        let f = File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let reader = OggStreamReader::new(BufReader::new(f))
            .map_err(|e| Error(format!("{}: vorbis: {e}", path.display())))?;
        Ok(OggStream {
            path: path.to_path_buf(),
            sample_rate: reader.ident_hdr.audio_sample_rate,
            channels: reader.ident_hdr.audio_channels as u16,
            reader,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends the next decoded packet (interleaved) to `out`. Returns `false`
    /// at the end of the stream.
    pub fn decode_packet(&mut self, out: &mut VecDeque<f32>) -> Result<bool> {
        loop {
            match self.reader.read_dec_packet_itl() {
                Ok(Some(pkt)) if pkt.is_empty() => continue,
                Ok(Some(pkt)) => {
                    out.extend(pkt.iter().map(|&s| s as f32 / 32768.0));
                    return Ok(true);
                }
                Ok(None) => return Ok(false),
                Err(e) => return Err(Error(format!("{}: vorbis: {e}", self.path.display()))),
            }
        }
    }
}

impl std::fmt::Debug for OggStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OggStream")
            .field("path", &self.path)
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_bytes(spec: hound::WavSpec, samples: &[i16]) -> Vec<u8> {
        let mut cur = Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
            for &s in samples {
                w.write_sample(s).unwrap();
            }
            w.finalize().unwrap();
        }
        cur.into_inner()
    }

    #[test]
    fn decodes_16bit_wav() {
        let spec = hound::WavSpec { channels: 2, sample_rate: 22050, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let bytes = wav_bytes(spec, &[0, 16384, -32768, 32767]);
        assert_eq!(sniff(&bytes), Some(Format::Wav));
        let pcm = decode(&bytes).unwrap();
        assert_eq!((pcm.sample_rate, pcm.channels, pcm.frames()), (22050, 2, 2));
        assert_eq!(pcm.samples[1], 0.5);
        assert_eq!(pcm.samples[2], -1.0);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"not audio at all").is_err());
    }
}
