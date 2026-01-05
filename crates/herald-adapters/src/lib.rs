//! Herald Adapters - Infrastructure implementations
//!
//! This crate contains concrete implementations of the ports defined in herald-core,
//! including ScreenCaptureKit integration, SQLite storage, and AI provider clients.

pub mod ai;
pub mod capture;
pub mod storage;

// Re-export primary adapter types
pub use storage::SqliteAdapter;

#[cfg(target_os = "macos")]
pub use capture::ScreenCaptureKitAdapter;

#[cfg(test)]
mod tests {
    use herald_core::config::Config;

    #[test]
    fn test_can_access_core_types() {
        // Verify that herald-adapters can use herald-core types
        let config = Config::default();
        assert_eq!(config.capture.interval_seconds, 60);
    }
}
