//! Capture scheduler for periodic screen capture
//!
//! Manages automatic screen capture at configurable intervals using Tokio timers.

use crate::config::Config;
use crate::ports::capture::{CaptureError, CapturePort};
use crate::ports::storage::{CaptureMetadata, StorageError, StoragePort};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use tokio::sync::Notify;
use tracing::{debug, error, info, warn};

/// Result of a successful capture operation
#[derive(Debug, Clone)]
pub struct CaptureResult {
    /// Path to the saved image file
    pub file_path: PathBuf,
    /// Unix timestamp of capture
    pub timestamp: i64,
    /// Size of the saved file in bytes
    pub size_bytes: u64,
}

/// Errors that can occur during scheduler operations
#[derive(Debug, Error)]
pub enum SchedulerError {
    /// Capture operation failed
    #[error("Capture failed: {0}")]
    CaptureFailed(String),

    /// Storage operation failed
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),

    /// IO error during file operations
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Permission denied for screen recording
    #[error("Screen recording permission denied")]
    PermissionDenied,

    /// Scheduler is already running
    #[error("Scheduler is already running")]
    AlreadyRunning,

    /// Scheduler is not running
    #[error("Scheduler is not running")]
    NotRunning,
}

impl From<CaptureError> for SchedulerError {
    fn from(err: CaptureError) -> Self {
        match err {
            CaptureError::PermissionDenied => SchedulerError::PermissionDenied,
            other => SchedulerError::CaptureFailed(other.to_string()),
        }
    }
}

/// Capture scheduler for managing periodic screen captures
///
/// The scheduler runs in a background Tokio task and captures screenshots
/// at the configured interval. It supports graceful shutdown and tracks
/// consecutive failures for error reporting.
pub struct CaptureScheduler<C, S>
where
    C: CapturePort + 'static,
    S: StoragePort + 'static,
{
    capture_port: Arc<C>,
    storage_port: Arc<S>,
    config: Arc<Config>,
    /// Flag to indicate if scheduler is running
    running: Arc<AtomicBool>,
    /// Signal to stop the scheduler
    stop_signal: Arc<Notify>,
    /// Counter for consecutive failures
    consecutive_failures: Arc<AtomicU64>,
    /// Maximum consecutive failures before warning
    max_consecutive_failures: u64,
}

