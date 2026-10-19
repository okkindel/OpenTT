//! The crash report (`tt_engine::crash`): a panic on any thread and the
//! error that ends the game go to `<data dir>/logs/crash.log`. One test:
//! the panic hook is the process's.

use tt_engine::crash;

#[test]
fn panics_and_the_fatal_error_go_to_the_crash_log() {
    let data = std::env::temp_dir().join(format!("tt-crash-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    crash::install(&data);
    let log = data.join(crash::CRASH_LOG);
    assert_eq!(crash::log_path(), log);
    assert!(crash::first_panic().is_none());

    let r = std::thread::Builder::new().name("engine".into()).spawn(|| panic!("boom {}", 42)).unwrap().join();
    assert!(r.is_err());
    let first = crash::first_panic().expect("the panic is kept");
    assert!(first.starts_with("boom 42 at "), "{first}");
    assert!(first.contains("crash.rs"), "{first}");
    let text = std::fs::read_to_string(&log).expect("crash.log written");
    assert!(text.contains("thread 'engine' panicked at "), "{text}");
    assert!(text.contains("boom 42"), "{text}");
    assert!(text.contains("backtrace:"), "{text}");
    assert!(text.lines().next().unwrap().contains(env!("CARGO_PKG_VERSION")), "{text}");

    // A second panic is added; the first one stays the one reported.
    let _ = std::thread::spawn(|| panic!("second")).join();
    assert!(crash::first_panic().unwrap().starts_with("boom 42"));
    // The error that ends the game, without the message box.
    crash::fatal("the game crashed: boom 42", false);
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("boom 42") && text.contains("second"), "{text}");
    assert!(text.trim_end().ends_with("error: the game crashed: boom 42"), "{text}");
    assert_eq!(text.matches("command line:").count(), 1, "one header per run: {text}");
    let _ = std::fs::remove_dir_all(&data);
}
