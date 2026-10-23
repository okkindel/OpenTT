//! `screenShot` (Ctrl+P → `doScreenShot`): the request, the PNG in the data
//! dir's `screenshots/` and the numbering that never overwrites an earlier session's shots.

use tt_engine::{Config, Engine};

mod common;
use common::*;

#[test]
fn ctrl_p_saves_numbered_pngs_in_the_data_dir() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let data = std::env::temp_dir().join(format!("tt-engine-test-screenshot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let frame = |w: u32, h: u32| -> Vec<u8> { (0..w * h).flat_map(|i| [(i % 251) as u8, 40, 200, 255]).collect() };
        for session in 0..2 {
            let cfg = Config { assets: tt_assets::layout::default_dir(), data: data.clone(), args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
            let mut e = Engine::new(&cfg).unwrap();
            assert!(e.boot());
            // The ported default.bind binds Ctrl+P to doScreenShot.
            e.key("lcontrol", true, None);
            e.key("p", true, None);
            e.key("p", false, None);
            e.key("lcontrol", false, None);
            assert_eq!(e.host.screenshot_request.as_deref(), Some("screenshots/screenshot_00001.png"), "session {session}");
            let path = e.save_screenshot(&frame(8, 6), (8, 6)).expect("saved");
            assert!(path.starts_with(data.join("screenshots")), "{}", path.display());
            let expect = if session == 0 { "screenshot_00001.png" } else { "screenshot_00002.png" };
            assert_eq!(path.file_name().unwrap().to_str().unwrap(), expect);
            let img = image::open(&path).unwrap().to_rgb8();
            assert_eq!(img.dimensions(), (8, 6));
            assert_eq!(img.get_pixel(1, 0).0, [1, 40, 200]);
            assert!(e.host.log.history.iter().any(|(_, m)| m.starts_with("Screenshot saved: ")));
            assert!(e.host.screenshot_request.is_none());
        }
        let _ = std::fs::remove_dir_all(&data);
    });
}

/// Alt+Tab away and back: Alt's release went to another window. While it is
/// stuck no plain binding matches (Ctrl+P reads as Ctrl+Alt+P); the focus
/// loss releases everything and Ctrl+P works again.
#[test]
fn focus_loss_releases_a_stuck_alt() {
    tt_assets::require_game_data!();
    big_stack(|| {
        let data = std::env::temp_dir().join(format!("tt-engine-test-focus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let cfg = Config { assets: tt_assets::layout::default_dir(), data: data.clone(), args: Vec::new(), audio_output: false, echo_log: false, windowed: true };
        let mut e = Engine::new(&cfg).unwrap();
        assert!(e.boot());
        let ctrl_p = |e: &mut Engine| {
            e.key("lcontrol", true, None);
            e.key("p", true, None);
            e.key("p", false, None);
            e.key("lcontrol", false, None);
        };
        e.key("lalt", true, None);
        ctrl_p(&mut e);
        assert!(e.host.screenshot_request.is_none());
        e.release_input();
        ctrl_p(&mut e);
        assert!(e.host.screenshot_request.is_some());
        let _ = std::fs::remove_dir_all(&data);
    });
}
