//! Data retention management for Herald
//!
//! Manages automatic cleanup of old capture data based on configured retention periods.
//! Runs periodic cleanup tasks and handles both file and database record deletion.

use crate::config::Config;
use crate::ports::storage::{StorageError, StoragePort};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use tokio::sync::Notify;
use tracing::{debug, error, info, warn};

/// Summary of a cleanup operation
#[derive(Debug, Clone, Default)]
pub struct CleanupSummary {
    /// Number of captures successfully deleted
    pub deleted_count: usize,
    /// Number of captures that failed to delete
    pub failed_count: usize,
    /// Total bytes freed by deleting files
    pub freed_bytes: u64,
}

/// Errors that can occur during retention management
#[derive(Debug, Error)]
pub enum RetentionError {
    /// Storage operation failed
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),

    /// File deletion failed
    #[error("File deletion failed: {0}")]
    FileDeletionFailed(String),

    /// Manager is already running
    #[error("Retention manager is already running")]
    AlreadyRunning,

    /// Manager is not running
    #[error("Retention manager is not running")]
    NotRunning,
}

/// Retention manager for automatic cleanup of old captures
///
/// The manager runs in a background Tokio task and periodically checks
/// for captures that have exceeded the configured retention period.
/// It deletes both the image file and the database record.
pub struct RetentionManager<S>
where
    S: StoragePort + 'static,
{
    storage_port: Arc<S>,
    config: Arc<Config>,
    /// Flag to indicate if manager is running
    running: Arc<AtomicBool>,
    /// Signal to stop the manager
    stop_signal: Arc<Notify>,
    /// Cleanup interval (default: 1 hour)
    cleanup_interval: Duration,
}

