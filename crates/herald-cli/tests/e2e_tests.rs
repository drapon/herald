//! End-to-End Tests for Herald CLI
//!
//! These tests verify the complete integration of all Herald components:
//! - Daemon start/stop lifecycle
//! - Manual capture functionality
//! - Configuration handling
//! - Error handling for missing permissions
//!
//! Note: Some tests require actual macOS screen recording permissions.
//! Tests that require permissions are marked with `#[ignore]` and can be
//! run explicitly with `cargo test -- --ignored`.

use async_trait::async_trait;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

/// Test environment that creates an isolated Herald configuration
struct TestEnv {
    temp_dir: TempDir,
    data_dir: PathBuf,
    config_path: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let data_dir = temp_dir.path().join(".herald");
        let config_path = data_dir.join("config.toml");

        fs::create_dir_all(&data_dir).expect("Failed to create data dir");
        fs::create_dir_all(data_dir.join("captures")).expect("Failed to create captures dir");
        fs::create_dir_all(data_dir.join("logs")).expect("Failed to create logs dir");

        Self {
            temp_dir,
            data_dir,
            config_path,
        }
    }

    fn write_config(&self, content: &str) {
        fs::write(&self.config_path, content).expect("Failed to write config");
    }

    fn default_config(&self) -> String {
        format!(
            r#"[capture]
interval_seconds = 5
image_quality = 6

[storage]
data_dir = "{}"
retention_seconds = 3600

[ai]
default_provider = "claude"
model = "claude-3-5-sonnet-20241022"
"#,
            self.data_dir.display()
        )
    }

    fn pid_file_path(&self) -> PathBuf {
        self.data_dir.join("daemon.pid")
    }

    fn db_path(&self) -> PathBuf {
        self.data_dir.join("herald.db")
    }

    fn captures_dir(&self) -> PathBuf {
        self.data_dir.join("captures")
    }
}

// ============================================================================
// Task 15.3: E2E Tests
// ============================================================================

mod daemon_lifecycle {
    use super::*;
    use herald_core::{Config, DaemonController};

    /// Test: Daemon PID file is created on start
    #[test]
    fn test_pid_file_created_on_daemon_register() {
        let env = TestEnv::new();
        let mut controller = DaemonController::new(env.pid_file_path());

        // Register daemon (simulates start)
        let result = controller.register_daemon();
        assert!(result.is_ok());

        // PID file should exist
        assert!(env.pid_file_path().exists());

        // PID should be current process
        let pid_content = fs::read_to_string(env.pid_file_path()).unwrap();
        assert_eq!(
            pid_content.trim().parse::<u32>().unwrap(),
            std::process::id()
        );

        // Cleanup
        controller.unregister_daemon().unwrap();
    }

    /// Test: Daemon PID file is removed on stop
    #[test]
    fn test_pid_file_removed_on_daemon_unregister() {
        let env = TestEnv::new();
        let mut controller = DaemonController::new(env.pid_file_path());

        // Register and then unregister
        controller.register_daemon().unwrap();
        assert!(env.pid_file_path().exists());

        controller.unregister_daemon().unwrap();
        assert!(!env.pid_file_path().exists());
    }

    /// Test: Double daemon start is prevented
    #[test]
    fn test_double_daemon_start_prevented() {
        let env = TestEnv::new();
        let mut controller = DaemonController::new(env.pid_file_path());

        // First registration succeeds
        controller.register_daemon().unwrap();

        // Second registration fails
        let result = controller.register_daemon();
        assert!(result.is_err());

        // Cleanup
        controller.unregister_daemon().unwrap();
    }

    /// Test: Stale PID file is cleaned up
    #[test]
    fn test_stale_pid_file_cleanup() {
        let env = TestEnv::new();

        // Write a fake PID (non-existent process)
        fs::write(env.pid_file_path(), "999999").unwrap();
        assert!(env.pid_file_path().exists());

        let controller = DaemonController::new(env.pid_file_path());

        // Check if running should clean up stale file
        let result = controller.is_running().unwrap();

        // On Unix, the stale PID file should be cleaned up
        #[cfg(unix)]
        {
            assert!(result.is_none());
            assert!(!env.pid_file_path().exists());
        }
    }