impl<C, S> CaptureScheduler<C, S>
where
    C: CapturePort + 'static,
    S: StoragePort + 'static,
{
    /// Creates a new capture scheduler
    ///
    /// # Arguments
    /// * `capture_port` - The capture port implementation
    /// * `storage_port` - The storage port implementation
    /// * `config` - Application configuration
    pub fn new(capture_port: Arc<C>, storage_port: Arc<S>, config: Arc<Config>) -> Self {
        Self {
            capture_port,
            storage_port,
            config,
            running: Arc::new(AtomicBool::new(false)),
            stop_signal: Arc::new(Notify::new()),
            consecutive_failures: Arc::new(AtomicU64::new(0)),
            max_consecutive_failures: 5,
        }
    }

    /// Returns whether the scheduler is currently running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Returns the number of consecutive capture failures
    pub fn consecutive_failures(&self) -> u64 {
        self.consecutive_failures.load(Ordering::SeqCst)
    }

    /// Starts the periodic capture loop
    ///
    /// This spawns a background task that captures screenshots at the
    /// configured interval. The task will run until `stop()` is called.
    ///
    /// # Errors
    /// Returns `SchedulerError::AlreadyRunning` if scheduler is already active
    pub async fn start(&self) -> Result<(), SchedulerError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(SchedulerError::AlreadyRunning);
        }

        let capture_port = Arc::clone(&self.capture_port);
        let storage_port = Arc::clone(&self.storage_port);
        let config = Arc::clone(&self.config);
        let running = Arc::clone(&self.running);
        let stop_signal = Arc::clone(&self.stop_signal);
        let consecutive_failures = Arc::clone(&self.consecutive_failures);
        let max_failures = self.max_consecutive_failures;

        tokio::spawn(async move {
            let interval = Duration::from_secs(config.capture.interval_seconds);
            info!("Starting periodic capture with interval: {:?}", interval);

            loop {
                tokio::select! {
                    _ = stop_signal.notified() => {
                        info!("Received stop signal, shutting down scheduler");
                        break;
                    }
                    _ = tokio::time::sleep(interval) => {
                        match Self::execute_capture(&capture_port, &storage_port, &config).await {
                            Ok(result) => {
                                consecutive_failures.store(0, Ordering::SeqCst);
                                info!(
                                    "Capture successful: {} ({} bytes)",
                                    result.file_path.display(),
                                    result.size_bytes
                                );
                            }
                            Err(e) => {
                                let failures = consecutive_failures.fetch_add(1, Ordering::SeqCst) + 1;
                                error!("Capture failed: {}", e);

                                if failures >= max_failures {
                                    warn!(
                                        "Capture has failed {} consecutive times",
                                        failures
                                    );
                                }
                            }
                        }
                    }
                }
            }

            running.store(false, Ordering::SeqCst);
            info!("Scheduler stopped");
        });

        Ok(())
    }

    /// Stops the periodic capture loop
    ///
    /// Sends a stop signal to the background task and waits for it to finish.
    ///
    /// # Errors
    /// Returns `SchedulerError::NotRunning` if scheduler is not active
    pub async fn stop(&self) -> Result<(), SchedulerError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(SchedulerError::NotRunning);
        }

        info!("Stopping scheduler...");
        self.stop_signal.notify_one();

        // Wait for the scheduler to stop
        while self.running.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }

    /// Executes a single capture immediately
    ///
    /// This can be called independently of the periodic capture loop.
    ///
    /// # Returns
    /// `CaptureResult` containing the file path, timestamp, and size
    pub async fn capture_now(&self) -> Result<CaptureResult, SchedulerError> {
        Self::execute_capture(&self.capture_port, &self.storage_port, &self.config).await
    }

    /// Internal method to execute a single capture
    async fn execute_capture(
        capture_port: &Arc<C>,
        storage_port: &Arc<S>,
        config: &Arc<Config>,
    ) -> Result<CaptureResult, SchedulerError> {
        let start = std::time::Instant::now();
        debug!("Starting capture...");

        // Capture screen
        let image = capture_port.capture_screen().await?;
        debug!("Screen captured in {:?}", start.elapsed());

        // Generate filename with timestamp
        let timestamp = image.timestamp;
        let datetime = chrono_format_timestamp(timestamp);
        let filename = format!("{}.png", datetime);

        // Determine save path
        let captures_dir = config.storage.data_dir.join("captures");
        let file_path = captures_dir.join(&filename);

        // Ensure captures directory exists
        fs::create_dir_all(&captures_dir).await?;

        // Save image file
        fs::write(&file_path, &image.data).await?;
        let size_bytes = image.data.len() as u64;
        debug!("Image saved to {:?}", file_path);

        // Save metadata to database
        let metadata = CaptureMetadata {
            id: None,
            timestamp,
            file_path: file_path.to_string_lossy().to_string(),
            created_at: timestamp,
        };
        storage_port.save_metadata(metadata).await?;
        debug!("Metadata saved to database");

        let elapsed = start.elapsed();
        if elapsed > Duration::from_secs(1) {
            warn!("Capture took {:?}, exceeds 1 second target", elapsed);
        }

        Ok(CaptureResult {
            file_path,
            timestamp,
            size_bytes,
        })
    }
}