impl<S> RetentionManager<S>
where
    S: StoragePort + 'static,
{
    /// Creates a new retention manager
    ///
    /// # Arguments
    /// * `storage_port` - The storage port implementation
    /// * `config` - Application configuration containing retention settings
    pub fn new(storage_port: Arc<S>, config: Arc<Config>) -> Self {
        Self {
            storage_port,
            config,
            running: Arc::new(AtomicBool::new(false)),
            stop_signal: Arc::new(Notify::new()),
            cleanup_interval: Duration::from_secs(3600), // 1 hour
        }
    }

    /// Creates a retention manager with a custom cleanup interval (for testing)
    #[cfg(test)]
    pub fn with_interval(storage_port: Arc<S>, config: Arc<Config>, interval: Duration) -> Self {
        Self {
            storage_port,
            config,
            running: Arc::new(AtomicBool::new(false)),
            stop_signal: Arc::new(Notify::new()),
            cleanup_interval: interval,
        }
    }

    /// Returns whether the manager is currently running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Calculates the cutoff timestamp for retention
    ///
    /// Returns the Unix timestamp before which captures should be deleted.
    pub fn calculate_cutoff(&self) -> i64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        now - self.config.storage.retention_seconds as i64
    }

    /// Starts the periodic cleanup loop
    ///
    /// This spawns a background task that checks for and deletes old captures
    /// at the configured interval (default: 1 hour).
    ///
    /// # Errors
    /// Returns `RetentionError::AlreadyRunning` if manager is already active
    pub async fn start(&self) -> Result<(), RetentionError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(RetentionError::AlreadyRunning);
        }

        let storage_port = Arc::clone(&self.storage_port);
        let config = Arc::clone(&self.config);
        let running = Arc::clone(&self.running);
        let stop_signal = Arc::clone(&self.stop_signal);
        let interval = self.cleanup_interval;

        tokio::spawn(async move {
            info!(
                "Starting retention manager with interval: {:?}, retention period: {} seconds",
                interval, config.storage.retention_seconds
            );

            loop {
                tokio::select! {
                    _ = stop_signal.notified() => {
                        info!("Received stop signal, shutting down retention manager");
                        break;
                    }
                    _ = tokio::time::sleep(interval) => {
                        match Self::execute_cleanup_static(&storage_port, &config).await {
                            Ok(summary) => {
                                info!(
                                    "Cleanup completed: {} deleted, {} failed, {} bytes freed",
                                    summary.deleted_count,
                                    summary.failed_count,
                                    summary.freed_bytes
                                );
                            }
                            Err(e) => {
                                error!("Cleanup failed: {}", e);
                            }
                        }
                    }
                }
            }

            running.store(false, Ordering::SeqCst);
            info!("Retention manager stopped");
        });

        Ok(())
    }

    /// Stops the periodic cleanup loop
    ///
    /// Sends a stop signal to the background task and waits for it to finish.
    ///
    /// # Errors
    /// Returns `RetentionError::NotRunning` if manager is not active
    pub async fn stop(&self) -> Result<(), RetentionError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(RetentionError::NotRunning);
        }

        info!("Stopping retention manager...");
        self.stop_signal.notify_one();

        // Wait for the manager to stop
        while self.running.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }

    /// Executes a cleanup immediately
    ///
    /// This can be called independently of the periodic cleanup loop.
    ///
    /// # Returns
    /// `CleanupSummary` containing deletion statistics
    pub async fn cleanup_now(&self) -> Result<CleanupSummary, RetentionError> {
        Self::execute_cleanup_static(&self.storage_port, &self.config).await
    }

    /// Internal method to execute cleanup (static for use in spawned task)
    async fn execute_cleanup_static(
        storage_port: &Arc<S>,
        config: &Arc<Config>,
    ) -> Result<CleanupSummary, RetentionError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let cutoff = now - config.storage.retention_seconds as i64;

        debug!("Running cleanup with cutoff timestamp: {}", cutoff);

        // Get old captures
        let old_captures = storage_port.get_old_captures(cutoff).await?;

        if old_captures.is_empty() {
            debug!("No captures to clean up");
            return Ok(CleanupSummary::default());
        }

        info!("Found {} captures to clean up", old_captures.len());

        let mut summary = CleanupSummary::default();

        for capture in old_captures {
            let file_path = Path::new(&capture.file_path);
            let capture_id = capture.id.unwrap_or(0);

            // Try to get file size before deletion
            let file_size = if file_path.exists() {
                match fs::metadata(file_path).await {
                    Ok(meta) => meta.len(),
                    Err(_) => 0,
                }
            } else {
                0
            };

            // Delete the file first
            let file_deleted = if file_path.exists() {
                match fs::remove_file(file_path).await {
                    Ok(_) => {
                        debug!("Deleted file: {:?}", file_path);
                        true
                    }
                    Err(e) => {
                        warn!("Failed to delete file {:?}: {}", file_path, e);
                        false
                    }
                }
            } else {
                // File doesn't exist, consider it deleted
                debug!("File already deleted: {:?}", file_path);
                true
            };

            // Delete the database record
            if file_deleted {
                match storage_port.delete_capture(capture_id).await {
                    Ok(_) => {
                        debug!("Deleted database record: id={}", capture_id);
                        summary.deleted_count += 1;
                        summary.freed_bytes += file_size;
                    }
                    Err(e) => {
                        error!(
                            "Failed to delete database record id={}: {}. File was deleted.",
                            capture_id, e
                        );
                        summary.failed_count += 1;
                    }
                }
            } else {
                // Don't delete DB record if file deletion failed
                // This will be retried in the next cleanup cycle
                summary.failed_count += 1;
            }
        }

        Ok(summary)
    }
}