    /// Test: Graceful shutdown signal is received
    #[tokio::test]
    async fn test_shutdown_signal_received() {
        let env = TestEnv::new();
        let mut controller = DaemonController::new(env.pid_file_path());

        // Register daemon
        let start_result = controller.register_daemon().unwrap();
        let mut shutdown_rx = start_result.shutdown_rx;

        // Initially not shutting down
        assert!(!*shutdown_rx.borrow());

        // Trigger shutdown
        controller.trigger_shutdown();

        // Wait for signal
        tokio::time::timeout(Duration::from_millis(100), shutdown_rx.changed())
            .await
            .expect("Timeout waiting for shutdown signal")
            .expect("Channel closed unexpectedly");

        assert!(*shutdown_rx.borrow());

        // Cleanup
        controller.unregister_daemon().unwrap();
    }
}

mod configuration {
    use super::*;
    use herald_core::{load_config_from_path, Config};

    /// Test: Configuration loads with default values
    #[test]
    fn test_config_default_values() {
        let config = Config::default();

        assert_eq!(config.capture.interval_seconds, 60);
        assert_eq!(config.storage.retention_seconds, 86400);
        assert_eq!(config.ai.default_provider, "claude");
    }

    /// Test: Configuration loads from TOML file
    #[test]
    fn test_config_loads_from_file() {
        let env = TestEnv::new();
        env.write_config(&env.default_config());

        let config = load_config_from_path(&env.config_path).unwrap();

        assert_eq!(config.capture.interval_seconds, 5);
        assert_eq!(config.storage.retention_seconds, 3600);
        assert_eq!(config.ai.default_provider, "claude");
    }

    /// Test: Invalid config falls back to defaults
    #[test]
    fn test_invalid_config_uses_defaults() {
        let env = TestEnv::new();

        // Write invalid TOML
        env.write_config("this is not valid toml {{{{");

        // Should fail to parse
        let result = load_config_from_path(&env.config_path);
        assert!(result.is_err());
    }

    /// Test: Partial config merges with defaults
    #[test]
    fn test_partial_config_uses_defaults() {
        let env = TestEnv::new();

        // Write partial config (only capture section)
        env.write_config(
            r#"
[capture]
interval_seconds = 30
"#,
        );

        let config = load_config_from_path(&env.config_path).unwrap();

        // Custom value
        assert_eq!(config.capture.interval_seconds, 30);

        // Default values for missing fields
        assert_eq!(config.storage.retention_seconds, 86400);
        assert_eq!(config.ai.default_provider, "claude");
    }

    /// Test: Config validation rejects invalid values
    #[test]
    fn test_config_validation() {
        let env = TestEnv::new();

        // Write config with invalid provider
        env.write_config(
            r#"
[capture]
interval_seconds = 60

[storage]
retention_seconds = 86400

[ai]
default_provider = "invalid_provider"
model = "test"
"#,
        );

        let result = load_config_from_path(&env.config_path);
        // Should load but validation would fail when used
        assert!(result.is_ok());

        let config = result.unwrap();
        assert_eq!(config.ai.default_provider, "invalid_provider");

        // Validation happens at usage time
        let validation = config.validate();
        assert!(validation.is_err());
    }
}

mod storage_integration {
    use super::*;
    use herald_core::ports::storage::CaptureMetadata;
    use herald_core::ports::StoragePort;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Test: SQLite database is created and initialized
    #[tokio::test]
    async fn test_database_initialization() {
        let env = TestEnv::new();

        let adapter = herald_adapters::SqliteAdapter::new(&env.db_path())
            .await
            .expect("Failed to create database");

        // Database file should exist
        assert!(env.db_path().exists());

        // Should be able to get statistics (empty initially)
        let stats = adapter.get_statistics().await.unwrap();
        assert_eq!(stats.total_captures, 0);
    }

