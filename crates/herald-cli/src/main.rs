//! Herald CLI - AI-powered screen capture assistant
//!
//! Main entry point for the Herald application.

use anyhow::Result;

fn main() -> Result<()> {
    println!("Herald v{}", env!("CARGO_PKG_VERSION"));
    println!("AI-powered screen capture assistant for macOS");

    // TODO: Implement CLI commands in Task 11.1
    // - herald daemon start/stop
    // - herald capture
    // - herald status
    // - herald suggest "<intent>"

    Ok(())
}

#[cfg(test)]
mod tests {
    use herald_core::config::Config;
    use herald_core::ports::{AIProviderPort, CapturePort, StoragePort};

    #[test]
    fn test_can_access_core_types() {
        // Verify CLI can use herald-core types
        let config = Config::default();
        assert_eq!(config.capture.interval_seconds, 60);
        assert_eq!(config.storage.retention_seconds, 86400);
        assert_eq!(config.ai.default_provider, "claude");
    }

    #[test]
    fn test_port_traits_are_accessible() {
        // Verify port traits are importable (compile-time check)
        fn _assert_capture_port<T: CapturePort>() {}
        fn _assert_storage_port<T: StoragePort>() {}
        fn _assert_ai_provider_port<T: AIProviderPort>() {}
    }
}
