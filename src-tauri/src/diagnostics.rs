use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::SecondsFormat;
use log::{LevelFilter, Log, Metadata, Record, SetLoggerError};

const LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
const LOG_RETAINED_FILES: usize = 3;

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

pub fn init() -> Result<PathBuf, String> {
    let level = configured_level();
    let path = crate::config::get_app_log_path()?;

    if level == LevelFilter::Off {
        return Ok(path);
    }

    rotate_logs(&path, LOG_MAX_BYTES, LOG_RETAINED_FILES)?;

    let logger = FileLogger::open(path.clone(), level)?;
    let logger = LOGGER.get_or_init(|| logger);
    set_global_logger(logger, level).map_err(|e| format!("Failed to initialize logger: {e}"))?;

    logger.write_line(&format!(
        "{} INFO app_start version={} os={} log_path={} level={}",
        timestamp(),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        path.display(),
        level
    ));

    Ok(path)
}

fn set_global_logger(
    logger: &'static FileLogger,
    level: LevelFilter,
) -> Result<(), SetLoggerError> {
    log::set_logger(logger)?;
    log::set_max_level(level);
    Ok(())
}

pub(crate) fn configured_level() -> LevelFilter {
    match std::env::var("SM_LOG") {
        Ok(value) => parse_level(&value).unwrap_or(LevelFilter::Warn),
        Err(_) => LevelFilter::Warn,
    }
}

pub(crate) fn parse_level(value: &str) -> Option<LevelFilter> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "disabled" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" | "warning" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        "" => None,
        _ => None,
    }
}

pub(crate) fn rotate_logs(
    path: &Path,
    max_bytes: u64,
    retained_files: usize,
) -> Result<(), String> {
    if max_bytes == 0 || retained_files == 0 {
        return Ok(());
    }

    let Ok(metadata) = fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() < max_bytes {
        return Ok(());
    }

    for index in (1..=retained_files).rev() {
        let source = rotated_path(path, index);
        let target = rotated_path(path, index + 1);
        if source.exists() {
            if index == retained_files {
                fs::remove_file(&source).map_err(|e| {
                    format!("Failed to remove old log file {}: {e}", source.display())
                })?;
            } else {
                fs::rename(&source, &target).map_err(|e| {
                    format!(
                        "Failed to rotate log file {} to {}: {e}",
                        source.display(),
                        target.display()
                    )
                })?;
            }
        }
    }

    let first = rotated_path(path, 1);
    fs::rename(path, &first).map_err(|e| {
        format!(
            "Failed to rotate log file {} to {}: {e}",
            path.display(),
            first.display()
        )
    })?;

    Ok(())
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!("{index}.{ext}"))
        .unwrap_or_else(|| index.to_string());
    path.with_extension(extension)
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) struct FileLogger {
    file: Mutex<File>,
    level: LevelFilter,
}

impl FileLogger {
    pub(crate) fn open(path: PathBuf, level: LevelFilter) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create log directory {}: {e}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("Failed to open log file {}: {e}", path.display()))?;
        Ok(Self {
            file: Mutex::new(file),
            level,
        })
    }

    pub(crate) fn write_line(&self, line: &str) {
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        self.write_line(&format!(
            "{} {} {} - {}",
            timestamp(),
            record.level(),
            record.target(),
            record.args()
        ));
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TEST_ENV_LOCK;
    use tempfile::tempdir;

    static ENV_LOCK: &std::sync::Mutex<()> = &TEST_ENV_LOCK;

    #[test]
    fn parse_level_accepts_expected_values() {
        assert_eq!(parse_level("debug"), Some(LevelFilter::Debug));
        assert_eq!(parse_level("WARNING"), Some(LevelFilter::Warn));
        assert_eq!(parse_level("off"), Some(LevelFilter::Off));
        assert_eq!(parse_level("nope"), None);
    }

    #[test]
    fn configured_level_defaults_to_warn() {
        let _guard = ENV_LOCK.lock().expect("lock");
        std::env::remove_var("SM_LOG");
        assert_eq!(configured_level(), LevelFilter::Warn);
    }

    #[test]
    fn file_logger_writes_and_flushes_lines() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-manager.log");
        let logger = FileLogger::open(path.clone(), LevelFilter::Debug).expect("logger");

        logger.write_line("hello diagnostics");

        let content = std::fs::read_to_string(path).expect("read log");
        assert!(content.contains("hello diagnostics"));
    }

    #[test]
    fn rotate_logs_moves_current_file_and_retains_backups() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-manager.log");
        std::fs::write(&path, "current").expect("write current");
        std::fs::write(rotated_path(&path, 1), "old1").expect("write old1");
        std::fs::write(rotated_path(&path, 2), "old2").expect("write old2");

        rotate_logs(&path, 1, 2).expect("rotate");

        assert!(!path.exists());
        assert_eq!(
            std::fs::read_to_string(rotated_path(&path, 1)).expect("read .1"),
            "current"
        );
        assert_eq!(
            std::fs::read_to_string(rotated_path(&path, 2)).expect("read .2"),
            "old1"
        );
        assert!(!rotated_path(&path, 3).exists());
    }
}