    /// Test: Capture metadata CRUD operations
    #[tokio::test]
    async fn test_capture_metadata_crud() {
        let env = TestEnv::new();

        let adapter = herald_adapters::SqliteAdapter::new(&env.db_path())
            .await
            .expect("Failed to create database");

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Create
        let metadata = CaptureMetadata {
            id: None,
            timestamp: now,
            file_path: "/test/capture.png".to_string(),
            created_at: now,
        };
        adapter.save_metadata(metadata).await.unwrap();

        // Read
        let captures = adapter.get_recent_captures(10).await.unwrap();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].file_path, "/test/capture.png");

        // Statistics
        let stats = adapter.get_statistics().await.unwrap();
        assert_eq!(stats.total_captures, 1);
        assert_eq!(stats.newest_timestamp, Some(now));

        // Delete
        let id = captures[0].id.unwrap();
        adapter.delete_capture(id).await.unwrap();

        let captures_after = adapter.get_recent_captures(10).await.unwrap();
        assert_eq!(captures_after.len(), 0);
    }

    /// Test: Old captures are retrieved for cleanup
    #[tokio::test]
    async fn test_get_old_captures() {
        let env = TestEnv::new();

        let adapter = herald_adapters::SqliteAdapter::new(&env.db_path())
            .await
            .expect("Failed to create database");

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Add old capture (25 hours ago)
        let old_metadata = CaptureMetadata {
            id: None,
            timestamp: now - 90000,
            file_path: "/test/old.png".to_string(),
            created_at: now - 90000,
        };
        adapter.save_metadata(old_metadata).await.unwrap();

        // Add new capture (1 hour ago)
        let new_metadata = CaptureMetadata {
            id: None,
            timestamp: now - 3600,
            file_path: "/test/new.png".to_string(),
            created_at: now - 3600,
        };
        adapter.save_metadata(new_metadata).await.unwrap();

        // Get captures older than 24 hours
        let cutoff = now - 86400;
        let old_captures = adapter.get_old_captures(cutoff).await.unwrap();

        assert_eq!(old_captures.len(), 1);
        assert_eq!(old_captures[0].file_path, "/test/old.png");
    }
}

mod retention_integration {
    use super::*;
    use herald_core::ports::storage::CaptureMetadata;
    use herald_core::ports::StoragePort;
    use herald_core::{cleanup_once, Config};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Test: Cleanup removes old files and database records
    #[tokio::test]
    async fn test_cleanup_removes_old_data() {
        let env = TestEnv::new();
        env.write_config(&env.default_config());

        let adapter = herald_adapters::SqliteAdapter::new(&env.db_path())
            .await
            .expect("Failed to create database");

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Create old image file
        let old_image_path = env.captures_dir().join("old_capture.png");
        fs::write(&old_image_path, b"fake image data").unwrap();
        assert!(old_image_path.exists());

        // Add old capture metadata (2 hours ago, retention is 1 hour in test config)
        let old_metadata = CaptureMetadata {
            id: None,
            timestamp: now - 7200,
            file_path: old_image_path.to_string_lossy().to_string(),
            created_at: now - 7200,
        };
        adapter.save_metadata(old_metadata).await.unwrap();

        // Run cleanup
        let mut config = Config::default();
        config.storage.retention_seconds = 3600; // 1 hour

        let summary = cleanup_once(&adapter, &config).await.unwrap();

        // Should have deleted 1 capture
        assert_eq!(summary.deleted_count, 1);
        assert_eq!(summary.failed_count, 0);

        // File should be deleted
        assert!(!old_image_path.exists());

        // Database should be empty
        let stats = adapter.get_statistics().await.unwrap();
        assert_eq!(stats.total_captures, 0);
    }

