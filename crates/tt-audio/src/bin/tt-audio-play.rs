//! `tt-audio-play <file.ogg|wav> [--seconds N] [--loop]`: plays a sound file
//! through the default audio device and exits after the file (or N seconds,
//! default at most 30) has played.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use tt_audio::{decode, output::Output, AudioDb, SoundDesc, Mixer};

fn main() -> ExitCode {
    let mut file = None;
    let mut seconds: Option<f32> = None;
    let mut looping = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().and_then(|v| v.parse().ok()),
            "--loop" => looping = true,
            "-h" | "--help" => {
                println!("usage: tt-audio-play <file.ogg|wav> [--seconds N] [--loop]");
                return ExitCode::SUCCESS;
            }
            _ => file = Some(PathBuf::from(a)),
        }
    }
    let Some(file) = file else {
        eprintln!("usage: tt-audio-play <file.ogg|wav> [--seconds N] [--loop]");
        return ExitCode::from(2);
    };
    let pcm = match decode::decode_file(&file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("{}: {} Hz, {} ch, {:.2} s, peak {:.3}", file.display(), pcm.sample_rate, pcm.channels, pcm.duration(), pcm.peak());
    let duration = seconds.unwrap_or(if looping { 10.0 } else { pcm.duration() as f32 }).clamp(0.0, 30.0);

    let mixer = Mixer::new(44100, AudioDb::new()).shared();
    let desc = SoundDesc { is_looping: looping, is_streaming: true, ..Default::default() };
    let handle = match mixer.lock().unwrap().create_source_path(desc, &file, None) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out = match Output::start(mixer.clone()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: cannot open audio output: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("playing on {} ({} Hz, {} ch) for {:.2} s", out.device_name, out.sample_rate, out.channels, duration);
    mixer.lock().unwrap().play(handle);
    let deadline = std::time::Instant::now() + Duration::from_secs_f32(duration + 0.2);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        let m = mixer.lock().unwrap();
        if !m.is_playing(handle) {
            break;
        }
    }
    for e in mixer.lock().unwrap().take_errors() {
        eprintln!("warning: {e}");
    }
    drop(out);
    ExitCode::SUCCESS
}
