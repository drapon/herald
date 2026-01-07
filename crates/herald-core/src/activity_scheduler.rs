//! Activity analysis scheduler for periodic activity extraction
//!
//! Manages automatic activity analysis at configurable intervals using Tokio timers.

use crate::config::Config;
use crate::ports::activity::{
    Activity, ActivityAnalyzerPort, ActivityStorageError, ActivityStoragePort, AnalysisError,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Notify;
use tracing::{debug, error, info, warn};

/// Maximum number of captures to analyze in a single batch
const DEFAULT_BATCH_SIZE: usize = 10;

/// Errors that can occur during activity analysis scheduler operations
#[derive(Debug, Error)]
pub enum ActivitySchedulerError {
    /// Analysis operation failed
    #[error("Analysis failed: {0}")]
    AnalysisFailed(String),

    /// Storage operation failed
    #[error("Storage error: {0}")]
    StorageError(String),

    /// Scheduler is already running
    #[error("Scheduler is already running")]
    AlreadyRunning,

    /// Scheduler is not running
    #[error("Scheduler is not running")]
    NotRunning,

    /// Activity feature is disabled
    #[error("Activity feature is disabled in config")]
    Disabled,
}

impl From<ActivityStorageError> for ActivitySchedulerError {
    fn from(err: ActivityStorageError) -> Self {
        ActivitySchedulerError::StorageError(err.to_string())
    }
}

impl From<AnalysisError> for ActivitySchedulerError {
    fn from(err: AnalysisError) -> Self {
        ActivitySchedulerError::AnalysisFailed(err.to_string())
    }
}

/// Result of an analysis batch operation
#[derive(Debug, Clone)]
pub struct AnalysisBatchResult {
    /// Number of captures successfully analyzed
    pub success_count: usize,
    /// Number of captures that failed analysis
    pub failure_count: usize,
}

/// Activity analysis scheduler for managing periodic activity extraction
///
/// The scheduler runs in a background Tokio task and analyzes screenshots
/// at the configured interval. It supports graceful shutdown and tracks
/// consecutive failures for error reporting.
pub struct ActivityAnalysisScheduler<A, S>
where
    A: ActivityAnalyzerPort + 'static,
    S: ActivityStoragePort + 'static,
{
    analyzer: Arc<A>,
    storage: Arc<S>,
    config: Arc<Config>,
    /// Flag to indicate if scheduler is running
    running: Arc<AtomicBool>,
    /// Signal to stop the scheduler
    stop_signal: Arc<Notify>,
    /// Counter for consecutive failures
    consecutive_failures: Arc<AtomicU64>,
    /// Maximum consecutive failures before warning
    max_consecutive_failures: u64,
    /// Batch size for analysis
    batch_size: usize,
}

impl<A, S> ActivityAnalysisScheduler<A, S>
where
    A: ActivityAnalyzerPort + 'static,
    S: ActivityStoragePort + 'static,
{
    /// Creates a new activity analysis scheduler
    ///
    /// # Arguments
    /// * `analyzer` - The activity analyzer implementation
    /// * `storage` - The activity storage implementation
    /// * `config` - Application configuration
    pub fn new(analyzer: Arc<A>, storage: Arc<S>, config: Arc<Config>) -> Self {
        Self {
            analyzer,
            storage,
            config,
            running: Arc::new(AtomicBool::new(false)),
            stop_signal: Arc::new(Notify::new()),
            consecutive_failures: Arc::new(AtomicU64::new(0)),
            max_consecutive_failures: 5,
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }

    /// Sets the batch size for analysis operations
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Returns whether the scheduler is currently running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Returns the number of consecutive analysis failures
    pub fn consecutive_failures(&self) -> u64 {
        self.consecutive_failures.load(Ordering::SeqCst)
    }

    /// Starts the periodic analysis loop
    ///
    /// This spawns a background task that analyzes screenshots at the
    /// configured interval. The task will run until `stop()` is called.
    ///
    /// # Errors
    /// - Returns `ActivitySchedulerError::AlreadyRunning` if scheduler is already active
    /// - Returns `ActivitySchedulerError::Disabled` if activity feature is disabled
    pub async fn start(&self) -> Result<(), ActivitySchedulerError> {
        // Check if activity feature is enabled
        if !self.config.activity.enabled {
            return Err(ActivitySchedulerError::Disabled);
        }

        if self.running.swap(true, Ordering::SeqCst) {
            return Err(ActivitySchedulerError::AlreadyRunning);
        }

        let analyzer = Arc::clone(&self.analyzer);
        let storage = Arc::clone(&self.storage);
        let config = Arc::clone(&self.config);
        let running = Arc::clone(&self.running);
        let stop_signal = Arc::clone(&self.stop_signal);
        let consecutive_failures = Arc::clone(&self.consecutive_failures);
        let max_failures = self.max_consecutive_failures;
        let batch_size = self.batch_size;

        tokio::spawn(async move {
            let interval = Duration::from_secs(config.activity.analyze_interval_seconds);
            info!(
                "Starting activity analysis scheduler with interval: {:?}",
                interval
            );

            loop {
                tokio::select! {
                    _ = stop_signal.notified() => {
                        info!("Received stop signal, shutting down activity scheduler");
                        break;
                    }
                    _ = tokio::time::sleep(interval) => {
                        match Self::execute_analysis_batch(&analyzer, &storage, batch_size).await {
                            Ok(result) => {
                                if result.success_count > 0 || result.failure_count > 0 {
                                    info!(
                                        "Activity analysis batch completed: {} successful, {} failed",
                                        result.success_count, result.failure_count
                                    );
                                }

                                if result.failure_count == 0 {
                                    consecutive_failures.store(0, Ordering::SeqCst);
                                } else {
                                    let failures = consecutive_failures.fetch_add(1, Ordering::SeqCst) + 1;
                                    if failures >= max_failures {
                                        warn!(
                                            "Activity analysis has failed {} consecutive times",
                                            failures
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                let failures = consecutive_failures.fetch_add(1, Ordering::SeqCst) + 1;
                                error!("Activity analysis batch failed: {}", e);

                                if failures >= max_failures {
                                    warn!(
                                        "Activity analysis has failed {} consecutive times",
                                        failures
                                    );
                                }
                            }
                        }
                    }
                }
            }

            running.store(false, Ordering::SeqCst);
            info!("Activity analysis scheduler stopped");
        });

        Ok(())
    }

    /// Stops the periodic analysis loop
    ///
    /// Sends a stop signal to the background task and waits for it to finish.
    ///
    /// # Errors
    /// Returns `ActivitySchedulerError::NotRunning` if scheduler is not active
    pub async fn stop(&self) -> Result<(), ActivitySchedulerError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(ActivitySchedulerError::NotRunning);
        }

        info!("Stopping activity analysis scheduler...");
        self.stop_signal.notify_one();

        // Wait for the scheduler to stop
        while self.running.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }

    /// Executes a single analysis batch immediately
    ///
    /// This can be called independently of the periodic analysis loop.
    ///
    /// # Returns
    /// `AnalysisBatchResult` containing success and failure counts
    pub async fn analyze_now(&self) -> Result<AnalysisBatchResult, ActivitySchedulerError> {
        Self::execute_analysis_batch(&self.analyzer, &self.storage, self.batch_size).await
    }

    /// Internal method to execute an analysis batch
    async fn execute_analysis_batch(
        analyzer: &Arc<A>,
        storage: &Arc<S>,
        batch_size: usize,
    ) -> Result<AnalysisBatchResult, ActivitySchedulerError> {
        debug!("Fetching unanalyzed captures...");

        // Get unanalyzed captures
        let captures = storage.get_unanalyzed_captures(batch_size).await?;

        if captures.is_empty() {
            debug!("No unanalyzed captures found");
            return Ok(AnalysisBatchResult {
                success_count: 0,
                failure_count: 0,
            });
        }

        debug!("Found {} unanalyzed captures", captures.len());

        // Analyze captures
        let results = analyzer.analyze_batch(captures).await;

        let mut success_count = 0;
        let mut failure_count = 0;

        // Process results and save activities
        for result in results {
            match result {
                Ok((capture_id, analysis_result)) => {
                    let activity = Activity {
                        id: None,
                        capture_id,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs() as i64,
                        application_name: analysis_result.application_name,
                        activity_description: analysis_result.activity_description,
                        category: analysis_result.category,
                        created_at: 0, // Will be set by storage
                    };

                    match storage.save_activity(activity).await {
                        Ok(_) => {
                            success_count += 1;
                            debug!("Activity saved for capture {}", capture_id);
                        }
                        Err(e) => {
                            failure_count += 1;
                            error!("Failed to save activity for capture {}: {}", capture_id, e);
                        }
                    }
                }
                Err(e) => {
                    failure_count += 1;
                    warn!("Analysis failed: {}", e);
                }
            }
        }

        Ok(AnalysisBatchResult {
            success_count,
            failure_count,
        })
    }
}

/// Performs a single analysis batch without requiring a scheduler instance
///
/// This is a convenience function for one-shot analysis operations.
///
/// # Arguments
/// * `analyzer` - The activity analyzer implementation
/// * `storage` - The activity storage implementation
/// * `batch_size` - Maximum number of captures to analyze
///
/// # Returns
/// `AnalysisBatchResult` containing success and failure counts
pub async fn analyze_once<A, S>(
    analyzer: &A,
    storage: &S,
    batch_size: usize,
) -> Result<AnalysisBatchResult, ActivitySchedulerError>
where
    A: ActivityAnalyzerPort,
    S: ActivityStoragePort,
{
    debug!("Starting manual activity analysis batch...");

    // Get unanalyzed captures
    let captures = storage.get_unanalyzed_captures(batch_size).await?;

    if captures.is_empty() {
        debug!("No unanalyzed captures found");
        return Ok(AnalysisBatchResult {
            success_count: 0,
            failure_count: 0,
        });
    }

    debug!("Found {} unanalyzed captures", captures.len());

    // Analyze captures
    let results = analyzer.analyze_batch(captures).await;

    let mut success_count = 0;
    let mut failure_count = 0;

    // Process results and save activities
    for result in results {
        match result {
            Ok((capture_id, analysis_result)) => {
                let activity = Activity {
                    id: None,
                    capture_id,
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64,
                    application_name: analysis_result.application_name,
                    activity_description: analysis_result.activity_description,
                    category: analysis_result.category,
                    created_at: 0, // Will be set by storage
                };

                match storage.save_activity(activity).await {
                    Ok(_) => {
                        success_count += 1;
                        debug!("Activity saved for capture {}", capture_id);
                    }
                    Err(e) => {
                        failure_count += 1;
                        error!("Failed to save activity for capture {}: {}", capture_id, e);
                    }
                }
            }
            Err(e) => {
                failure_count += 1;
                warn!("Analysis failed: {}", e);
            }
        }
    }

    info!(
        "Manual analysis completed: {} successful, {} failed",
        success_count, failure_count
    );

    Ok(AnalysisBatchResult {
        success_count,
        failure_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::activity::{ActivityCategory, AnalysisResult, CaptureForAnalysis};
    use async_trait::async_trait;
    use std::sync::Mutex;

    // Mock analyzer for testing
    struct MockAnalyzer {
        should_fail: AtomicBool,
        analyze_count: AtomicU64,
    }

    impl MockAnalyzer {
        fn new() -> Self {
            Self {
                should_fail: AtomicBool::new(false),
                analyze_count: AtomicU64::new(0),
            }
        }

        fn set_should_fail(&self, fail: bool) {
            self.should_fail.store(fail, Ordering::SeqCst);
        }

        fn analyze_count(&self) -> u64 {
            self.analyze_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ActivityAnalyzerPort for MockAnalyzer {
        async fn analyze(
            &self,
            capture: CaptureForAnalysis,
        ) -> Result<AnalysisResult, AnalysisError> {
            self.analyze_count.fetch_add(1, Ordering::SeqCst);

            if self.should_fail.load(Ordering::SeqCst) {
                return Err(AnalysisError::AIError("Mock failure".to_string()));
            }

            Ok(AnalysisResult {
                application_name: "Test App".to_string(),
                activity_description: format!("Testing capture {}", capture.id),
                category: ActivityCategory::Coding,
            })
        }

        async fn analyze_batch(
            &self,
            captures: Vec<CaptureForAnalysis>,
        ) -> Vec<Result<(i64, AnalysisResult), AnalysisError>> {
            let mut results = Vec::new();
            for capture in captures {
                let id = capture.id;
                match self.analyze(capture).await {
                    Ok(result) => results.push(Ok((id, result))),
                    Err(e) => results.push(Err(e)),
                }
            }
            results
        }
    }

    // Mock storage for testing
    struct MockStorage {
        captures: Mutex<Vec<CaptureForAnalysis>>,
        activities: Mutex<Vec<Activity>>,
        should_fail: AtomicBool,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                captures: Mutex::new(Vec::new()),
                activities: Mutex::new(Vec::new()),
                should_fail: AtomicBool::new(false),
            }
        }

        fn add_capture(&self, capture: CaptureForAnalysis) {
            self.captures.lock().unwrap().push(capture);
        }

        fn activity_count(&self) -> usize {
            self.activities.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl ActivityStoragePort for MockStorage {
        async fn save_activity(&self, activity: Activity) -> Result<i64, ActivityStorageError> {
            if self.should_fail.load(Ordering::SeqCst) {
                return Err(ActivityStorageError::DatabaseError(
                    "Mock failure".to_string(),
                ));
            }

            let mut activities = self.activities.lock().unwrap();
            let id = activities.len() as i64 + 1;
            activities.push(activity);

            // Remove from captures (mark as analyzed)
            let capture_id = activities.last().unwrap().capture_id;
            self.captures.lock().unwrap().retain(|c| c.id != capture_id);

            Ok(id)
        }

        async fn get_activities_by_date_range(
            &self,
            _from: i64,
            _to: i64,
        ) -> Result<Vec<Activity>, ActivityStorageError> {
            Ok(self.activities.lock().unwrap().clone())
        }

        async fn get_today_activities(&self) -> Result<Vec<Activity>, ActivityStorageError> {
            Ok(self.activities.lock().unwrap().clone())
        }

        async fn get_unanalyzed_capture_ids(&self) -> Result<Vec<i64>, ActivityStorageError> {
            Ok(self.captures.lock().unwrap().iter().map(|c| c.id).collect())
        }

        async fn get_unanalyzed_captures(
            &self,
            limit: usize,
        ) -> Result<Vec<CaptureForAnalysis>, ActivityStorageError> {
            let captures = self.captures.lock().unwrap();
            Ok(captures.iter().take(limit).cloned().collect())
        }

        async fn get_stats_by_application(
            &self,
            _from: i64,
            _to: i64,
        ) -> Result<Vec<crate::ports::activity::ApplicationStats>, ActivityStorageError> {
            Ok(vec![])
        }

        async fn get_stats_by_category(
            &self,
            _from: i64,
            _to: i64,
        ) -> Result<Vec<crate::ports::activity::CategoryStats>, ActivityStorageError> {
            Ok(vec![])
        }
    }

    fn create_test_config(enabled: bool, interval_seconds: u64) -> Config {
        let mut config = Config::default();
        config.activity.enabled = enabled;
        config.activity.analyze_interval_seconds = interval_seconds;
        config
    }

    // === Scheduler Creation Tests ===

    #[tokio::test]
    async fn test_scheduler_creation() {
        let analyzer = Arc::new(MockAnalyzer::new());
        let storage = Arc::new(MockStorage::new());
        let config = Arc::new(create_test_config(true, 60));

        let scheduler = ActivityAnalysisScheduler::new(analyzer, storage, config);

        assert!(!scheduler.is_running());
        assert_eq!(scheduler.consecutive_failures(), 0);
    }

    #[tokio::test]
    async fn test_scheduler_with_custom_batch_size() {
        let analyzer = Arc::new(MockAnalyzer::new());
        let storage = Arc::new(MockStorage::new());
        let config = Arc::new(create_test_config(true, 60));

        let scheduler =
            ActivityAnalysisScheduler::new(analyzer, storage, config).with_batch_size(5);

        assert_eq!(scheduler.batch_size, 5);
    }

    // === Start/Stop Tests ===

    #[tokio::test]
    async fn test_scheduler_start_when_disabled() {
        let analyzer = Arc::new(MockAnalyzer::new());
        let storage = Arc::new(MockStorage::new());
        let config = Arc::new(create_test_config(false, 60));

        let scheduler = ActivityAnalysisScheduler::new(analyzer, storage, config);

        let result = scheduler.start().await;
        assert!(matches!(result, Err(ActivitySchedulerError::Disabled)));
    }

    #[tokio::test]
    async fn test_scheduler_start_stop() {
        let analyzer = Arc::new(MockAnalyzer::new());
        let storage = Arc::new(MockStorage::new());
        let config = Arc::new(create_test_config(true, 1));

        let scheduler = ActivityAnalysisScheduler::new(analyzer, storage, config);

        // Start scheduler
        let result = scheduler.start().await;
        assert!(result.is_ok());
        assert!(scheduler.is_running());

        // Wait briefly
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Stop scheduler
        let result = scheduler.stop().await;
        assert!(result.is_ok());
        assert!(!scheduler.is_running());
    }

    #[tokio::test]
    async fn test_scheduler_double_start_fails() {
        let analyzer = Arc::new(MockAnalyzer::new());
        let storage = Arc::new(MockStorage::new());
        let config = Arc::new(create_test_config(true, 60));

        let scheduler = ActivityAnalysisScheduler::new(analyzer, storage, config);

        // First start should succeed
        scheduler.start().await.unwrap();

        // Second start should fail
        let result = scheduler.start().await;
        assert!(matches!(
            result,
            Err(ActivitySchedulerError::AlreadyRunning)
        ));

        // Cleanup
        let _ = scheduler.stop().await;
    }

    #[tokio::test]
    async fn test_scheduler_stop_when_not_running() {
        let analyzer = Arc::new(MockAnalyzer::new());
        let storage = Arc::new(MockStorage::new());
        let config = Arc::new(create_test_config(true, 60));

        let scheduler = ActivityAnalysisScheduler::new(analyzer, storage, config);

        let result = scheduler.stop().await;
        assert!(matches!(result, Err(ActivitySchedulerError::NotRunning)));
    }

    // === Analysis Tests ===

    #[tokio::test]
    async fn test_analyze_now_with_no_captures() {
        let analyzer = Arc::new(MockAnalyzer::new());
        let storage = Arc::new(MockStorage::new());
        let config = Arc::new(create_test_config(true, 60));

        let scheduler = ActivityAnalysisScheduler::new(analyzer, storage, config);

        let result = scheduler.analyze_now().await;
        assert!(result.is_ok());

        let batch_result = result.unwrap();
        assert_eq!(batch_result.success_count, 0);
        assert_eq!(batch_result.failure_count, 0);
    }

    #[tokio::test]
    async fn test_analyze_now_with_captures() {
        let analyzer = Arc::new(MockAnalyzer::new());
        let storage = Arc::new(MockStorage::new());

        // Add some test captures
        for i in 1..=3 {
            storage.add_capture(CaptureForAnalysis {
                id: i,
                timestamp: 1704326400 + i * 60,
                file_path: format!("/captures/test_{}.png", i),
            });
        }

        let config = Arc::new(create_test_config(true, 60));
        let scheduler =
            ActivityAnalysisScheduler::new(Arc::clone(&analyzer), Arc::clone(&storage), config);

        let result = scheduler.analyze_now().await;
        assert!(result.is_ok());

        let batch_result = result.unwrap();
        assert_eq!(batch_result.success_count, 3);
        assert_eq!(batch_result.failure_count, 0);

        // Verify activities were saved
        assert_eq!(storage.activity_count(), 3);

        // Verify analyzer was called
        assert_eq!(analyzer.analyze_count(), 3);
    }

    #[tokio::test]
    async fn test_analyze_now_with_failures() {
        let analyzer = Arc::new(MockAnalyzer::new());
        let storage = Arc::new(MockStorage::new());

        // Add test captures
        for i in 1..=3 {
            storage.add_capture(CaptureForAnalysis {
                id: i,
                timestamp: 1704326400 + i * 60,
                file_path: format!("/captures/test_{}.png", i),
            });
        }

        // Make analyzer fail
        analyzer.set_should_fail(true);

        let config = Arc::new(create_test_config(true, 60));
        let scheduler = ActivityAnalysisScheduler::new(analyzer, Arc::clone(&storage), config);

        let result = scheduler.analyze_now().await;
        assert!(result.is_ok());

        let batch_result = result.unwrap();
        assert_eq!(batch_result.success_count, 0);
        assert_eq!(batch_result.failure_count, 3);

        // No activities should be saved
        assert_eq!(storage.activity_count(), 0);
    }

    #[tokio::test]
    async fn test_analyze_now_respects_batch_size() {
        let analyzer = Arc::new(MockAnalyzer::new());
        let storage = Arc::new(MockStorage::new());

        // Add 5 test captures
        for i in 1..=5 {
            storage.add_capture(CaptureForAnalysis {
                id: i,
                timestamp: 1704326400 + i * 60,
                file_path: format!("/captures/test_{}.png", i),
            });
        }

        let config = Arc::new(create_test_config(true, 60));
        let scheduler =
            ActivityAnalysisScheduler::new(Arc::clone(&analyzer), Arc::clone(&storage), config)
                .with_batch_size(2);

        let result = scheduler.analyze_now().await;
        assert!(result.is_ok());

        let batch_result = result.unwrap();
        assert_eq!(batch_result.success_count, 2);

        // Only 2 activities should be saved due to batch size
        assert_eq!(storage.activity_count(), 2);
    }

    // === analyze_once Function Tests ===

    #[tokio::test]
    async fn test_analyze_once_function() {
        let analyzer = MockAnalyzer::new();
        let storage = MockStorage::new();

        // Add test captures
        for i in 1..=2 {
            storage.add_capture(CaptureForAnalysis {
                id: i,
                timestamp: 1704326400 + i * 60,
                file_path: format!("/captures/test_{}.png", i),
            });
        }

        let result = analyze_once(&analyzer, &storage, 10).await;
        assert!(result.is_ok());

        let batch_result = result.unwrap();
        assert_eq!(batch_result.success_count, 2);
        assert_eq!(batch_result.failure_count, 0);
    }

    // === Error Handling Tests ===

    #[tokio::test]
    async fn test_error_conversion_from_storage_error() {
        let err = ActivityStorageError::DatabaseError("test".to_string());
        let scheduler_err: ActivitySchedulerError = err.into();
        assert!(matches!(
            scheduler_err,
            ActivitySchedulerError::StorageError(_)
        ));
    }

    #[tokio::test]
    async fn test_error_conversion_from_analysis_error() {
        let err = AnalysisError::AIError("test".to_string());
        let scheduler_err: ActivitySchedulerError = err.into();
        assert!(matches!(
            scheduler_err,
            ActivitySchedulerError::AnalysisFailed(_)
        ));
    }

    #[test]
    fn test_scheduler_error_display() {
        let err = ActivitySchedulerError::AnalysisFailed("test error".to_string());
        assert!(err.to_string().contains("test error"));

        let err = ActivitySchedulerError::StorageError("db error".to_string());
        assert!(err.to_string().contains("db error"));

        let err = ActivitySchedulerError::AlreadyRunning;
        assert!(err.to_string().contains("already running"));

        let err = ActivitySchedulerError::NotRunning;
        assert!(err.to_string().contains("not running"));

        let err = ActivitySchedulerError::Disabled;
        assert!(err.to_string().contains("disabled"));
    }

    #[test]
    fn test_analysis_batch_result() {
        let result = AnalysisBatchResult {
            success_count: 5,
            failure_count: 2,
        };
        assert_eq!(result.success_count, 5);
        assert_eq!(result.failure_count, 2);
    }
}
