//! Opt-in timings of the mission load path (`TT_LOAD_STATS=1`): each
//! [`span`] prints `[load] name: N ms` on stderr when it ends.

use std::sync::OnceLock;
use std::time::Instant;

/// `TT_LOAD_STATS` is set (and not `0`).
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TT_LOAD_STATS").is_some_and(|v| v != "0"))
}

/// A timed stage; reports when dropped (nothing if disabled).
pub struct Span {
    name: Option<String>,
    start: Instant,
}

pub fn span(name: impl FnOnce() -> String) -> Span {
    Span { name: enabled().then(name), start: Instant::now() }
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some(n) = &self.name {
            eprintln!("[load] {n}: {:.1} ms", self.start.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

/// Prints a line when enabled.
pub fn note(text: impl FnOnce() -> String) {
    if enabled() {
        eprintln!("[load] {}", text());
    }
}

static START: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// Starts the wall clock of a load (e.g. the menu's start action).
pub fn start(what: &str) {
    if enabled() {
        *START.lock().unwrap() = Some(Instant::now());
        eprintln!("[load] ---- {what} ----");
    }
}

/// Wall time since [`start`] at a milestone; `last` ends the load.
pub fn milestone(what: &str, last: bool) {
    if !enabled() {
        return;
    }
    let mut s = START.lock().unwrap();
    if let Some(t) = *s {
        eprintln!("[load] @{:.0} ms: {what}", t.elapsed().as_secs_f64() * 1000.0);
        if last {
            *s = None;
        }
    }
}
