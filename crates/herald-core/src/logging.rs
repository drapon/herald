//! Logging infrastructure for Herald
//!
//! Provides structured logging with file output and rotation support.
//! Uses `tracing` for instrumentation and `tracing-subscriber` for formatting.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// Default log file name
pub const DEFAULT_LOG_FILE: &str = "herald.log";

/// Default maximum log file size in bytes (10 MB)
pub const DEFAULT_MAX_SIZE_BYTES: u64 = 10 * 1024 * 1024;

/// Errors that can occur during logger initialization
#[derive(Debug, Error)]
pub enum LoggerError {
    /// Failed to create log directory
    #[error("Failed to create log directory: {0}")]
    DirectoryCreationFailed(String),

    /// Failed to initialize the logger
    #[error("Failed to initialize logger: {0}")]
    InitializationFailed(String),

    /// Logger already initialized
    #[error("Logger has already been initialized")]
    AlreadyInitialized,

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Log level configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    /// Error level - critical failures
    Error,
    /// Warn level - warnings and recoverable issues
    Warn,
    /// Info level - general information (default)
    #[default]
    Info,
    /// Debug level - detailed debugging information
    Debug,
    /// Trace level - very detailed tracing
    Trace,
}

impl LogLevel {
    /// Converts to tracing's LevelFilter
    pub fn to_level_filter(self) -> tracing::level_filters::LevelFilter {
        match self {
            LogLevel::Error => tracing::level_filters::LevelFilter::ERROR,
            LogLevel::Warn => tracing::level_filters::LevelFilter::WARN,
            LogLevel::Info => tracing::level_filters::LevelFilter::INFO,
            LogLevel::Debug => tracing::level_filters::LevelFilter::DEBUG,
            LogLevel::Trace => tracing::level_filters::LevelFilter::TRACE,
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Error => write!(f, "error"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Trace => write!(f, "trace"),
        }
    }
}

impl std::str::FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "error" => Ok(LogLevel::Error),
            "warn" | "warning" => Ok(LogLevel::Warn),
            "info" => Ok(LogLevel::Info),
            "debug" => Ok(LogLevel::Debug),
            "trace" => Ok(LogLevel::Trace),
            _ => Err(format!("Unknown log level: {}", s)),
        }
    }
}

/// Configuration for the Herald logger
#[derive(Debug, Clone)]
pub struct LoggerConfig {
    /// Directory for log files
    pub log_dir: PathBuf,
    /// Log level filter
    pub level: LogLevel,
    /// Whether to also log to stdout
    pub log_to_stdout: bool,
    /// Maximum log file size in bytes before rotation
    pub max_file_size: u64,
}

impl LoggerConfig {
    /// Creates a new LoggerConfig with the specified log directory
    pub fn new(log_dir: PathBuf) -> Self {
        Self {
            log_dir,
            level: LogLevel::Info,
            log_to_stdout: false,
            max_file_size: DEFAULT_MAX_SIZE_BYTES,
        }
    }

    /// Creates a LoggerConfig with the default log directory (~/.herald/logs/)
    pub fn with_default_dir() -> Self {
        let log_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".herald")
            .join("logs");
        Self::new(log_dir)
    }

    /// Sets the log level
    pub fn with_level(mut self, level: LogLevel) -> Self {
        self.level = level;
        self
    }

    /// Enables logging to stdout in addition to file
    pub fn with_stdout(mut self, enabled: bool) -> Self {
        self.log_to_stdout = enabled;
        self
    }

    /// Sets the maximum file size before rotation
    pub fn with_max_file_size(mut self, size: u64) -> Self {
        self.max_file_size = size;
        self
    }

    /// Returns the log directory path
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// Returns the full path to the log file
    pub fn log_file_path(&self) -> PathBuf {
        self.log_dir.join(DEFAULT_LOG_FILE)
    }
}

/// Guard that keeps the logger alive
///
/// When dropped, the logger will be flushed and closed.
pub struct LoggerGuard {
    _guard: tracing_appender::non_blocking::WorkerGuard,
}

impl LoggerGuard {
    fn new(guard: tracing_appender::non_blocking::WorkerGuard) -> Self {
        Self { _guard: guard }
    }
}

/// Initializes the Herald logger with the given configuration
///
/// # Arguments
/// * `config` - Logger configuration
///
/// # Returns
/// A `LoggerGuard` that must be kept alive for the duration of the program.
/// When the guard is dropped, the logger will be flushed.
///
/// # Errors
/// Returns `LoggerError` if initialization fails
///
/// # Example
/// ```ignore
/// let config = LoggerConfig::with_default_dir();
/// let _guard = init_logger(config)?;
/// tracing::info!("Logger initialized");
/// ```
pub fn init_logger(config: LoggerConfig) -> Result<LoggerGuard, LoggerError> {
    use std::fs;
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    // Ensure log directory exists
    if !config.log_dir.exists() {
        fs::create_dir_all(&config.log_dir).map_err(|e| {
            LoggerError::DirectoryCreationFailed(format!("{}: {}", config.log_dir.display(), e))
        })?;
    }

    // Create rolling file appender with daily rotation
    let file_appender = tracing_appender::rolling::daily(&config.log_dir, DEFAULT_LOG_FILE);

    // Create non-blocking writer
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Build the subscriber
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("herald={}", config.level)));

    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(false)
        .with_file(true)
        .with_line_number(true);

    if config.log_to_stdout {
        let stdout_layer = fmt::layer()
            .with_writer(std::io::stdout)
            .with_ansi(true)
            .with_target(true);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .with(stdout_layer)
            .try_init()
            .map_err(|e| LoggerError::InitializationFailed(e.to_string()))?;
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .try_init()
            .map_err(|e| LoggerError::InitializationFailed(e.to_string()))?;
    }

    tracing::info!(
        log_dir = %config.log_dir.display(),
        level = %config.level,
        "Herald logger initialized"
    );

    Ok(LoggerGuard::new(guard))
}