/// Performs a single capture without requiring a scheduler instance
///
/// This is a convenience function for one-shot captures, such as the
/// `herald capture` CLI command. It works independently of any running
/// scheduler and does not affect periodic capture timing.
///
/// # Arguments
/// * `capture_port` - The capture port implementation
/// * `storage_port` - The storage port implementation
/// * `config` - Application configuration
///
/// # Returns
/// `CaptureResult` containing the file path, timestamp, and size
///
/// # Example
/// ```ignore
/// let result = capture_once(&capture_port, &storage_port, &config).await?;
/// println!("Captured: {} at {}", result.file_path.display(), result.timestamp);
/// ```
pub async fn capture_once<C, S>(
    capture_port: &C,
    storage_port: &S,
    config: &Config,
) -> Result<CaptureResult, SchedulerError>
where
    C: CapturePort,
    S: StoragePort,
{
    let start = std::time::Instant::now();
    debug!("Starting manual capture...");

    // Capture screen
    let image = capture_port.capture_screen().await?;
    debug!("Screen captured in {:?}", start.elapsed());

    // Generate filename with timestamp
    let timestamp = image.timestamp;
    let datetime = chrono_format_timestamp(timestamp);
    let filename = format!("{}.png", datetime);

    // Determine save path
    let captures_dir = config.storage.data_dir.join("captures");
    let file_path = captures_dir.join(&filename);

    // Ensure captures directory exists
    fs::create_dir_all(&captures_dir).await?;

    // Save image file
    fs::write(&file_path, &image.data).await?;
    let size_bytes = image.data.len() as u64;
    debug!("Image saved to {:?}", file_path);

    // Save metadata to database
    let metadata = CaptureMetadata {
        id: None,
        timestamp,
        file_path: file_path.to_string_lossy().to_string(),
        created_at: timestamp,
    };
    storage_port.save_metadata(metadata).await?;
    debug!("Metadata saved to database");

    let elapsed = start.elapsed();
    info!(
        "Manual capture completed: {} ({} bytes) in {:?}",
        file_path.display(),
        size_bytes,
        elapsed
    );

    Ok(CaptureResult {
        file_path,
        timestamp,
        size_bytes,
    })
}

