//! Console log: stdout plus `<data dir>/logs/console.log` (rewritten every session).

use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Severity of a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

pub struct Logger {
    file: Option<File>,
    /// Print to stdout (off in quiet/headless report runs).
    pub echo: bool,
    /// The log mode (`-log`, the log-mode key: 0 off, 1 append+flush, 2
    /// overwrite; +4 dump). Recorded only; our log file is always written.
    pub mode: i32,
    /// Every line, kept for reports (bounded).
    pub history: Vec<(LogLevel, String)>,
    pub counts: [usize; 3],
}

const HISTORY: usize = 20_000;

impl Logger {
    pub fn new(data_dir: Option<&Path>, echo: bool) -> Logger {
        let file = data_dir.and_then(|d| {
            let path = d.join(tt_shell::save::CONSOLE_LOG);
            std::fs::create_dir_all(path.parent()?).ok()?;
            File::create(path).ok()
        });
        Logger { file, echo, mode: 0, history: Vec::new(), counts: [0; 3] }
    }

    pub fn log(&mut self, level: LogLevel, msg: &str) {
        let idx = match level {
            LogLevel::Info => 0,
            LogLevel::Warning => 1,
            LogLevel::Error => 2,
        };
        self.counts[idx] += 1;
        if self.echo {
            println!("{msg}");
        }
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{msg}");
        }
        if self.history.len() < HISTORY {
            self.history.push((level, msg.to_string()));
        }
    }

    pub fn info(&mut self, msg: &str) {
        self.log(LogLevel::Info, msg);
    }

    pub fn warn(&mut self, msg: &str) {
        self.log(LogLevel::Warning, msg);
    }

    pub fn error(&mut self, msg: &str) {
        self.log(LogLevel::Error, msg);
    }
}
