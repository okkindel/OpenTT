//! Real-time playback of a [`SharedMixer`] through the default audio device
//! (cpal).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, Stream, StreamConfig};

use crate::mixer::SharedMixer;
use crate::{Error, Result};

/// A running output stream. Audio stops when it is dropped.
pub struct Output {
    _stream: Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Output {
    /// Opens the default output device and starts pulling audio from
    /// `mixer`. The mixer's sample rate is switched to the device's.
    pub fn start(mixer: SharedMixer) -> Result<Output> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| Error(format!("no audio output device (host {:?})", host.id())))?;
        let device_name = device.name().unwrap_or_else(|_| "<unknown>".into());
        let supported = device.default_output_config().map_err(|e| Error(format!("audio device {device_name}: {e}")))?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let (sample_rate, channels) = (config.sample_rate.0, config.channels);
        if let Ok(mut m) = mixer.lock() {
            m.set_sample_rate(sample_rate);
        }
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, mixer),
            SampleFormat::I16 => build::<i16>(&device, &config, mixer),
            SampleFormat::U16 => build::<u16>(&device, &config, mixer),
            SampleFormat::I32 => build::<i32>(&device, &config, mixer),
            other => return Err(Error(format!("audio device {device_name}: unsupported sample format {other}"))),
        }
        .map_err(|e| Error(format!("audio device {device_name}: {e}")))?;
        stream.play().map_err(|e| Error(format!("audio device {device_name}: {e}")))?;
        Ok(Output { _stream: stream, device_name, sample_rate, channels })
    }
}

fn build<T>(device: &cpal::Device, config: &StreamConfig, mixer: SharedMixer) -> std::result::Result<Stream, cpal::BuildStreamError>
where
    T: SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut stereo: Vec<f32> = Vec::new();
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let frames = data.len() / channels.max(1);
            stereo.resize(frames * 2, 0.0);
            match mixer.lock() {
                Ok(mut m) => m.render(&mut stereo),
                Err(_) => stereo.fill(0.0),
            }
            for (f, out) in data.chunks_mut(channels.max(1)).enumerate() {
                let (l, r) = (stereo[f * 2], stereo[f * 2 + 1]);
                for (c, s) in out.iter_mut().enumerate() {
                    let v = match (channels, c) {
                        (1, _) => 0.5 * (l + r),
                        (_, 0) => l,
                        (_, 1) => r,
                        _ => 0.0,
                    };
                    *s = T::from_sample(v);
                }
            }
        },
        |e| eprintln!("tt-audio: output stream error: {e}"),
        None,
    )
}
