//! Herald Core - Domain logic for the Herald screen capture assistant
//!
//! This crate contains the core business logic, domain models, and port definitions
//! following the Hexagonal Architecture pattern.

pub mod api_key;
pub mod config;
pub mod daemon;
pub mod directory;
pub mod error;
pub mod logging;
pub mod ports;
pub mod prompt;
pub mod retention;
pub mod scheduler;

// Re-export primary types for convenient access
pub use api_key::{
    AIProvider, ApiKeyManager, SecretApiKey, ANTHROPIC_API_KEY_ENV, GOOGLE_AI_API_KEY_ENV,
};
pub use config::{
    get_default_config_path, load_config, load_config_from_path, AiConfig, CaptureConfig, Config,
    StorageConfig,
};
pub use daemon::{DaemonController, DaemonError, DaemonStartResult, PidManager, ShutdownSender};
pub use directory::DirectoryManager;
pub use error::{AIError, CaptureError, ConfigError, HeraldError, StorageError};
pub use logging::{init_logger, LogLevel, LoggerConfig, LoggerError, LoggerGuard};
pub use prompt::{PromptBuilder, MAX_IMAGES};
pub use retention::{cleanup_once, CleanupSummary, RetentionError, RetentionManager};
pub use scheduler::{capture_once, CaptureResult, CaptureScheduler, SchedulerError};

#[cfg(test)]
mod tests {
    #[test]
    fn test_crate_compiles() {
        // Basic sanity test to ensure the crate structure is valid
        assert!(true);
    }
}
