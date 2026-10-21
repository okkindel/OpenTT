//! The game data (`docs/import.md` § The game's import): whether the
//! asset directory has it, and the import flow behind the first-run
//! screen and Settings → Game data ([`SetupFlow`]: the folder check, the
//! system's folder dialog, the import on a background thread, its
//! progress and cancel, its report: on the screen and in
//! `<user data>/logs/import.log`, [`IMPORT_LOG`]). The screen itself is
//! `tt_menu::SetupState`; the window around it is [`crate::launcher`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tt_import::{InstallError, Progress, Report};
use tt_menu::{SetupAction, SetupMode, SetupProgress, SetupReport, SetupState};

/// The import's report, relative to the user data directory (rewritten by
/// every import; the next session's `logs/console.log` repeats it).
pub const IMPORT_LOG: &str = "logs/import.log";

/// Opens `dir` in the system's file manager (best effort).
pub fn open_folder(dir: &Path) -> Result<(), String> {
    let opener = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    match std::process::Command::new(opener).arg(dir).spawn() {
        Ok(mut child) => {
            // Reaped when the opener exits.
            std::thread::spawn(move || child.wait());
            Ok(())
        }
        Err(e) => Err(format!("cannot open {} with {opener}: {e}", dir.display())),
    }
}

/// The text of an import log: where from and to, then `lines`.
pub fn log_text(source: &Path, target: &Path, ok: bool, lines: &[String]) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut t = format!("{} import of the ThinkTanks game data, {}\n", tt_assets::product::NAME, utc_stamp(now));
    t.push_str(&format!("from: {}\ninto: {}\nresult: {}\n\n", source.display(), target.display(), if ok { "imported" } else { "failed" }));
    for l in lines {
        t.push_str(l);
        t.push('\n');
    }
    t
}

/// Whether `assets` is an imported asset directory (one the game's import
/// or `tt-import` made) of another version of the program: its stamp
/// ([`tt_import::OURS_VERSION`]) differs, or an imported `gamedata/` has
/// none. The game then imports it again (`docs/import.md`).
pub fn stale(assets: &Path) -> bool {
    stale_in(assets, &tt_assets::layout::imported_candidates())
}

fn stale_in(assets: &Path, imported: &[PathBuf]) -> bool {
    let same = |d: &PathBuf| d.canonicalize().ok().is_some_and(|d| assets.canonicalize().is_ok_and(|a| a == d));
    match std::fs::read_to_string(assets.join(tt_import::OURS_VERSION)) {
        Ok(v) => v.trim() != env!("CARGO_PKG_VERSION"),
        Err(_) => imported.iter().any(same),
    }
}

/// Brings our own files in a [`stale`] asset directory to this version
/// ([`tt_import::refresh_ours`]; the game's files are not needed): the
/// fallback until it is imported again.
pub fn refresh_ours(assets: &Path) {
    match tt_import::refresh_ours(assets) {
        Ok(true) => eprintln!("game data: our own files in {} updated to {}", assets.display(), env!("CARGO_PKG_VERSION")),
        Ok(false) => {}
        Err(e) => eprintln!("game data: our own files in {} not updated: {e}", assets.display()),
    }
}

/// Settings keys of the imported data's origin (`docs/layout.md` § Settings).
pub const SOURCE_KEY: &str = "game_data.source";
pub const IMPORTED_KEY: &str = "game_data.imported";

/// Whether the asset directory `assets` holds the game data (the marker
/// files of [`tt_assets::testdata::MARKERS`]: a mission, the tank
/// defs, a tank shape; our own files alone are not enough).
pub fn present(assets: &Path) -> bool {
    tt_assets::testdata::present_in(assets)
}

/// Where the game's import writes for the asset directory `assets`:
/// `assets` itself when it was given (`--assets`, `$TT_ASSETS`) or is an
/// imported `gamedata/` directory, else [`tt_assets::layout::import_target`].
pub fn target_for(assets: &Path, explicit: bool) -> PathBuf {
    let imported = assets.file_name().is_some_and(|n| n == tt_assets::layout::IMPORTED);
    if explicit || imported {
        assets.to_path_buf()
    } else {
        tt_assets::layout::import_target()
    }
}