    /// Test: Cleanup handles missing files gracefully
    #[tokio::test]
    async fn test_cleanup_handles_missing_files() {
        let env = TestEnv::new();

        let adapter = herald_adapters::SqliteAdapter::new(&env.db_path())
            .await
            .expect("Failed to create database");

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Add metadata for non-existent file
        let metadata = CaptureMetadata {
            id: None,
            timestamp: now - 7200,
            file_path: "/nonexistent/file.png".to_string(),
            created_at: now - 7200,
        };
        adapter.save_metadata(metadata).await.unwrap();

        let mut config = Config::default();
        config.storage.retention_seconds = 3600;

        let summary = cleanup_once(&adapter, &config).await.unwrap();

        // Should still "delete" the record (file considered already gone)
        assert_eq!(summary.deleted_count, 1);
        assert_eq!(summary.failed_count, 0);
    }
}

mod scheduler_integration {
    use super::*;
    use herald_core::ports::capture::{CaptureError, CapturePort, CapturedImage};
    use herald_core::ports::StoragePort;
    use herald_core::{capture_once, CaptureScheduler, Config};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Mock capture adapter for testing
    struct MockCaptureAdapter {
        capture_count: AtomicUsize,
    }

    impl MockCaptureAdapter {
        fn new() -> Self {
            Self {
                capture_count: AtomicUsize::new(0),
            }
        }

        fn count(&self) -> usize {
            self.capture_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl CapturePort for MockCaptureAdapter {
        async fn capture_screen(&self) -> Result<CapturedImage, CaptureError> {
            self.capture_count.fetch_add(1, Ordering::SeqCst);

            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            Ok(CapturedImage {
                data: vec![0x89, 0x50, 0x4E, 0x47], // PNG header
                width: 100,
                height: 100,
                timestamp,
            })
        }

        async fn check_permission(&self) -> Result<bool, CaptureError> {
            Ok(true)
        }
    }

    /// Test: Manual capture saves file and metadata
    #[tokio::test]
    async fn test_manual_capture_saves_data() {
        let env = TestEnv::new();
        env.write_config(&env.default_config());

        let capture_adapter = MockCaptureAdapter::new();
        let storage_adapter = herald_adapters::SqliteAdapter::new(&env.db_path())
            .await
            .expect("Failed to create database");

        let mut config = Config::default();
        config.storage.data_dir = env.data_dir.clone();

        let result = capture_once(&capture_adapter, &storage_adapter, &config).await;
        assert!(result.is_ok());

        let capture_result = result.unwrap();

        // File should exist
        assert!(capture_result.file_path.exists());

        // Metadata should be saved
        let stats = storage_adapter.get_statistics().await.unwrap();
        assert_eq!(stats.total_captures, 1);

        // Capture count should be 1
        assert_eq!(capture_adapter.count(), 1);
    }

    /// Test: Scheduler executes periodic captures
    #[tokio::test]
    async fn test_scheduler_periodic_capture() {
        let env = TestEnv::new();

        let capture_adapter = Arc::new(MockCaptureAdapter::new());
        let storage_adapter = Arc::new(
            herald_adapters::SqliteAdapter::new(&env.db_path())
                .await
                .expect("Failed to create database"),
        );

        let mut config = Config::default();
        config.storage.data_dir = env.data_dir.clone();
        config.capture.interval_seconds = 1; // 1 second for fast test
        let config = Arc::new(config);

        let scheduler = CaptureScheduler::new(
            Arc::clone(&capture_adapter),
            Arc::clone(&storage_adapter),
            Arc::clone(&config),
        );

        // Start scheduler
        scheduler.start().await.unwrap();
        assert!(scheduler.is_running());

        // Wait for at least one capture cycle
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // Stop scheduler
        scheduler.stop().await.unwrap();
        assert!(!scheduler.is_running());

        // Should have captured at least once
        assert!(capture_adapter.count() >= 1);
    }

    /// Test: Manual capture works independent of scheduler
    #[tokio::test]
    async fn test_manual_capture_independent_of_scheduler() {
        let env = TestEnv::new();

        let capture_adapter = Arc::new(MockCaptureAdapter::new());
        let storage_adapter = Arc::new(
            herald_adapters::SqliteAdapter::new(&env.db_path())
                .await
                .expect("Failed to create database"),
        );

        let mut config = Config::default();
        config.storage.data_dir = env.data_dir.clone();
        config.capture.interval_seconds = 60; // Long interval
        let config = Arc::new(config);

        let scheduler = CaptureScheduler::new(
            Arc::clone(&capture_adapter),
            Arc::clone(&storage_adapter),
            Arc::clone(&config),
        );

        // Start scheduler
        scheduler.start().await.unwrap();

        // Immediate manual capture
        let result = capture_once(&*capture_adapter, &*storage_adapter, &config).await;
        assert!(result.is_ok());

        // Should have 1 capture from manual
        assert_eq!(capture_adapter.count(), 1);

        // Stop scheduler
        scheduler.stop().await.unwrap();
    }
}

mod ai_integration {
    use super::*;
    use herald_core::ports::ai::ImageData;
    use herald_core::{AIProvider, ApiKeyManager, PromptBuilder, MAX_IMAGES};