/// Format Unix timestamp as YYYY-MM-DD_HH-mm-ss (UTC)
fn chrono_format_timestamp(timestamp: i64) -> String {
    // Convert to UTC time components
    // For simplicity, we implement manual calculation here.
    // In production, consider using chrono crate for timezone handling.
    let secs = timestamp;
    let days = secs / 86400;
    let remaining = secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    // Simple date calculation (approximate, good enough for filenames)
    // This is a simplified calculation - for production, use chrono
    let mut year = 1970i64;
    let mut remaining_days = days;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let days_in_months: [i64; 12] = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u32;
    for days_in_month in days_in_months.iter() {
        if remaining_days < *days_in_month {
            break;
        }
        remaining_days -= days_in_month;
        month += 1;
    }

    let day = remaining_days + 1;

    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        year, month, day, hours, minutes, seconds
    )
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::capture::CapturedImage;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Mock capture port for testing
    struct MockCapturePort {
        should_fail: AtomicBool,
        capture_count: AtomicUsize,
        fail_count: AtomicUsize,
    }

    impl MockCapturePort {
        fn new() -> Self {
            Self {
                should_fail: AtomicBool::new(false),
                capture_count: AtomicUsize::new(0),
                fail_count: AtomicUsize::new(0),
            }
        }

        fn set_should_fail(&self, fail: bool) {
            self.should_fail.store(fail, Ordering::SeqCst);
        }

        fn capture_count(&self) -> usize {
            self.capture_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl CapturePort for MockCapturePort {
        async fn capture_screen(&self) -> Result<CapturedImage, CaptureError> {
            self.capture_count.fetch_add(1, Ordering::SeqCst);

            if self.should_fail.load(Ordering::SeqCst) {
                self.fail_count.fetch_add(1, Ordering::SeqCst);
                return Err(CaptureError::CaptureFailed("Mock failure".to_string()));
            }

            // Return a small valid PNG-like data
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            Ok(CapturedImage {
                data: vec![0u8; 100], // Minimal test data
                width: 100,
                height: 100,
                timestamp,
            })
        }

        async fn check_permission(&self) -> Result<bool, CaptureError> {
            Ok(true)
        }
    }

    // Mock storage port for testing
    struct MockStoragePort {
        saved_metadata: Mutex<Vec<CaptureMetadata>>,
        should_fail: AtomicBool,
    }

    impl MockStoragePort {
        fn new() -> Self {
            Self {
                saved_metadata: Mutex::new(Vec::new()),
                should_fail: AtomicBool::new(false),
            }
        }

        fn saved_count(&self) -> usize {
            self.saved_metadata.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl StoragePort for MockStoragePort {
        async fn save_metadata(&self, metadata: CaptureMetadata) -> Result<(), StorageError> {
            if self.should_fail.load(Ordering::SeqCst) {
                return Err(StorageError::DatabaseError("Mock failure".to_string()));
            }
            self.saved_metadata.lock().unwrap().push(metadata);
            Ok(())
        }

        async fn get_recent_captures(
            &self,
            limit: usize,
        ) -> Result<Vec<CaptureMetadata>, StorageError> {
            let metadata = self.saved_metadata.lock().unwrap();
            Ok(metadata.iter().take(limit).cloned().collect())
        }

        async fn get_old_captures(
            &self,
            _cutoff: i64,
        ) -> Result<Vec<CaptureMetadata>, StorageError> {
            Ok(vec![])
        }

        async fn delete_capture(&self, _id: i64) -> Result<(), StorageError> {
            Ok(())
        }

        async fn get_statistics(
            &self,
        ) -> Result<crate::ports::storage::StorageStatistics, StorageError> {
            Ok(crate::ports::storage::StorageStatistics {
                total_captures: self.saved_metadata.lock().unwrap().len(),
                total_size_bytes: 0,
                oldest_timestamp: None,
                newest_timestamp: None,
            })
        }
    }

    fn create_test_config(interval_seconds: u64) -> Config {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir().join(format!(
            "herald_test_{}_{}_{}",
            std::process::id(),
            unique_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut config = Config::default();
        config.capture.interval_seconds = interval_seconds;
        config.storage.data_dir = temp_dir;
        config
    }

    // === Task 7.1 Tests: Periodic Capture ===

    #[tokio::test]
    async fn test_scheduler_creation() {
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(60));

        let scheduler = CaptureScheduler::new(capture, storage, config);

        assert!(!scheduler.is_running());
        assert_eq!(scheduler.consecutive_failures(), 0);
    }

    #[tokio::test]
    async fn test_capture_now_saves_file_and_metadata() {
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(60));

        let scheduler = CaptureScheduler::new(
            Arc::clone(&capture),
            Arc::clone(&storage),
            Arc::clone(&config),
        );

        let result = scheduler.capture_now().await;
        assert!(result.is_ok());

        let capture_result = result.unwrap();
        assert!(capture_result.file_path.exists());
        assert!(capture_result.size_bytes > 0);
        assert_eq!(storage.saved_count(), 1);
        assert_eq!(capture.capture_count(), 1);

        // Cleanup
        let _ = std::fs::remove_file(&capture_result.file_path);
    }

    #[tokio::test]
    async fn test_capture_filename_format() {
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(60));

        let scheduler = CaptureScheduler::new(capture, storage, config);

        let result = scheduler.capture_now().await.unwrap();

        // Filename should be YYYY-MM-DD_HH-mm-ss.png
        let filename = result.file_path.file_name().unwrap().to_string_lossy();
        assert!(filename.ends_with(".png"));
        // Check format: should contain underscores and hyphens in right places
        let name_without_ext = filename.trim_end_matches(".png");
        assert_eq!(name_without_ext.len(), 19); // YYYY-MM-DD_HH-mm-ss
        assert_eq!(&name_without_ext[4..5], "-");
        assert_eq!(&name_without_ext[7..8], "-");
        assert_eq!(&name_without_ext[10..11], "_");

        // Cleanup
        let _ = std::fs::remove_file(&result.file_path);
    }

    #[tokio::test]
    async fn test_scheduler_start_stop() {
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(1)); // 1 second interval for fast test

        let scheduler = CaptureScheduler::new(capture, storage, config);

        // Start scheduler
        let result = scheduler.start().await;
        assert!(result.is_ok());
        assert!(scheduler.is_running());

        // Wait briefly for at least one capture
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // Stop scheduler
        let result = scheduler.stop().await;
        assert!(result.is_ok());
        assert!(!scheduler.is_running());
    }

    #[tokio::test]
    async fn test_scheduler_double_start_fails() {
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(60));

        let scheduler = CaptureScheduler::new(capture, storage, config);

        // First start should succeed
        scheduler.start().await.unwrap();

        // Second start should fail
        let result = scheduler.start().await;
        assert!(matches!(result, Err(SchedulerError::AlreadyRunning)));

        // Cleanup
        let _ = scheduler.stop().await;
    }

    #[tokio::test]
    async fn test_scheduler_stop_when_not_running_fails() {
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(60));

        let scheduler = CaptureScheduler::new(capture, storage, config);

        let result = scheduler.stop().await;
        assert!(matches!(result, Err(SchedulerError::NotRunning)));
    }

    // === Task 7.2 Tests: Error Handling ===

    #[tokio::test]
    async fn test_capture_failure_logs_error_and_continues() {
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(1));

        capture.set_should_fail(true);

        let scheduler = CaptureScheduler::new(
            Arc::clone(&capture),
            Arc::clone(&storage),
            Arc::clone(&config),
        );

        scheduler.start().await.unwrap();

        // Wait for a few capture attempts
        tokio::time::sleep(Duration::from_millis(2500)).await;

        // Consecutive failures should be tracked
        assert!(scheduler.consecutive_failures() > 0);

        scheduler.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_capture_failure_resets_on_success() {
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(1));

        let scheduler = CaptureScheduler::new(
            Arc::clone(&capture),
            Arc::clone(&storage),
            Arc::clone(&config),
        );

        // Start with failures
        capture.set_should_fail(true);
        scheduler.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;

        assert!(scheduler.consecutive_failures() > 0);

        // Switch to success
        capture.set_should_fail(false);
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // Failures should be reset
        assert_eq!(scheduler.consecutive_failures(), 0);

        scheduler.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_capture_now_returns_error_on_failure() {
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(60));

        capture.set_should_fail(true);

        let scheduler = CaptureScheduler::new(capture, storage, config);

        let result = scheduler.capture_now().await;
        assert!(result.is_err());
        assert!(matches!(result, Err(SchedulerError::CaptureFailed(_))));
    }

    #[tokio::test]
    async fn test_storage_error_propagates() {
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(60));

        storage.should_fail.store(true, Ordering::SeqCst);

        let scheduler = CaptureScheduler::new(capture, storage, config);

        let result = scheduler.capture_now().await;
        assert!(result.is_err());
        assert!(matches!(result, Err(SchedulerError::StorageError(_))));
    }

    // === Timestamp formatting tests ===

    #[test]
    fn test_timestamp_format() {
        // Test known timestamp: 2024-01-04 00:00:00 UTC
        let timestamp: i64 = 1704326400;
        let formatted = chrono_format_timestamp(timestamp);
        assert_eq!(formatted, "2024-01-04_00-00-00");
    }

    #[test]
    fn test_timestamp_format_with_time() {
        // Test: 2024-06-15 13:30:45 UTC (1718458245 seconds since epoch)
        let timestamp: i64 = 1718458245;
        let formatted = chrono_format_timestamp(timestamp);
        assert_eq!(formatted, "2024-06-15_13-30-45");
    }

    #[test]
    fn test_leap_year() {
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2023));
        assert!(!is_leap_year(1900));
    }

    // === Error conversion tests ===

    #[test]
    fn test_scheduler_error_from_capture_error() {
        let err: SchedulerError = CaptureError::PermissionDenied.into();
        assert!(matches!(err, SchedulerError::PermissionDenied));

        let err: SchedulerError = CaptureError::CaptureFailed("test".to_string()).into();
        assert!(matches!(err, SchedulerError::CaptureFailed(_)));
    }

    #[test]
    fn test_scheduler_error_from_storage_error() {
        let err: SchedulerError = StorageError::DatabaseError("test".to_string()).into();
        assert!(matches!(err, SchedulerError::StorageError(_)));
    }

    // === Task 8.1 Tests: Manual Capture ===

    #[tokio::test]
    async fn test_capture_once_standalone_function() {
        // Test that capture_once works independently without scheduler
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(60));

        let result = capture_once(&*capture, &*storage, &config).await;
        assert!(result.is_ok());

        let capture_result = result.unwrap();
        assert!(capture_result.file_path.exists());
        assert!(capture_result.timestamp > 0);
        assert!(capture_result.size_bytes > 0);
        assert_eq!(storage.saved_count(), 1);

        // Cleanup
        let _ = std::fs::remove_file(&capture_result.file_path);
    }

    #[tokio::test]
    async fn test_capture_once_returns_correct_path_and_timestamp() {
        // Verify that manual capture returns file path and timestamp correctly
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(60));

        let before_capture = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = capture_once(&*capture, &*storage, &config).await.unwrap();

        let after_capture = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Timestamp should be within the capture window
        assert!(result.timestamp >= before_capture);
        assert!(result.timestamp <= after_capture);

        // Path should be in the captures directory
        assert!(result.file_path.to_string_lossy().contains("captures"));
        assert!(result.file_path.extension().map_or(false, |e| e == "png"));

        // Cleanup
        let _ = std::fs::remove_file(&result.file_path);
    }

    #[tokio::test]
    async fn test_manual_capture_independent_of_scheduler_state() {
        // Manual capture should work regardless of scheduler running state
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(60));

        // Capture before scheduler exists
        let result1 = capture_once(&*capture, &*storage, &config).await;
        assert!(result1.is_ok());

        // Create scheduler but don't start it
        let scheduler = CaptureScheduler::new(
            Arc::clone(&capture),
            Arc::clone(&storage),
            Arc::clone(&config),
        );
        assert!(!scheduler.is_running());

        // Capture with scheduler not running
        let result2 = capture_once(&*capture, &*storage, &config).await;
        assert!(result2.is_ok());

        // Start scheduler
        scheduler.start().await.unwrap();
        assert!(scheduler.is_running());

        // Capture with scheduler running
        let result3 = capture_once(&*capture, &*storage, &config).await;
        assert!(result3.is_ok());

        // All three captures should have succeeded
        assert_eq!(capture.capture_count(), 3);

        // Cleanup
        scheduler.stop().await.unwrap();
        let _ = std::fs::remove_file(&result1.unwrap().file_path);
        let _ = std::fs::remove_file(&result2.unwrap().file_path);
        let _ = std::fs::remove_file(&result3.unwrap().file_path);
    }

    #[tokio::test]
    async fn test_manual_capture_does_not_affect_scheduler_timing() {
        // Manual capture should not reset or affect the periodic capture interval
        let capture = Arc::new(MockCapturePort::new());
        let storage = Arc::new(MockStoragePort::new());
        let config = Arc::new(create_test_config(1)); // 1 second interval

        let scheduler = CaptureScheduler::new(
            Arc::clone(&capture),
            Arc::clone(&storage),
            Arc::clone(&config),
        );

        // Start scheduler
        scheduler.start().await.unwrap();

        // Wait for first scheduled capture
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let count_after_first = capture.capture_count();
        assert!(
            count_after_first >= 1,
            "Should have at least 1 scheduled capture"
        );

        // Do a manual capture
        let manual_result = capture_once(&*capture, &*storage, &config).await;
        assert!(manual_result.is_ok());
        let count_after_manual = capture.capture_count();
        assert_eq!(
            count_after_manual,
            count_after_first + 1,
            "Manual capture should add exactly 1 to count"
        );

        // Wait for another scheduled capture - it should still happen on schedule
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let count_after_second = capture.capture_count();
        assert!(
            count_after_second >= count_after_manual + 1,
            "Scheduler should continue normally after manual capture"
        );

        // Cleanup
        scheduler.stop().await.unwrap();
        let _ = std::fs::remove_file(&manual_result.unwrap().file_path);
    }
}