/// The mission pictures rendered by the game's renderer
/// ([`crate::mission_picture`], an offscreen GPU device per picture).
pub fn picture_renderer() -> tt_import::PictureFn {
    Arc::new(|root: &Path, job: &tt_import::PictureJob| {
        let run = || -> Result<(), String> {
            let mut p = crate::mission_picture::Pictures::new(root, &job.mission, 1.0, true, 2)?;
            p.render(&root.join(&job.asset), job.camera, tt_import::PICTURE_SIZE)
        };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err("the renderer failed".into()))
    })
}

/// The import options the screen uses: the install `source` into `target`
/// (all or nothing, [`tt_import::install`]), normal maps on, the mission
/// pictures rendered when `pictures`.
pub fn options(source: &Path, target: &Path, pictures: bool) -> tt_import::Options {
    let mut o = tt_import::Options::new(source, target);
    if pictures {
        o.pictures = Some(picture_renderer());
    }
    o
}

/// An import running on a background thread.
pub struct ImportJob {
    pub source: PathBuf,
    pub target: PathBuf,
    progress: Arc<Mutex<Option<Progress>>>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<Report, InstallError>>>,
}

impl ImportJob {
    /// Starts importing with `opts` (its progress and cancel are set here).
    pub fn start(mut opts: tt_import::Options) -> ImportJob {
        let progress = Arc::new(Mutex::new(None));
        let cancel = Arc::new(AtomicBool::new(false));
        let p = progress.clone();
        opts.progress = Some(Arc::new(move |x: &Progress| {
            if let Ok(mut g) = p.lock() {
                *g = Some(x.clone());
            }
        }));
        opts.cancel = Some(cancel.clone());
        let (source, target) = (opts.game.clone(), opts.out.clone());
        // A big stack: the script reader recurses.
        let handle = std::thread::Builder::new()
            .name("import".into())
            .stack_size(64 << 20)
            .spawn(move || tt_import::install(&opts))
            .map_err(|e| e.to_string());
        match handle {
            Ok(h) => ImportJob { source, target, progress, cancel, handle: Some(h) },
            Err(e) => {
                let failed = std::thread::spawn(move || Err(InstallError::Failed(format!("cannot start the import: {e}"))));
                ImportJob { source, target, progress, cancel, handle: Some(failed) }
            }
        }
    }

    /// The last progress report.
    pub fn progress(&self) -> Option<Progress> {
        self.progress.lock().ok().and_then(|g| g.clone())
    }

    /// Asks the import to stop (after its current file).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn finished(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }

    /// Waits for the import and returns its result.
    pub fn join(mut self) -> Result<Report, InstallError> {
        match self.handle.take() {
            Some(h) => h.join().unwrap_or_else(|_| Err(InstallError::Failed("the import stopped unexpectedly".into()))),
            None => Err(InstallError::Failed("no import".into())),
        }
    }
}

/// Whether a folder dialog can be shown here (Linux: the desktop portal
/// on the session bus, or zenity); without one the path is typed.
pub fn dialog_available() -> bool {
    if cfg!(target_os = "linux") || cfg!(target_os = "freebsd") {
        let bus = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|v| !v.is_empty());
        let zenity = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("zenity").is_file()));
        bus || zenity
    } else {
        true
    }
}

/// Opens the system's folder dialog on a thread (it blocks); the result
/// arrives on the returned channel (`None`: closed without a choice).
fn browse(start: &str) -> Receiver<Option<PathBuf>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let start = PathBuf::from(start);
    std::thread::spawn(move || {
        let mut d = rfd::FileDialog::new().set_title("Locate your ThinkTanks installation");
        if start.is_dir() {
            d = d.set_directory(&start);
        } else if let Some(h) = std::env::var_os("HOME") {
            d = d.set_directory(h);
        }
        let _ = tx.send(d.pick_folder());
    });
    rx
}