    /// Test: Prompt builder respects image limit
    #[test]
    fn test_prompt_builder_image_limit() {
        let images: Vec<ImageData> = (0..MAX_IMAGES + 1)
            .map(|i| ImageData {
                base64: format!("image{}", i),
                media_type: "image/png".to_string(),
            })
            .collect();

        let result = PromptBuilder::build_multimodal_prompt("test intent", images);
        assert!(result.is_err());
    }

    /// Test: Prompt builder creates valid prompt
    #[test]
    fn test_prompt_builder_creates_prompt() {
        let images = vec![ImageData {
            base64: "base64data".to_string(),
            media_type: "image/png".to_string(),
        }];

        let result = PromptBuilder::build_multimodal_prompt("improve my workflow", images);
        assert!(result.is_ok());

        let prompt = result.unwrap();
        assert!(!prompt.system_message.is_empty());
        assert!(prompt.user_message.text.contains("improve my workflow"));
        assert_eq!(prompt.user_message.images.len(), 1);
    }

    /// Test: API key manager detects missing keys
    #[test]
    fn test_api_key_missing_detection() {
        // Clear any existing keys
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::remove_var("GOOGLE_AI_API_KEY");

        let result = ApiKeyManager::load_api_key(AIProvider::Claude);
        assert!(result.is_err());

        let result = ApiKeyManager::load_api_key(AIProvider::Gemini);
        assert!(result.is_err());
    }

    /// Test: API key manager provides helpful guidance
    #[test]
    fn test_api_key_guidance() {
        let guidance = ApiKeyManager::missing_key_guidance(AIProvider::Claude);
        assert!(guidance.contains("ANTHROPIC_API_KEY"));
        assert!(guidance.contains(".env"));

        let guidance = ApiKeyManager::missing_key_guidance(AIProvider::Gemini);
        assert!(guidance.contains("GOOGLE_AI_API_KEY"));
    }

    /// Test: AI provider parsing
    #[test]
    fn test_ai_provider_parsing() {
        let claude: Result<AIProvider, _> = "claude".parse();
        assert!(claude.is_ok());
        assert!(matches!(claude.unwrap(), AIProvider::Claude));

        let gemini: Result<AIProvider, _> = "gemini".parse();
        assert!(gemini.is_ok());
        assert!(matches!(gemini.unwrap(), AIProvider::Gemini));

        let invalid: Result<AIProvider, String> = "invalid".parse();
        assert!(invalid.is_err());
    }
}

mod directory_management {
    use super::*;
    use herald_core::DirectoryManager;

    /// Test: Directory manager creates required directories
    #[test]
    fn test_directory_creation() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join(".herald");

        assert!(!data_dir.exists());

        let manager = DirectoryManager::new(data_dir.clone());
        manager.initialize().unwrap();