/// Performs a single cleanup without requiring a manager instance
///
/// This is a convenience function for one-shot cleanup operations.
///
/// # Arguments
/// * `storage_port` - The storage port implementation
/// * `config` - Application configuration
///
/// # Returns
/// `CleanupSummary` containing deletion statistics
pub async fn cleanup_once<S>(
    storage_port: &S,
    config: &Config,
) -> Result<CleanupSummary, RetentionError>
where
    S: StoragePort,
{
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let cutoff = now - config.storage.retention_seconds as i64;

    debug!("Running one-shot cleanup with cutoff timestamp: {}", cutoff);

    // Get old captures
    let old_captures = storage_port.get_old_captures(cutoff).await?;

    if old_captures.is_empty() {
        debug!("No captures to clean up");
        return Ok(CleanupSummary::default());
    }

    info!("Found {} captures to clean up", old_captures.len());

    let mut summary = CleanupSummary::default();

    for capture in old_captures {
        let file_path = Path::new(&capture.file_path);
        let capture_id = capture.id.unwrap_or(0);

        // Try to get file size before deletion
        let file_size = if file_path.exists() {
            match fs::metadata(file_path).await {
                Ok(meta) => meta.len(),
                Err(_) => 0,
            }
        } else {
            0
        };

        // Delete the file first
        let file_deleted = if file_path.exists() {
            match fs::remove_file(file_path).await {
                Ok(_) => {
                    debug!("Deleted file: {:?}", file_path);
                    true
                }
                Err(e) => {
                    warn!("Failed to delete file {:?}: {}", file_path, e);
                    false
                }
            }
        } else {
            // File doesn't exist, consider it deleted
            debug!("File already deleted: {:?}", file_path);
            true
        };

        // Delete the database record
        if file_deleted {
            match storage_port.delete_capture(capture_id).await {
                Ok(_) => {
                    debug!("Deleted database record: id={}", capture_id);
                    summary.deleted_count += 1;
                    summary.freed_bytes += file_size;
                }
                Err(e) => {
                    error!(
                        "Failed to delete database record id={}: {}. File was deleted.",
                        capture_id, e
                    );
                    summary.failed_count += 1;
                }
            }
        } else {
            // Don't delete DB record if file deletion failed
            summary.failed_count += 1;
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::storage::{CaptureMetadata, StorageStatistics};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // Mock storage port for testing
    struct MockStoragePort {
        captures: Mutex<HashMap<i64, CaptureMetadata>>,
        next_id: Mutex<i64>,
        should_fail_delete: AtomicBool,
    }

    impl MockStoragePort {
        fn new() -> Self {
            Self {
                captures: Mutex::new(HashMap::new()),
                next_id: Mutex::new(1),
                should_fail_delete: AtomicBool::new(false),
            }
        }

        fn add_capture(&self, timestamp: i64, file_path: &str) -> i64 {
            let mut next_id = self.next_id.lock().unwrap();
            let id = *next_id;
            *next_id += 1;

            let metadata = CaptureMetadata {
                id: Some(id),
                timestamp,
                file_path: file_path.to_string(),
                created_at: timestamp,
            };

            self.captures.lock().unwrap().insert(id, metadata);
            id
        }

        fn capture_count(&self) -> usize {
            self.captures.lock().unwrap().len()
        }

        fn set_should_fail_delete(&self, fail: bool) {
            self.should_fail_delete.store(fail, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl StoragePort for MockStoragePort {
        async fn save_metadata(&self, metadata: CaptureMetadata) -> Result<(), StorageError> {
            let mut next_id = self.next_id.lock().unwrap();
            let id = *next_id;
            *next_id += 1;

            let mut captures = self.captures.lock().unwrap();
            let metadata_with_id = CaptureMetadata {
                id: Some(id),
                ..metadata
            };
            captures.insert(id, metadata_with_id);
            Ok(())
        }

        async fn get_recent_captures(
            &self,
            limit: usize,
        ) -> Result<Vec<CaptureMetadata>, StorageError> {
            let captures = self.captures.lock().unwrap();
            let mut vec: Vec<_> = captures.values().cloned().collect();
            vec.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            vec.truncate(limit);
            Ok(vec)
        }

        async fn get_old_captures(
            &self,
            cutoff_timestamp: i64,
        ) -> Result<Vec<CaptureMetadata>, StorageError> {
            let captures = self.captures.lock().unwrap();
            let old: Vec<_> = captures
                .values()
                .filter(|c| c.timestamp < cutoff_timestamp)
                .cloned()
                .collect();
            Ok(old)
        }

        async fn delete_capture(&self, id: i64) -> Result<(), StorageError> {
            if self.should_fail_delete.load(Ordering::SeqCst) {
                return Err(StorageError::DatabaseError(
                    "Mock delete failure".to_string(),
                ));
            }

            let mut captures = self.captures.lock().unwrap();
            if captures.remove(&id).is_some() {
                Ok(())
            } else {
                Err(StorageError::NotFound(id))
            }
        }

        async fn get_statistics(&self) -> Result<StorageStatistics, StorageError> {
            let captures = self.captures.lock().unwrap();
            Ok(StorageStatistics {
                total_captures: captures.len(),
                total_size_bytes: 0,
                oldest_timestamp: captures.values().map(|c| c.timestamp).min(),
                newest_timestamp: captures.values().map(|c| c.timestamp).max(),
            })
        }
    }

    fn create_test_config(retention_seconds: u64) -> Config {
        let mut config = Config::default();
        config.storage.retention_seconds = retention_seconds;
        config
    }

    // === Task 9.1 Tests: Retention Check ===

    #[test]
    fn test_calculate_cutoff_timestamp() {
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400)); // 24 hours

        let manager = RetentionManager::new(storage, config);
        let cutoff = manager.calculate_cutoff();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Cutoff should be approximately 24 hours ago
        let expected_cutoff = now - 86400;
        assert!(
            (cutoff - expected_cutoff).abs() < 2,
            "Cutoff {} should be close to expected {}",
            cutoff,
            expected_cutoff
        );
    }

    #[test]
    fn test_calculate_cutoff_with_different_retention() {
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(3600)); // 1 hour

        let manager = RetentionManager::new(storage, config);
        let cutoff = manager.calculate_cutoff();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Cutoff should be approximately 1 hour ago
        let expected_cutoff = now - 3600;
        assert!(
            (cutoff - expected_cutoff).abs() < 2,
            "Cutoff {} should be close to expected {}",
            cutoff,
            expected_cutoff
        );
    }

    #[tokio::test]
    async fn test_get_old_captures_based_on_retention() {
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400)); // 24 hours

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Add captures: 2 old (25 hours ago), 2 new (1 hour ago)
        storage.add_capture(now - 90000, "/old1.png"); // 25 hours ago
        storage.add_capture(now - 100000, "/old2.png"); // ~27 hours ago
        storage.add_capture(now - 3600, "/new1.png"); // 1 hour ago
        storage.add_capture(now - 1800, "/new2.png"); // 30 min ago

        let manager = RetentionManager::new(Arc::clone(&storage), config);
        let cutoff = manager.calculate_cutoff();

        let old_captures = storage.get_old_captures(cutoff).await.unwrap();
        assert_eq!(old_captures.len(), 2);

        for capture in &old_captures {
            assert!(capture.timestamp < cutoff);
        }
    }

    // === Task 9.2 Tests: Auto Delete ===

    #[tokio::test]
    async fn test_cleanup_now_deletes_old_captures() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400));

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Create actual files for old captures
        let old_file1 = temp_dir.path().join("old1.png");
        let old_file2 = temp_dir.path().join("old2.png");
        fs::write(&old_file1, b"test data 1").await.unwrap();
        fs::write(&old_file2, b"test data 2").await.unwrap();

        // Add captures to storage
        storage.add_capture(now - 90000, old_file1.to_str().unwrap());
        storage.add_capture(now - 100000, old_file2.to_str().unwrap());
        storage.add_capture(now - 3600, "/new1.png"); // This shouldn't be deleted

        assert_eq!(storage.capture_count(), 3);

        let manager = RetentionManager::new(Arc::clone(&storage), config);
        let summary = manager.cleanup_now().await.unwrap();

        // Should delete 2 old captures
        assert_eq!(summary.deleted_count, 2);
        assert_eq!(summary.failed_count, 0);

        // Files should be deleted
        assert!(!old_file1.exists());
        assert!(!old_file2.exists());

        // Database should have only the new capture
        assert_eq!(storage.capture_count(), 1);
    }

    #[tokio::test]
    async fn test_cleanup_tracks_freed_bytes() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400));

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Create file with known size
        let old_file = temp_dir.path().join("old.png");
        let file_content = vec![0u8; 1024]; // 1KB
        fs::write(&old_file, &file_content).await.unwrap();

        storage.add_capture(now - 90000, old_file.to_str().unwrap());

        let manager = RetentionManager::new(Arc::clone(&storage), config);
        let summary = manager.cleanup_now().await.unwrap();

        assert_eq!(summary.deleted_count, 1);
        assert_eq!(summary.freed_bytes, 1024);
    }

    #[tokio::test]
    async fn test_cleanup_handles_missing_file_gracefully() {
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400));

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Add capture with non-existent file
        storage.add_capture(now - 90000, "/nonexistent/file.png");

        let manager = RetentionManager::new(Arc::clone(&storage), config);
        let summary = manager.cleanup_now().await.unwrap();

        // Should still delete the DB record
        assert_eq!(summary.deleted_count, 1);
        assert_eq!(summary.failed_count, 0);
        assert_eq!(storage.capture_count(), 0);
    }

    #[tokio::test]
    async fn test_cleanup_handles_db_delete_failure() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400));

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let old_file = temp_dir.path().join("old.png");
        fs::write(&old_file, b"test").await.unwrap();

        storage.add_capture(now - 90000, old_file.to_str().unwrap());
        storage.set_should_fail_delete(true);

        let manager = RetentionManager::new(Arc::clone(&storage), config);
        let summary = manager.cleanup_now().await.unwrap();

        // File deleted but DB delete failed
        assert_eq!(summary.deleted_count, 0);
        assert_eq!(summary.failed_count, 1);
        assert!(!old_file.exists()); // File was still deleted
    }

    #[tokio::test]
    async fn test_cleanup_does_nothing_when_no_old_captures() {
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400));

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Only add new captures
        storage.add_capture(now - 3600, "/new1.png");
        storage.add_capture(now - 1800, "/new2.png");

        let manager = RetentionManager::new(Arc::clone(&storage), config);
        let summary = manager.cleanup_now().await.unwrap();

        assert_eq!(summary.deleted_count, 0);
        assert_eq!(summary.failed_count, 0);
        assert_eq!(storage.capture_count(), 2);
    }

    // === Manager Lifecycle Tests ===

    #[tokio::test]
    async fn test_manager_creation() {
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400));

        let manager = RetentionManager::new(storage, config);

        assert!(!manager.is_running());
    }

    #[tokio::test]
    async fn test_manager_start_stop() {
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400));

        let manager = RetentionManager::with_interval(storage, config, Duration::from_millis(100));

        // Start
        manager.start().await.unwrap();
        assert!(manager.is_running());

        // Wait a bit
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Stop
        manager.stop().await.unwrap();
        assert!(!manager.is_running());
    }

    #[tokio::test]
    async fn test_manager_double_start_fails() {
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400));

        let manager = RetentionManager::with_interval(storage, config, Duration::from_millis(100));

        manager.start().await.unwrap();

        let result = manager.start().await;
        assert!(matches!(result, Err(RetentionError::AlreadyRunning)));

        manager.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_manager_stop_when_not_running_fails() {
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400));

        let manager = RetentionManager::new(storage, config);

        let result = manager.stop().await;
        assert!(matches!(result, Err(RetentionError::NotRunning)));
    }

    #[tokio::test]
    async fn test_periodic_cleanup_executes() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(86400));

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Create old file
        let old_file = temp_dir.path().join("old.png");
        fs::write(&old_file, b"test").await.unwrap();
        storage.add_capture(now - 90000, old_file.to_str().unwrap());

        // Use very short interval for testing
        let manager = RetentionManager::with_interval(
            Arc::clone(&storage),
            config,
            Duration::from_millis(50),
        );

        manager.start().await.unwrap();

        // Wait for at least one cleanup cycle
        tokio::time::sleep(Duration::from_millis(100)).await;

        manager.stop().await.unwrap();

        // Should have cleaned up the old capture
        assert!(!old_file.exists());
        assert_eq!(storage.capture_count(), 0);
    }

    // === cleanup_once function tests ===

    #[tokio::test]
    async fn test_cleanup_once_standalone() {
        let temp_dir = TempDir::new().unwrap();
        let storage = MockStoragePort::new();
        let config = create_test_config(86400);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let old_file = temp_dir.path().join("old.png");
        fs::write(&old_file, b"test").await.unwrap();
        storage.add_capture(now - 90000, old_file.to_str().unwrap());

        let summary = cleanup_once(&storage, &config).await.unwrap();

        assert_eq!(summary.deleted_count, 1);
        assert!(!old_file.exists());
    }

    // === Error display tests ===

    #[test]
    fn test_retention_error_display() {
        let err = RetentionError::FileDeletionFailed("permission denied".to_string());
        assert!(err.to_string().contains("permission denied"));

        let err = RetentionError::AlreadyRunning;
        assert!(err.to_string().contains("already running"));

        let err = RetentionError::NotRunning;
        assert!(err.to_string().contains("not running"));
    }

    #[test]
    fn test_cleanup_summary_default() {
        let summary = CleanupSummary::default();
        assert_eq!(summary.deleted_count, 0);
        assert_eq!(summary.failed_count, 0);
        assert_eq!(summary.freed_bytes, 0);
    }
}
