//! The game binary's startup (src/main.rs): on macOS the game runs on the
//! main thread (the window's event loop must be created there) with the
//! stack linked into the binary (build.rs); elsewhere on its own thread
//! "engine".

mod common;

/// Which thread runs the game: a run that ends at once (no game data) says
/// so with `TT_PRINT_ENGINE_THREAD`.
#[test]
fn the_game_runs_on_the_main_thread_on_macos_only() {
    let base = std::env::temp_dir().join(format!("tt-startup-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let nothing = base.join("nothing");
    std::fs::create_dir_all(&nothing).unwrap();
    let out = std::process::Command::new(common::game_exe())
        .env("TT_PRINT_ENGINE_THREAD", "1")
        .env("TT_NO_ERROR_DIALOG", "1")
        .args(["--assets"])
        .arg(&nothing)
        .args(["--data"])
        .arg(base.join("data"))
        .args(["--dedicated", "--no-sound", "--quiet"])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&base);
    let err = String::from_utf8_lossy(&out.stderr);
    let want = if cfg!(target_os = "macos") { "engine thread: main" } else { "engine thread: engine" };
    assert!(err.contains(want), "expected {want:?}:\n{err}");
}

/// The macOS binary's main thread has the 32 MB stack of build.rs (the
/// `LC_MAIN` load command's `stacksize`, read with `otool -l`).
#[cfg(target_os = "macos")]
#[test]
fn the_macos_binary_links_a_big_main_thread_stack() {
    let out = std::process::Command::new("otool").arg("-l").arg(common::game_exe()).output().expect("otool");
    let text = String::from_utf8_lossy(&out.stdout);
    let size = text.lines().map(str::trim).find_map(|l| l.strip_prefix("stacksize ")).and_then(|v| v.trim().parse::<u64>().ok());
    assert_eq!(size, Some(32 << 20), "LC_MAIN stacksize of {}", common::game_exe());
}