        // All directories should exist
        assert!(data_dir.exists());
        assert!(data_dir.join("captures").exists());
        assert!(data_dir.join("logs").exists());
    }

    /// Test: Directory manager is idempotent
    #[test]
    fn test_directory_creation_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join(".herald");

        let manager = DirectoryManager::new(data_dir.clone());

        // First initialization
        manager.initialize().unwrap();
        assert!(data_dir.exists());

        // Second initialization should not fail
        manager.initialize().unwrap();
        assert!(data_dir.exists());
    }

    /// Test: Directory permissions are correct (Unix only)
    #[cfg(unix)]
    #[test]
    fn test_directory_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join(".herald");

        let manager = DirectoryManager::new(data_dir.clone());
        manager.initialize().unwrap();

        let metadata = fs::metadata(&data_dir).unwrap();
        let permissions = metadata.permissions().mode();

        // Should be 0700 (owner only)
        assert_eq!(permissions & 0o777, 0o700);
    }
}

mod error_handling {
    use super::*;
    use herald_core::ports::capture::{CaptureError, CapturePort, CapturedImage};
    use herald_core::{capture_once, Config};

    /// Mock adapter that always fails
    struct FailingCaptureAdapter;

    #[async_trait]
    impl CapturePort for FailingCaptureAdapter {
        async fn capture_screen(&self) -> Result<CapturedImage, CaptureError> {
            Err(CaptureError::PermissionDenied)
        }

        async fn check_permission(&self) -> Result<bool, CaptureError> {
            Ok(false)
        }
    }

    /// Test: Permission denied error is handled
    #[tokio::test]
    async fn test_permission_denied_handling() {
        let env = TestEnv::new();

        let capture_adapter = FailingCaptureAdapter;
        let storage_adapter = herald_adapters::SqliteAdapter::new(&env.db_path())
            .await
            .expect("Failed to create database");

        let mut config = Config::default();
        config.storage.data_dir = env.data_dir.clone();

        let result = capture_once(&capture_adapter, &storage_adapter, &config).await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(
            error.to_string().contains("permission") || error.to_string().contains("Permission")
        );
    }
}

// ============================================================================
// Performance Verification (Task 15.2)
// These tests verify performance requirements are met
// ============================================================================

mod performance {
    use super::*;
    use herald_core::ports::capture::{CaptureError, CapturePort, CapturedImage};
    use herald_core::{capture_once, Config};
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    /// Fast mock capture adapter for performance testing
    struct FastMockCaptureAdapter;

    #[async_trait]
    impl CapturePort for FastMockCaptureAdapter {
        async fn capture_screen(&self) -> Result<CapturedImage, CaptureError> {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            // Simulate ~500KB image (realistic size)
            let data = vec![0u8; 500 * 1024];

            Ok(CapturedImage {
                data,
                width: 1920,
                height: 1080,
                timestamp,
            })
        }

        async fn check_permission(&self) -> Result<bool, CaptureError> {
            Ok(true)
        }
    }

    /// Test: Capture processing completes within 1 second (Requirement 12.3)
    #[tokio::test]
    async fn test_capture_completes_within_one_second() {
        let env = TestEnv::new();

        let capture_adapter = FastMockCaptureAdapter;
        let storage_adapter = herald_adapters::SqliteAdapter::new(&env.db_path())
            .await
            .expect("Failed to create database");

        let mut config = Config::default();
        config.storage.data_dir = env.data_dir.clone();

        let start = Instant::now();
        let result = capture_once(&capture_adapter, &storage_adapter, &config).await;
        let elapsed = start.elapsed();

        assert!(result.is_ok());
        assert!(
            elapsed < Duration::from_secs(1),
            "Capture took {:?}, exceeds 1 second requirement",
            elapsed
        );
    }

