//! The client's sound data: the interface sounds and the music of `data/sounds.ron`
//! must give exactly these declarations of the game install (in our names,
//! `tt_import::names`).

use tt_assets::data::defs::{load_client_file, CLIENT_AUDIO, MUSIC};
use tt_assets::data::Field;
use tt_import::decl::ScriptField;

fn f(n: &str, i: &str, v: &str) -> ScriptField {
    ScriptField::new(n, i, v)
}
fn g(n: &str, i: &str, v: &str) -> ScriptField {
    ScriptField::variable(n, i, v)
}

fn declarations(stem: &str) -> Vec<(&'static str, &'static str, Vec<ScriptField>)> {
    if stem == CLIENT_AUDIO {
        vec![
            ("AudioDescription", "AudioGui", vec![f("volume", "", "1"), f("isLooping", "", "0"), f("is3D", "", "0"), g("type", "", "$GuiAudioType")]),
            ("AudioDescription", "AudioMessage", vec![f("volume", "", "1"), f("isLooping", "", "0"), f("is3D", "", "0"), g("type", "", "$MessageAudioType")]),
            ("AudioDescription", "AudioMusic", vec![f("volume", "", "1"), f("isLooping", "", "1"), f("isStreaming", "", "1"), f("is3D", "", "0"), g("type", "", "$MusicAudioType")]),
            ("AudioProfile", "AudioButtonOver", vec![f("fileName", "", "sound/ui/mouseover.wav"), f("description", "", "AudioGui"), f("preload", "", "1")]),
            ("AudioProfile", "AudioButtonDown", vec![f("fileName", "", "sound/ui/mousedown.wav"), f("description", "", "AudioGui"), f("preload", "", "1")]),
            ("AudioProfile", "AudioTickToc", vec![f("fileName", "", "sound/ui/ticktoc.wav"), f("description", "", "AudioGui"), f("preload", "", "1")]),
            ("AudioProfile", "SPSurvival", vec![f("fileName", "", "sound/ui/spsurvival.ogg"), f("description", "", "AudioGui"), f("preload", "", "1")]),
            ("AudioProfile", "SPExtra", vec![f("fileName", "", "sound/ui/spextra.ogg"), f("description", "", "AudioGui"), f("preload", "", "1")]),
            ("AudioProfile", "SPQuick", vec![f("fileName", "", "sound/ui/spquick.ogg"), f("description", "", "AudioGui"), f("preload", "", "1")]),
            ("AudioProfile", "SPCount1", vec![f("fileName", "", "sound/ui/spcount1.ogg"), f("description", "", "AudioGui"), f("preload", "", "1")]),
            ("AudioProfile", "SPCount2", vec![f("fileName", "", "sound/ui/spcount2.wav"), f("description", "", "AudioGui"), f("preload", "", "1")]),
            ("AudioProfile", "SPBonus", vec![f("fileName", "", "sound/ui/spbonus.ogg"), f("description", "", "AudioGui"), f("preload", "", "1")]),
        ]
    } else {
        vec![
            ("MusicData", "Lush", vec![f("numMoods", "", "1"), f("mood1", "0", "sound/music/g1.ogg"), f("mood1", "1", "sound/music/g2.ogg"), f("mood1", "2", "sound/music/g3.ogg"), f("sequences1", "0", "1 1 2 3 1 1 2 3"), f("sequences1", "1", "1 2 2 3 1 2 2 3"), f("k1", "", "0.33"), f("k2", "", "0.33"), f("k3", "", "0.33"), f("smooth1", "", "0.5"), f("smooth2", "", "0.5"), f("smooth3", "", "0.5"), f("moodSmooth", "", "0.5"), f("moodScale", "", "1")]),
            ("MusicData", "Spooky", vec![f("numMoods", "", "1"), f("mood1", "0", "sound/music/b1.ogg"), f("mood1", "1", "sound/music/b3.ogg"), f("sequences1", "0", "1 1 2 1 1 2"), f("sequences1", "1", "1 2 2 1 2 2"), f("k1", "", "0.33"), f("k2", "", "0.33"), f("k3", "", "0.33"), f("smooth1", "", "0.5"), f("smooth2", "", "0.5"), f("smooth3", "", "0.5"), f("moodSmooth", "", "0.5"), f("moodScale", "", "1")]),
            ("MusicData", "Frantic", vec![f("numMoods", "", "1"), f("mood1", "0", "sound/music/r1.ogg"), f("mood1", "1", "sound/music/r3.ogg"), f("sequences1", "0", "1 1 2"), f("sequences1", "1", "1 1 1"), f("k1", "", "0.33"), f("k2", "", "0.33"), f("k3", "", "0.33"), f("smooth1", "", "0.5"), f("smooth2", "", "0.5"), f("smooth3", "", "0.5"), f("moodSmooth", "", "0.5"), f("moodScale", "", "1")]),
        ]
    }
}

#[test]
fn client_files_hold_the_declarations() {
    tt_assets::require_game_data!();
    let root = tt_assets::layout::default_dir();
    for stem in [CLIENT_AUDIO, MUSIC] {
        let file = load_client_file(&root, stem).unwrap();
        let want = declarations(stem);
        assert_eq!(file.len(), want.len(), "{stem}");
        for (o, (c, n, fields)) in file.iter().zip(&want) {
            let (class, mut b) = tt_import::names::translate(c, fields).unwrap();
            assert!(o.class() == class && o.name() == *n && !o.is_def(), "{stem}: {} {}", o.class(), o.name());
            let mut a = o.to_fields();
            let key = |x: &Field| (x.name.to_ascii_lowercase(), x.index.clone());
            a.sort_by_key(key);
            b.sort_by_key(key);
            assert_eq!(a, b, "{stem}: {n}");
        }
    }
}
