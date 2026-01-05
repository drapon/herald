//! Application initialization and lifecycle management
//!
//! Provides centralized initialization sequence and fatal error handling
//! for the Herald CLI application.

use anyhow::{Context, Result};
use herald_core::{
    init_logger, load_config, Config, DirectoryManager, LogLevel, LoggerConfig, LoggerGuard,
};
use std::panic;
use std::sync::Arc;
use tracing::{error, info};

/// Application context holding initialized components
pub struct AppContext {
    /// Application configuration
    pub config: Arc<Config>,
    /// Logger guard (keeps logger alive)
    #[allow(dead_code)]
    logger_guard: Option<LoggerGuard>,
}

impl AppContext {
    /// Returns reference to the configuration
    pub fn config(&self) -> &Config {
        &self.config
    }
}

/// Application initialization options
#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    /// Whether to initialize the logger
    pub init_logger: bool,
    /// Whether to create directory structure
    pub create_directories: bool,
    /// Log level override
    pub log_level: Option<LogLevel>,
}

impl InitOptions {
    /// Creates options for daemon mode (full initialization)
    pub fn daemon() -> Self {
        Self {
            init_logger: true,
            create_directories: true,
            log_level: Some(LogLevel::Info),
        }
    }

    /// Creates options for CLI command mode (minimal initialization)
    pub fn command() -> Self {
        Self {
            init_logger: false,
            create_directories: true,
            log_level: None,
        }
    }
}

/// Initializes the Herald application
///
/// This function performs the following initialization sequence:
/// 1. Load configuration from `~/.herald/config.toml`
/// 2. Initialize directory structure (`~/.herald/`, `~/.herald/captures/`, etc.)
/// 3. Initialize logging (if requested)
/// 4. Set up panic hook for fatal error handling
///
/// # Arguments
/// * `options` - Initialization options
///
/// # Returns
/// * `Ok(AppContext)` - Initialized application context
/// * `Err` - Initialization failed
///
/// # Example
/// ```ignore
/// let ctx = initialize(InitOptions::daemon()).await?;
/// let config = ctx.config();
/// ```
pub fn initialize(options: InitOptions) -> Result<AppContext> {
    // Step 1: Load configuration
    let config = load_config().context("Failed to load configuration")?;
    let config = Arc::new(config);

    // Step 2: Create directory structure
    if options.create_directories {
        let dir_manager = DirectoryManager::new(config.storage.data_dir.clone());
        dir_manager
            .initialize()
            .context("Failed to create directory structure")?;
    }

    // Step 3: Initialize logging
    let logger_guard = if options.init_logger {
        let log_level = options.log_level.unwrap_or(LogLevel::Info);
        let logger_config = LoggerConfig::new(config.storage.data_dir.join("logs"))
            .with_level(log_level)
            .with_max_file_size(10 * 1024 * 1024); // 10MB

        Some(init_logger(logger_config).context("Failed to initialize logger")?)
    } else {
        None
    };

    // Step 4: Set up panic hook for fatal errors
    setup_panic_hook(Arc::clone(&config));

    Ok(AppContext {
        config,
        logger_guard,
    })
}

/// Sets up a custom panic hook for fatal error handling
///
/// The panic hook:
/// 1. Logs the panic information
/// 2. Attempts to clean up resources (e.g., PID file)
/// 3. Prints user-friendly error message
fn setup_panic_hook(config: Arc<Config>) {
    let default_hook = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        // Log the panic
        let location = panic_info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };

        error!("FATAL ERROR at {}: {}", location, message);

        // Attempt to clean up PID file
        let pid_file = config.storage.data_dir.join("daemon.pid");
        if pid_file.exists() {
            if let Err(e) = std::fs::remove_file(&pid_file) {
                error!("Failed to clean up PID file: {}", e);
            } else {
                info!("Cleaned up PID file after panic");
            }
        }

        // Print user-friendly error message
        eprintln!();
        eprintln!("Herald encountered a fatal error and must exit.");
        eprintln!("Location: {}", location);
        eprintln!("Error: {}", message);
        eprintln!();
        eprintln!(
            "Please check the log file at: {}/logs/herald.log",
            config.storage.data_dir.display()
        );
        eprintln!();

        // Call the default hook for standard panic behavior
        default_hook(panic_info);
    }));
}

/// Performs graceful shutdown of the application
///
/// This function should be called when the application receives a shutdown signal
/// or encounters a fatal error. It:
/// 1. Logs the shutdown reason
/// 2. Cleans up resources
/// 3. Returns appropriate exit code
///
/// # Arguments
/// * `reason` - The reason for shutdown
/// * `config` - Application configuration
///
/// # Returns
/// Exit code (0 for success, 1 for error)
pub fn graceful_shutdown(reason: &str, config: &Config) -> i32 {
    info!("Initiating graceful shutdown: {}", reason);

    // Clean up PID file if it exists
    let pid_file = config.storage.data_dir.join("daemon.pid");
    if pid_file.exists() {
        match std::fs::remove_file(&pid_file) {
            Ok(_) => info!("Cleaned up PID file"),
            Err(e) => error!("Failed to clean up PID file: {}", e),
        }
    }

    info!("Shutdown complete");

    if reason.contains("error") || reason.contains("fatal") {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_options_daemon() {
        let options = InitOptions::daemon();
        assert!(options.init_logger);
        assert!(options.create_directories);
        assert_eq!(options.log_level, Some(LogLevel::Info));
    }

    #[test]
    fn test_init_options_command() {
        let options = InitOptions::command();
        assert!(!options.init_logger);
        assert!(options.create_directories);
        assert!(options.log_level.is_none());
    }

    #[test]
    fn test_graceful_shutdown_success() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = Config::default();
        config.storage.data_dir = temp_dir.path().to_path_buf();

        let exit_code = graceful_shutdown("normal shutdown", &config);
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn test_graceful_shutdown_error() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = Config::default();
        config.storage.data_dir = temp_dir.path().to_path_buf();

        let exit_code = graceful_shutdown("fatal error occurred", &config);
        assert_eq!(exit_code, 1);
    }

    #[test]
    fn test_graceful_shutdown_cleans_pid_file() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = Config::default();
        config.storage.data_dir = temp_dir.path().to_path_buf();

        // Create PID file
        let pid_file = temp_dir.path().join("daemon.pid");
        std::fs::write(&pid_file, "12345").unwrap();
        assert!(pid_file.exists());

        graceful_shutdown("shutdown", &config);

        // PID file should be deleted
        assert!(!pid_file.exists());
    }
}