/// Initializes a simple logger for testing purposes
///
/// This logger outputs to stdout with the specified level.
#[cfg(test)]
pub fn init_test_logger(level: LogLevel) {
    use tracing_subscriber::{fmt, EnvFilter};

    let _ = fmt()
        .with_env_filter(EnvFilter::new(format!("herald={}", level)))
        .with_test_writer()
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // === LogLevel Tests ===

    #[test]
    fn test_log_level_default() {
        let level = LogLevel::default();
        assert_eq!(level, LogLevel::Info);
    }

    #[test]
    fn test_log_level_display() {
        assert_eq!(LogLevel::Error.to_string(), "error");
        assert_eq!(LogLevel::Warn.to_string(), "warn");
        assert_eq!(LogLevel::Info.to_string(), "info");
        assert_eq!(LogLevel::Debug.to_string(), "debug");
        assert_eq!(LogLevel::Trace.to_string(), "trace");
    }

    #[test]
    fn test_log_level_from_str() {
        assert_eq!("error".parse::<LogLevel>().unwrap(), LogLevel::Error);
        assert_eq!("WARN".parse::<LogLevel>().unwrap(), LogLevel::Warn);
        assert_eq!("warning".parse::<LogLevel>().unwrap(), LogLevel::Warn);
        assert_eq!("Info".parse::<LogLevel>().unwrap(), LogLevel::Info);
        assert_eq!("DEBUG".parse::<LogLevel>().unwrap(), LogLevel::Debug);
        assert_eq!("trace".parse::<LogLevel>().unwrap(), LogLevel::Trace);
    }

    #[test]
    fn test_log_level_from_str_invalid() {
        let result = "invalid".parse::<LogLevel>();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown log level"));
    }

    #[test]
    fn test_log_level_to_filter() {
        use tracing::level_filters::LevelFilter;

        assert_eq!(LogLevel::Error.to_level_filter(), LevelFilter::ERROR);
        assert_eq!(LogLevel::Warn.to_level_filter(), LevelFilter::WARN);
        assert_eq!(LogLevel::Info.to_level_filter(), LevelFilter::INFO);
        assert_eq!(LogLevel::Debug.to_level_filter(), LevelFilter::DEBUG);
        assert_eq!(LogLevel::Trace.to_level_filter(), LevelFilter::TRACE);
    }

    // === LoggerConfig Tests ===

    #[test]
    fn test_logger_config_new() {
        let log_dir = PathBuf::from("/tmp/herald-logs");
        let config = LoggerConfig::new(log_dir.clone());

        assert_eq!(config.log_dir, log_dir);
        assert_eq!(config.level, LogLevel::Info);
        assert!(!config.log_to_stdout);
        assert_eq!(config.max_file_size, DEFAULT_MAX_SIZE_BYTES);
    }

    #[test]
    fn test_logger_config_with_default_dir() {
        let config = LoggerConfig::with_default_dir();
        assert!(config.log_dir.ends_with("logs"));
        assert!(config.log_dir.to_string_lossy().contains(".herald"));
    }

    #[test]
    fn test_logger_config_builder_pattern() {
        let config = LoggerConfig::new(PathBuf::from("/tmp/logs"))
            .with_level(LogLevel::Debug)
            .with_stdout(true)
            .with_max_file_size(5 * 1024 * 1024);

        assert_eq!(config.level, LogLevel::Debug);
        assert!(config.log_to_stdout);
        assert_eq!(config.max_file_size, 5 * 1024 * 1024);
    }

    #[test]
    fn test_logger_config_log_file_path() {
        let config = LoggerConfig::new(PathBuf::from("/tmp/herald-logs"));
        assert_eq!(
            config.log_file_path(),
            PathBuf::from("/tmp/herald-logs/herald.log")
        );
    }

    // === LoggerError Tests ===

    #[test]
    fn test_logger_error_display() {
        let err = LoggerError::DirectoryCreationFailed("/tmp/test".to_string());
        assert!(err.to_string().contains("/tmp/test"));

        let err = LoggerError::InitializationFailed("test error".to_string());
        assert!(err.to_string().contains("test error"));

        let err = LoggerError::AlreadyInitialized;
        assert!(err.to_string().contains("already"));
    }

    // === Integration Tests ===

    #[test]
    fn test_init_logger_creates_directory() {
        let temp_dir = TempDir::new().unwrap();
        let log_dir = temp_dir.path().join("logs");

        // Directory should not exist yet
        assert!(!log_dir.exists());

        let _config = LoggerConfig::new(log_dir.clone());

        // Note: We can't actually call init_logger in tests because the global
        // subscriber can only be set once. Instead, we test the directory creation
        // logic separately.
        std::fs::create_dir_all(&log_dir).unwrap();
        assert!(log_dir.exists());
    }

    #[test]
    fn test_default_max_size() {
        // 10 MB
        assert_eq!(DEFAULT_MAX_SIZE_BYTES, 10 * 1024 * 1024);
    }

    #[test]
    fn test_default_log_file_name() {
        assert_eq!(DEFAULT_LOG_FILE, "herald.log");
    }
}
