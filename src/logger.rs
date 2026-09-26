//! Tiny file logger (no console on Windows). Set `SPOTILITE_LOG=debug` for details.
//!
//! The last warnings of the playback library are also kept in memory, so the
//! interface can say *why* a track could not be played.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Recent librespot warnings and errors, newest last.
static RECENT: Mutex<VecDeque<(Instant, String)>> = Mutex::new(VecDeque::new());
const RECENT_MAX: usize = 32;

struct FileLogger {
    file: Option<Mutex<File>>,
    level: LevelFilter,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let own = metadata.target().starts_with("spotilite");
        // Third-party crates are only interesting when something goes wrong.
        metadata.level() <= if own { self.level } else { self.level.min(LevelFilter::Warn) }
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if record.target().starts_with("librespot") && record.level() <= Level::Warn {
            remember(record.args().to_string());
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = format!("{secs} {:<5} {}: {}\n", record.level(), record.target(), record.args());
        if let Some(Ok(mut file)) = self.file.as_ref().map(Mutex::lock) {
            let _ = file.write_all(line.as_bytes());
        }
        if cfg!(debug_assertions) && record.level() <= Level::Warn {
            eprint!("{line}");
        }
    }

    fn flush(&self) {
        if let Some(Ok(mut file)) = self.file.as_ref().map(Mutex::lock) {
            let _ = file.flush();
        }
    }
}

fn remember(message: String) {
    if let Ok(mut recent) = RECENT.lock() {
        recent.push_back((Instant::now(), message));
        while recent.len() > RECENT_MAX {
            recent.pop_front();
        }
    }
}

/// librespot warnings and errors logged since `since`.
pub fn playback_messages_since(since: Instant) -> Vec<String> {
    RECENT
        .lock()
        .map(|recent| recent.iter().filter(|(at, _)| *at >= since).map(|(_, m)| m.clone()).collect())
        .unwrap_or_default()
}

pub fn init(path: &Path) {
    let level = std::env::var("SPOTILITE_LOG").ok().and_then(|v| v.parse().ok()).unwrap_or(LevelFilter::Info);
    // Keep the log small: start over once it exceeds 512 KB.
    let too_big = std::fs::metadata(path).map(|m| m.len() > 512 * 1024).unwrap_or(false);
    let file = OpenOptions::new().create(true).append(!too_big).write(true).truncate(too_big).open(path).ok();
    // Installed even without a log file: playback diagnostics rely on it.
    let logger = FileLogger { file: file.map(Mutex::new), level };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level.max(LevelFilter::Warn));
    }
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
    }));
}

#[cfg(test)]
pub fn remember_for_test(message: &str) {
    remember(message.to_string());
}
