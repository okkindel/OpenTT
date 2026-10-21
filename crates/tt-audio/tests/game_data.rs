//! Tests against the game data of the asset directory: its sounds and the
//! sound declarations of the data file `data/sounds.ron`
//! (the game's, interface and music sounds), as the game declares them.
//! Run with `--nocapture` to see the sound inventory.

use std::path::PathBuf;

use tt_assets::data::defs::{self as dbf, DefSet};
use tt_audio::{decode, AudioDb, Mixer};

/// The asset directory.
fn assets() -> PathBuf {
    tt_assets::layout::default_dir()
}

/// The sound objects of the data files (parents merged), with their files
/// resolved in the asset directory.
fn script_db() -> Option<AudioDb> {
    let root = assets();
    let mut set = DefSet::new();
    for stem in dbf::SETS {
        for d in dbf::load_file(&root, stem).unwrap() {
            set.declare_def(&d);
        }
    }
    let mut db = AudioDb::new();
    db.game_root = Some(root.clone());
    for o in set.objects() {
        db.set_object(&o.class, &o.name, &o.fields);
    }
    for stem in [dbf::CLIENT_AUDIO, dbf::MUSIC] {
        for o in dbf::load_client_file(&root, stem).unwrap() {
            let fields: Vec<(String, String)> = o.to_fields().into_iter().map(|f| (f.key(), f.value)).collect();
            db.set_object(o.class(), o.name(), &fields);
        }
    }
    Some(db)
}

#[test]
fn every_sound_file_decodes() {
    let mut files: Vec<_> = tt_assets::layout::files(&assets().join("sound")).into_iter().map(|(_, p)| p).collect();
    files.retain(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("ogg") || e.eq_ignore_ascii_case("wav")));
    files.sort();
    assert!(!files.is_empty());
    let mut total = 0.0;
    for f in &files {
        let pcm = decode::decode_file(f).unwrap_or_else(|e| panic!("{e}"));
        assert!(pcm.frames() > 0, "{}: empty", f.display());
        assert!(pcm.peak() > 0.0, "{}: silent", f.display());
        assert!(matches!(pcm.channels, 1 | 2));
        total += pcm.duration();
        println!(
            "{:<20} {:>5} Hz {} ch {:>7.3} s",
            f.file_name().unwrap().to_string_lossy(),
            pcm.sample_rate,
            pcm.channels,
            pcm.duration()
        );
    }
    println!("{} files, {:.1} s total", files.len(), total);
}

#[test]
fn audio_defs_resolve_and_decode() {
    tt_assets::require_game_data!();
    let Some(db) = script_db() else { return };
    println!("{} descriptions, {} profiles, {} music tracks", db.descriptions().len(), db.profiles().len(), db.music_tracks().len());
    for d in db.descriptions() {
        println!(
            "  desc {:<26} vol {:<4} loop {} 3d {} stream {} ref {:<4} max {:<4} type {}",
            d.name, d.volume, d.is_looping as u8, d.is_3d as u8, d.is_streaming as u8, d.reference_distance, d.max_distance, d.channel
        );
    }
    assert_eq!(db.descriptions().len(), 15);
    assert_eq!(db.profiles().len(), 43);
    // Lush, Spooky, Frantic and the Moon's Lunar.
    assert_eq!(db.music_tracks().len(), 4);

    let music = db.description("AudioMusic").unwrap();
    assert!(music.is_looping && music.is_streaming && !music.is_3d);
    assert_eq!(music.channel, 4);
    assert_eq!(db.description("AudioGui").unwrap().channel, 1);
    let med = db.description("AudioDefault3DMed").unwrap();
    assert_eq!((med.volume, med.reference_distance, med.max_distance, med.channel), (0.6, 50.0, 100.0, 2));

    for p in db.profiles() {
        let desc = db.description(&p.description).unwrap_or_else(|| panic!("{}: description {} missing", p.name, p.description));
        let path = db.resolve_file(&p.filename).unwrap_or_else(|| panic!("{}: {} not found", p.name, p.filename));
        let pcm = decode::decode_file(&path).unwrap();
        assert!(pcm.frames() > 0);
        println!("  profile {:<20} {:<28} -> {:<18} {}", p.name, p.filename, path.file_name().unwrap().to_string_lossy(), desc.name);
    }
    for t in db.music_tracks() {
        assert_eq!(t.num_moods, 1);
        assert!(t.sequence_count(1) >= 2);
        for f in t.files() {
            let path = db.resolve_file(f).unwrap_or_else(|| panic!("{}: {f} not found", t.name));
            decode::decode_file(&path).unwrap();
        }
        // Every sequence token must reference an existing segment.
        for s in t.sequences[0].iter().filter(|s| !s.is_empty()) {
            for tok in s.split_whitespace() {
                let n: usize = tok.parse().unwrap();
                assert!(n >= 1 && !t.moods[0][n - 1].is_empty(), "{}: bad token {tok}", t.name);
            }
        }
        println!("  music {:<8} {:?} {:?}", t.name, t.moods[0], t.sequences[0]);
    }

    // Every mission's music_track names one of the tracks.
    let missions = assets().join("missions");
    for e in std::fs::read_dir(missions).unwrap().flatten() {
        let p = e.path();
        // (missions/catalog.ron is the map list, no mission.)
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ron")) || p.ends_with("catalog.ron") {
            continue;
        }
        let m = tt_assets::data::mission::load(&p).unwrap();
        for o in m.scene_objects(|_| true).iter().flat_map(|o| o.walk()) {
            if let Some(name) = o.get("music_track") {
                assert!(db.music(name).is_some(), "{}: unknown track {name}", p.display());
            }
        }
    }
}

