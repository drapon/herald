//! Common error types for Herald
//!
//! This module provides centralized error handling for the Herald application.
//! All domain-specific errors are defined in their respective port modules and
//! re-exported here for convenience.

use thiserror::Error;

// Re-export domain-specific errors from ports
pub use crate::logging::LoggerError;
pub use crate::ports::ai::AIError;
pub use crate::ports::capture::CaptureError;
pub use crate::ports::storage::StorageError;

/// Top-level error type for Herald operations
///
/// This enum wraps all domain-specific errors and provides automatic conversion
/// via the `From` trait, enabling seamless error propagation with `?`.
#[derive(Debug, Error)]
pub enum HeraldError {
    /// Configuration-related errors
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    /// Capture-related errors
    #[error("Capture error: {0}")]
    Capture(#[from] CaptureError),

    /// Storage-related errors
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    /// AI provider errors
    #[error("AI error: {0}")]
    AI(#[from] AIError),

    /// Logger errors
    #[error("Logger error: {0}")]
    Logger(#[from] LoggerError),

    /// IO errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Configuration-specific errors
#[derive(Debug, Error)]
pub enum ConfigError {
    /// File not found
    #[error("Configuration file not found: {0}")]
    NotFound(String),

    /// Parse error
    #[error("Failed to parse configuration: {0}")]
    ParseError(String),

    /// Invalid value
    #[error("Invalid configuration value: {0}")]
    InvalidValue(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::InvalidValue("interval_seconds must be > 0".to_string());
        assert!(err.to_string().contains("interval_seconds"));
    }

    #[test]
    fn test_herald_error_from_config() {
        let config_err = ConfigError::NotFound("config.toml".to_string());
        let herald_err: HeraldError = config_err.into();
        assert!(matches!(herald_err, HeraldError::Config(_)));
    }

    // === CaptureError Tests ===
    #[test]
    fn test_capture_error_permission_denied() {
        let err = CaptureError::PermissionDenied;
        assert!(err.to_string().contains("permission"));
    }

    #[test]
    fn test_capture_error_unsupported_os() {
        let err = CaptureError::UnsupportedOS("12.0".to_string());
        assert!(err.to_string().contains("12.3"));
    }

    #[test]
    fn test_capture_error_failed() {
        let err = CaptureError::CaptureFailed("Stream error".to_string());
        assert!(err.to_string().contains("Stream error"));
    }

    #[test]
    fn test_herald_error_from_capture() {
        let capture_err = CaptureError::PermissionDenied;
        let herald_err: HeraldError = capture_err.into();
        assert!(matches!(herald_err, HeraldError::Capture(_)));
    }

    // === StorageError Tests ===
    #[test]
    fn test_storage_error_database() {
        let err = StorageError::DatabaseError("Connection failed".to_string());
        assert!(err.to_string().contains("Connection failed"));
    }

    #[test]
    fn test_storage_error_connection() {
        let err = StorageError::ConnectionError("Timeout".to_string());
        assert!(err.to_string().contains("Timeout"));
    }

    #[test]
    fn test_storage_error_not_found() {
        let err = StorageError::NotFound(42);
        assert!(err.to_string().contains("42"));
    }

    #[test]
    fn test_herald_error_from_storage() {
        let storage_err = StorageError::DatabaseError("test".to_string());
        let herald_err: HeraldError = storage_err.into();
        assert!(matches!(herald_err, HeraldError::Storage(_)));
    }

    // === AIError Tests ===
    #[test]
    fn test_ai_error_unauthorized() {
        let err = AIError::Unauthorized;
        let msg = err.to_string();
        assert!(msg.contains("API key") || msg.contains("Unauthorized"));
    }

    #[test]
    fn test_ai_error_rate_limit() {
        let err = AIError::RateLimitExceeded;
        assert!(err.to_string().to_lowercase().contains("rate"));
    }

    #[test]
    fn test_ai_error_invalid_request() {
        let err = AIError::InvalidRequest("Too many images".to_string());
        assert!(err.to_string().contains("Too many images"));
    }

    #[test]
    fn test_ai_error_provider() {
        let err = AIError::ProviderError("claude".to_string(), "Server error".to_string());
        let msg = err.to_string();
        assert!(msg.contains("claude"));
        assert!(msg.contains("Server error"));
    }

    #[test]
    fn test_herald_error_from_ai() {
        let ai_err = AIError::Unauthorized;
        let herald_err: HeraldError = ai_err.into();
        assert!(matches!(herald_err, HeraldError::AI(_)));
    }

    // === LoggerError Tests ===
    #[test]
    fn test_logger_error_display() {
        let err = LoggerError::DirectoryCreationFailed("/tmp/logs".to_string());
        assert!(err.to_string().contains("/tmp/logs"));
    }

    #[test]
    fn test_herald_error_from_logger() {
        let logger_err = LoggerError::AlreadyInitialized;
        let herald_err: HeraldError = logger_err.into();
        assert!(matches!(herald_err, HeraldError::Logger(_)));
    }

    // === Anyhow Interoperability Tests ===
    #[test]
    fn test_herald_error_to_anyhow() {
        let err = HeraldError::Config(ConfigError::InvalidValue("test".to_string()));
        let anyhow_err: anyhow::Error = err.into();
        assert!(anyhow_err.to_string().contains("test"));
    }

    #[test]
    fn test_result_with_anyhow() {
        fn fallible_operation() -> anyhow::Result<()> {
            Err(CaptureError::PermissionDenied)?
        }

        let result = fallible_operation();
        assert!(result.is_err());
    }
}