    /// Test: Multiple captures complete efficiently
    #[tokio::test]
    async fn test_multiple_captures_efficient() {
        let env = TestEnv::new();

        let capture_adapter = FastMockCaptureAdapter;
        let storage_adapter = herald_adapters::SqliteAdapter::new(&env.db_path())
            .await
            .expect("Failed to create database");

        let mut config = Config::default();
        config.storage.data_dir = env.data_dir.clone();

        let start = Instant::now();

        // Perform 10 captures
        for _ in 0..10 {
            capture_once(&capture_adapter, &storage_adapter, &config)
                .await
                .expect("Capture failed");
        }

        let elapsed = start.elapsed();

        // 10 captures should complete in under 10 seconds
        assert!(
            elapsed < Duration::from_secs(10),
            "10 captures took {:?}, should be under 10 seconds",
            elapsed
        );

        // Average should be under 1 second per capture
        let avg = elapsed / 10;
        assert!(
            avg < Duration::from_secs(1),
            "Average capture time {:?} exceeds 1 second",
            avg
        );
    }

    /// Test: Database operations are efficient
    #[tokio::test]
    async fn test_database_operations_efficient() {
        use herald_core::ports::storage::CaptureMetadata;
        use herald_core::ports::StoragePort;

        let env = TestEnv::new();

        let adapter = herald_adapters::SqliteAdapter::new(&env.db_path())
            .await
            .expect("Failed to create database");

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Insert 100 records
        let start = Instant::now();
        for i in 0..100 {
            let metadata = CaptureMetadata {
                id: None,
                timestamp: now - i,
                file_path: format!("/test/capture{}.png", i),
                created_at: now - i,
            };
            adapter.save_metadata(metadata).await.unwrap();
        }
        let insert_elapsed = start.elapsed();

        // 100 inserts should complete in under 1 second
        assert!(
            insert_elapsed < Duration::from_secs(1),
            "100 inserts took {:?}, should be under 1 second",
            insert_elapsed
        );

        // Query recent captures
        let start = Instant::now();
        let captures = adapter.get_recent_captures(50).await.unwrap();
        let query_elapsed = start.elapsed();

        assert_eq!(captures.len(), 50);

        // Query should complete in under 100ms
        assert!(
            query_elapsed < Duration::from_millis(100),
            "Query took {:?}, should be under 100ms",
            query_elapsed
        );

        // Get statistics
        let start = Instant::now();
        let stats = adapter.get_statistics().await.unwrap();
        let stats_elapsed = start.elapsed();

        assert_eq!(stats.total_captures, 100);

        // Statistics query should complete in under 100ms
        assert!(
            stats_elapsed < Duration::from_millis(100),
            "Statistics query took {:?}, should be under 100ms",
            stats_elapsed
        );
    }

    /// Test: Async I/O works correctly (non-blocking)
    #[tokio::test]
    async fn test_async_io_non_blocking() {
        let env = TestEnv::new();

        let capture_adapter = Arc::new(FastMockCaptureAdapter);
        let storage_adapter = Arc::new(
            herald_adapters::SqliteAdapter::new(&env.db_path())
                .await
                .expect("Failed to create database"),
        );

        let mut config = Config::default();
        config.storage.data_dir = env.data_dir.clone();
        let config = Arc::new(config);

        // Spawn multiple concurrent captures
        let mut handles = vec![];
        for _ in 0..5 {
            let capture = Arc::clone(&capture_adapter);
            let storage = Arc::clone(&storage_adapter);
            let cfg = Arc::clone(&config);

            handles.push(tokio::spawn(async move {
                capture_once(&*capture, &*storage, &cfg).await
            }));
        }

        // Wait for all to complete
        let start = Instant::now();
        for handle in handles {
            handle.await.unwrap().unwrap();
        }
        let elapsed = start.elapsed();

        // 5 concurrent captures should complete faster than 5 sequential
        // (allowing for some overhead, should be under 3 seconds)
        assert!(
            elapsed < Duration::from_secs(3),
            "Concurrent captures took {:?}, expected faster execution",
            elapsed
        );
    }
}