#[test]
fn mixer_plays_profiles_and_music() {
    tt_assets::require_game_data!();
    let Some(db) = script_db() else { return };
    let mut m = Mixer::new(44100, db);
    assert!(m.preload_profiles().is_empty());
    m.set_channel_volume(4, 0.8);

    // A GUI sound (2D) and a 3D explosion near the listener.
    let gui = m.play_profile("AudioButtonDown", None).unwrap();
    let boom = m.play_profile("DefaultBoom", Some([10.0, 10.0, 0.0])).unwrap();
    assert!(m.is_playing(gui) && m.is_playing(boom));
    let chat = m.create_source_desc("AudioMessage", "sound/voice/vochealth.ogg", None).unwrap();
    m.source_f(chat, tt_audio::AlParam::Pitch, 1.2);
    m.play(chat);
    assert!(m.wave_len_ms("sound/ui/ticktoc.wav").unwrap() > 0);

    m.music_set_track("Frantic").unwrap();
    let music = m.music_handle();
    assert!(m.is_playing(music));
    let first = m.music().current_file().to_string();
    assert!(first.contains("/r"), "{first}");

    let out = m.render_seconds(2.0);
    assert!(out.iter().any(|&s| s.abs() > 0.01));
    assert!(!m.is_playing(gui), "short GUI click finished");
    assert!(m.is_playing(music));

    // Render past the end of the first segment to cross into the next one.
    let seg = m.load_buffer(&first).unwrap().duration() as f32;
    let out = m.render_seconds(seg + 1.0);
    let tail = &out[out.len() - 44100..];
    assert!(tail.iter().any(|&s| s.abs() > 0.01), "music continues after the first segment");
    assert!(m.is_playing(music));
    assert!(m.take_errors().is_empty());

    m.music_stop();
    assert!(!m.is_playing(music));
    m.stop_all();
    assert!(m.render_seconds(0.1).iter().all(|&s| s == 0.0));
}

/// Mixer cost of a busy battle: music, 16 tanks' looping engine sounds
/// (two each, 3D, pitched) and a burst of explosions; prints the time per
/// second of audio and a checksum of the samples (the mixing must not
/// change them). `cargo test --release -p tt-audio --test game_data mixer_cost -- --ignored --nocapture`
#[test]
#[ignore]
fn mixer_cost() {
    let Some(db) = script_db() else { return };
    let mut m = Mixer::new(44100, db);
    assert!(m.preload_profiles().is_empty());
    m.music_set_track("Frantic").unwrap();
    let mut engines = Vec::new();
    for i in 0..32 {
        let h = m.create_source(if i % 2 == 0 { "TankEngineSound" } else { "SaucerEngineSound" }, Some([i as f32 * 3.0, 5.0, 0.0])).unwrap();
        m.source_f(h, tt_audio::AlParam::Pitch, 0.7 + i as f32 * 0.02);
        engines.push(m.play(h));
    }
    let t = std::time::Instant::now();
    let mut sum = 0.0f64;
    for s in 0..10 {
        for k in 0..8 {
            m.play_profile("DefaultBoom", Some([k as f32 * 7.0, s as f32, 0.0])).unwrap();
        }
        let out = m.render_seconds(1.0);
        sum += out.iter().enumerate().map(|(i, &v)| v as f64 * ((i % 97) as f64 + 1.0)).sum::<f64>();
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!("[mixer] {} sources: {:.2} ms per second of audio ({:.2}% of one core); checksum {sum:.6}", engines.len() + 9, ms / 10.0, ms / 100.0);
    assert!(m.take_errors().is_empty());
}