/// How the screen ended.
#[derive(Debug)]
pub enum Outcome {
    /// The import succeeded: `assets` holds the game data from `source`.
    Imported { assets: PathBuf, source: PathBuf, report: Report },
    /// Back to the game without importing (re-import only).
    Back,
    Quit,
}

/// The first-run / re-import screen's logic: the player's actions
/// ([`SetupFlow::handle`]) and the background work ([`SetupFlow::poll`]).
pub struct SetupFlow {
    /// What the screen shows.
    pub ui: SetupState,
    /// Where the import goes.
    pub target: PathBuf,
    /// Render the mission pictures (the GPU; off in tests).
    pub pictures: bool,
    job: Option<ImportJob>,
    dialog: Option<Receiver<Option<PathBuf>>>,
    /// The import's result while its report is shown (Continue takes it).
    finished: Option<Outcome>,
    /// Where the import log goes (`<user data>/logs`); `None`: not written.
    pub log_dir: Option<PathBuf>,
    /// Set when the screen is done.
    pub outcome: Option<Outcome>,
}

impl SetupFlow {
    /// The screen in `mode`, importing into `target`; `source`: the folder
    /// of the last import (prefilled).
    pub fn new(mode: SetupMode, target: PathBuf, source: Option<String>) -> SetupFlow {
        let mut ui = SetupState::new(mode);
        ui.target = target.display().to_string();
        ui.path = source.unwrap_or_default();
        ui.can_browse = dialog_available();
        // The game's data directory (set at start-up), else the platform's.
        let log_dir = Some(crate::crash::data_dir().unwrap_or_else(tt_assets::layout::user_data_dir).join("logs"));
        SetupFlow { ui, target, pictures: true, job: None, dialog: None, finished: None, log_dir, outcome: None }
    }

    /// An import is running.
    pub fn importing(&self) -> bool {
        self.job.is_some()
    }

    /// Writes the import log (and a copy to stderr); returns its path,
    /// `None` when there is no log folder or it failed.
    fn write_log(&self, source: &Path, ok: bool, lines: &[String]) -> Option<PathBuf> {
        let text = log_text(source, &self.target, ok, lines);
        for l in text.lines() {
            eprintln!("import: {l}");
        }
        let dir = self.log_dir.as_ref()?;
        std::fs::create_dir_all(dir).ok()?;
        let path = dir.join(Path::new(IMPORT_LOG).file_name()?);
        std::fs::write(&path, &text).ok()?;
        Some(path)
    }

    /// Shows the report of a failed import or check: `message` in red on
    /// the folder screen (with the log's place), the details in the report.
    fn failed(&mut self, source: &Path, message: String, lines: Vec<String>) {
        let mut all = vec![message.clone()];
        all.extend(lines.iter().cloned());
        let log = self.write_log(source, false, &all);
        let at = log.as_ref().map(|p| format!(" The full report is in {}.", p.display())).unwrap_or_default();
        self.ui.error = Some(format!("{message}{at}"));
        self.ui.report = Some(SetupReport { ok: false, summary: message, lines, log: log.map(|p| p.display().to_string()).unwrap_or_default() });
    }

