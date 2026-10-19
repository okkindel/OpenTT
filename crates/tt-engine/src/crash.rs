//! Fatal errors made visible. A release build on Windows has no console
//! window (main.rs), so a failure there would otherwise vanish: every panic
//! is written to `<data dir>/logs/crash.log` ([`install`]), the error that
//! ends the game is added to it and shown in a message box ([`fatal`]),
//! and a run started from a console writes its output there
//! ([`attach_console`]).

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The crash report, relative to the user data directory (next to
/// `logs/console.log`; rewritten by the first report of a run).
pub const CRASH_LOG: &str = "logs/crash.log";

/// Disables the message box of [`fatal`] when set (unattended runs).
pub const NO_DIALOG_ENV: &str = "TT_NO_ERROR_DIALOG";

/// The user data directory the report goes to.
static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
/// The first panic of the run: message and place.
static FIRST_PANIC: Mutex<Option<String>> = Mutex::new(None);
/// Whether this run wrote the report already (later reports append).
static WRITTEN: AtomicBool = AtomicBool::new(false);

/// The user data directory set by [`install`], if it was called.
pub fn data_dir() -> Option<PathBuf> {
    DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Sets the user data directory of the report and installs the panic hook
/// (once; later calls only change the directory). The hook writes the
/// panic's thread, message, place and backtrace to [`CRASH_LOG`], keeps the
/// first panic for [`first_panic`], then runs the default hook (stderr).
pub fn install(data_dir: &Path) {
    let first = {
        let mut d = DIR.lock().unwrap_or_else(|e| e.into_inner());
        let first = d.is_none();
        *d = Some(data_dir.to_path_buf());
        first
    };
    if !first {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "(no message)".to_string()
        };
        let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        {
            let mut f = FIRST_PANIC.lock().unwrap_or_else(|e| e.into_inner());
            if f.is_none() {
                *f = Some(format!("{msg}{at}"));
            }
        }
        let mut text = format!("thread '{thread}' panicked{at}:\n{msg}\n");
        let _ = write!(text, "\nbacktrace:\n{}\n", std::backtrace::Backtrace::force_capture());
        let _ = write_report(&text);
        previous(info);
    }));
}

/// The first panic of this run (message and place), if any.
pub fn first_panic() -> Option<String> {
    FIRST_PANIC.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Where the report goes ([`install`]'s directory, else the default user
/// data directory).
pub fn log_path() -> PathBuf {
    let dir = DIR.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(tt_assets::layout::user_data_dir);
    dir.join(CRASH_LOG)
}

/// Adds `text` to the report (a header with the version and the time
/// first); the first report of a run replaces the previous run's.
pub fn write_report(text: &str) -> std::io::Result<PathBuf> {
    let path = log_path();
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let append = WRITTEN.swap(true, Ordering::SeqCst);
    let mut f = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&path)?;
    if !append {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        writeln!(f, "{} {} ({} {}), unix time {secs}", tt_assets::product::NAME, env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH)?;
        writeln!(f, "command line: {:?}\n", std::env::args().collect::<Vec<_>>())?;
    }
    writeln!(f, "{text}")?;
    Ok(path)
}

/// The error `msg` that ends the game: printed to stderr, added to the
/// report and, with `dialog` (a windowed run) and without
/// [`NO_DIALOG_ENV`], shown in a message box with the report's path.
/// Call it on the main thread (macOS shows dialogs only there).
pub fn fatal(msg: &str, dialog: bool) {
    eprintln!("error: {msg}");
    let logged = write_report(&format!("error: {msg}"));
    if !dialog || std::env::var_os(NO_DIALOG_ENV).is_some_and(|v| !v.is_empty()) {
        return;
    }
    let mut text = format!("{} stopped because of an error:\n\n{msg}\n\n", tt_assets::product::NAME);
    match &logged {
        Ok(p) => {
            let _ = write!(text, "The details are in {} (the game's log is console.log in the same folder).", p.display());
        }
        Err(e) => {
            let _ = write!(text, "(The report could not be written to {}: {e}.)", log_path().display());
        }
    }
    rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title(tt_assets::product::NAME).set_description(text).set_buttons(rfd::MessageButtons::Ok).show();
}

/// Gives the output a console (Windows): a release build is a GUI program
/// without a console window of its own (main.rs), so it attaches to the
/// console of the program that started it (cmd, PowerShell) and its output
/// appears there; without one, with `alloc` (a server's console), it opens
/// a console window of its own. Returns whether there is a console. Other
/// systems: nothing to do (true).
#[cfg(windows)]
pub fn attach_console(alloc: bool) -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleWindow() -> *mut core::ffi::c_void;
        fn AttachConsole(process: u32) -> i32;
        fn AllocConsole() -> i32;
    }
    /// `ATTACH_PARENT_PROCESS`: `(DWORD)-1`.
    const PARENT: u32 = u32::MAX;
    // SAFETY: plain Win32 calls without pointers.
    unsafe { !GetConsoleWindow().is_null() || AttachConsole(PARENT) != 0 || (alloc && AllocConsole() != 0) }
}

/// See the Windows version.
#[cfg(not(windows))]
pub fn attach_console(_alloc: bool) -> bool {
    true
}