    /// Handles what the player asked for.
    pub fn handle(&mut self, action: SetupAction) {
        match action {
            SetupAction::Browse => {
                if self.dialog.is_none() && self.job.is_none() {
                    self.dialog = Some(browse(self.ui.path.trim()));
                    self.ui.browsing = true;
                }
            }
            SetupAction::Import(path) => {
                if self.job.is_some() {
                    return;
                }
                self.ui.note = None;
                self.ui.report = None;
                let root = match tt_import::locate::find_root(Path::new(&path)) {
                    Ok(root) => root,
                    Err(e) => {
                        self.ui.error = Some(e);
                        return;
                    }
                };
                self.ui.path = root.display().to_string();
                let missing = tt_import::locate::missing_required(&root, &tt_import::mapping::Mapping::builtin());
                if !missing.is_empty() {
                    let mut lines = vec![format!("Missing from the install ({}):", missing.len())];
                    lines.extend(missing.iter().map(|m| format!("  {m}")));
                    self.failed(&root, tt_import::locate::incomplete(&root, &missing), lines);
                    return;
                }
                self.ui.error = None;
                self.ui.progress = Some(SetupProgress { phase: tt_import::Phase::Starting.label().into(), ..Default::default() });
                self.ui.cancelling = false;
                self.job = Some(ImportJob::start(options(&root, &self.target, self.pictures)));
            }
            SetupAction::OpenLogFolder => {
                if let Some(dir) = &self.log_dir {
                    if let Err(e) = open_folder(dir) {
                        eprintln!("import: {e}");
                    }
                }
            }
            SetupAction::Continue => {
                self.ui.report = None;
                if let Some(o) = self.finished.take() {
                    self.outcome = Some(o);
                }
            }
            SetupAction::Cancel => {
                if let Some(j) = &self.job {
                    j.cancel();
                    self.ui.cancelling = true;
                }
            }
            SetupAction::Back => {
                if self.job.is_none() && self.ui.mode == SetupMode::Reimport {
                    self.outcome = Some(Outcome::Back);
                }
            }
            SetupAction::Quit => {
                if let Some(j) = self.job.take() {
                    j.cancel();
                    let _ = j.join();
                }
                self.outcome = Some(Outcome::Quit);
            }
        }
    }

    /// Takes in the folder dialog's answer and the import's progress; sets
    /// [`SetupFlow::outcome`] when the import succeeded.
    pub fn poll(&mut self) {
        if let Some(rx) = &self.dialog {
            match rx.try_recv() {
                Ok(choice) => {
                    self.dialog = None;
                    self.ui.browsing = false;
                    if let Some(dir) = choice {
                        self.ui.error = None;
                        self.ui.path = dir.display().to_string();
                        // A folder that is not an install is reported now.
                        if let Err(e) = tt_import::locate::find_root(&dir) {
                            self.ui.error = Some(e);
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.dialog = None;
                    self.ui.browsing = false;
                    self.ui.can_browse = false;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let Some(job) = &self.job else { return };
        if let Some(p) = job.progress() {
            self.ui.progress = Some(SetupProgress { phase: p.phase.label().into(), done: p.done, total: p.total, file: p.file });
        }
        if !job.finished() {
            return;
        }
        let job = self.job.take().unwrap();
        let source = job.source.clone();
        let result = job.join();
        self.ui.progress = None;
        self.ui.cancelling = false;
        match result {
            Ok(report) => {
                let lines = report.lines();
                let log = self.write_log(&source, true, &lines);
                self.ui.report = Some(SetupReport { ok: true, summary: lines[0].clone(), lines: lines[1..].to_vec(), log: log.map(|p| p.display().to_string()).unwrap_or_default() });
                self.finished = Some(Outcome::Imported { assets: self.target.clone(), source, report });
            }
            Err(InstallError::Cancelled) => self.ui.note = Some("Import cancelled; nothing was changed.".into()),
            Err(InstallError::Failed(e)) => {
                let lines: Vec<&str> = e.lines().collect();
                let more = if lines.len() > 2 { format!(" (and {} more problems)", lines.len() - 2) } else { String::new() };
                let message = format!("The import failed: {}{more}.", lines.iter().take(2).copied().collect::<Vec<_>>().join("; "));
                let mut details = vec![format!("Problems ({}):", lines.len())];
                details.extend(lines.iter().map(|l| format!("  {l}")));
                self.failed(&source, message, details);
            }
        }
    }

    /// The game was updated since the import ([`stale`]): imports again
    /// from the recorded folder when it is there, else asks for it; a note
    /// says why.
    pub fn refresh(&mut self) {
        let path = self.ui.path.trim().to_string();
        if Path::new(&path).is_dir() {
            self.handle(SetupAction::Import(path));
        }
        let v = env!("CARGO_PKG_VERSION");
        self.ui.note = Some(if self.importing() {
            format!("Updated to {v}: refreshing the game data from your ThinkTanks folder.")
        } else {
            format!("Updated to {v}: import the game data again from your ThinkTanks folder (Back keeps the current data).")
        });
    }

    /// Runs the import to its end (no window): `Ok` with the report, or
    /// the message the screen would show.
    pub fn run_blocking(&mut self, path: &str, mut on_progress: impl FnMut(&SetupProgress)) -> Result<Report, String> {
        self.handle(SetupAction::Import(path.to_string()));
        while self.job.is_some() {
            std::thread::sleep(std::time::Duration::from_millis(100));
            self.poll();
            if let Some(p) = &self.ui.progress {
                on_progress(p);
            }
        }
        if self.finished.is_some() {
            self.handle(SetupAction::Continue);
        }
        match self.outcome.take() {
            Some(Outcome::Imported { report, .. }) => Ok(report),
            _ => Err(self.ui.error.clone().or_else(|| self.ui.note.clone()).unwrap_or_else(|| "the import did not run".into())),
        }
    }
}

/// `YYYY-MM-DD HH:MM UTC` of a Unix time.
pub fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // Days since 1970-01-01 to a civil date (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", rem / 3600, rem % 3600 / 60)
}

/// Records where the game data came from (`game_data.*`) and saves the settings.
pub fn record_source(h: &mut crate::host::EngineHost, source: &Path) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    crate::shell::set_pref(h, SOURCE_KEY, &source.display().to_string());
    crate::shell::set_pref(h, IMPORTED_KEY, &utc_stamp(now));
    crate::shell::save_prefs(h);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps() {
        assert_eq!(utc_stamp(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc_stamp(951_782_400 + 3600 * 13 + 60 * 7), "2000-02-29 13:07 UTC");
        assert_eq!(utc_stamp(1_792_281_600), "2026-10-18 00:00 UTC");
    }

    #[test]
    fn targets() {
        let g = Path::new("/x/gamedata");
        assert_eq!(target_for(g, false), g);
        assert_eq!(target_for(Path::new("/x/mydata"), true), Path::new("/x/mydata"));
        assert_eq!(target_for(Path::new("/x/assets"), false), tt_assets::layout::import_target());
    }

    #[test]
    fn stale_data() {
        let d = std::env::temp_dir().join(format!("tt-stale-{}", std::process::id()));
        let g = d.join(tt_assets::layout::IMPORTED);
        std::fs::create_dir_all(&g).unwrap();
        let imported = [g.clone()];
        assert!(stale_in(&g, &imported), "an imported directory without a stamp");
        assert!(!stale_in(&g, &[]), "no stamp and not imported (the repository's assets/)");
        std::fs::write(g.join(tt_import::OURS_VERSION), "0.1.2").unwrap();
        assert!(stale_in(&g, &imported) && stale_in(&g, &[]), "another version's");
        std::fs::write(g.join(tt_import::OURS_VERSION), format!("{}\n", env!("CARGO_PKG_VERSION"))).unwrap();
        assert!(!stale_in(&g, &imported), "this version's");
        let _ = std::fs::remove_dir_all(&d);
        assert!(!stale(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")), "the repository's assets/");
        // No recorded folder (or a gone one): the screen asks for it.
        let mut flow = SetupFlow::new(SetupMode::Reimport, d.clone(), Some("/nonexistent/ThinkTanks".into()));
        flow.refresh();
        assert!(!flow.importing() && flow.ui.error.is_none());
        assert!(flow.ui.note.as_deref().is_some_and(|n| n.starts_with(&format!("Updated to {}", env!("CARGO_PKG_VERSION")))));
    }
}
