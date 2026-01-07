//! Daemon management commands
//!
//! Handles `herald daemon start` and `herald daemon stop` commands.

use anyhow::{Context, Result};
use herald_core::{
    init_logger, load_config, ActivityAnalysisScheduler, CaptureScheduler, Config,
    DaemonController, DirectoryManager, LogLevel, LoggerConfig, RetentionManager,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;

/// Get the PID file path from config
fn get_pid_file_path(config: &Config) -> PathBuf {
    config.storage.data_dir.join("daemon.pid")
}

/// Start the Herald daemon
///
/// This starts the background capture scheduler and retention manager.
/// The daemon will run until stopped with `herald daemon stop` or interrupted.
pub async fn start() -> Result<()> {
    // Load configuration
    let config = load_config().context("Failed to load configuration")?;
    let config = Arc::new(config);

    // Initialize directory structure
    let dir_manager = DirectoryManager::new(config.storage.data_dir.clone());
    dir_manager
        .initialize()
        .context("Failed to create directories")?;

    // Initialize logging
    let logger_config = LoggerConfig::new(config.storage.data_dir.join("logs"))
        .with_level(LogLevel::Info)
        .with_max_file_size(10 * 1024 * 1024); // 10MB
    let _logger_guard = init_logger(logger_config).context("Failed to initialize logger")?;

    // Check if daemon is already running
    let pid_file_path = get_pid_file_path(&config);
    let mut daemon_controller = DaemonController::new(pid_file_path);

    let daemon_result = daemon_controller
        .register_daemon()
        .context("Failed to register daemon")?;

    println!("Herald daemon started (PID: {})", daemon_result.pid);
    println!(
        "Capture interval: {} seconds",
        config.capture.interval_seconds
    );
    println!(
        "Data directory: {}",
        config.storage.data_dir.to_string_lossy()
    );

    tracing::info!("Herald daemon started with PID {}", daemon_result.pid);

    // Initialize adapters
    #[cfg(target_os = "macos")]
    let capture_adapter = {
        use herald_adapters::ScreenCaptureKitAdapter;
        Arc::new(ScreenCaptureKitAdapter::new())
    };

    #[cfg(not(target_os = "macos"))]
    let capture_adapter = {
        anyhow::bail!("Herald currently only supports macOS");
    };

    let db_path = config.storage.data_dir.join("herald.db");
    let storage_adapter = Arc::new(
        herald_adapters::SqliteAdapter::new(&db_path)
            .await
            .context("Failed to initialize database")?,
    );

    // Start capture scheduler
    let scheduler = CaptureScheduler::new(
        Arc::clone(&capture_adapter),
        Arc::clone(&storage_adapter),
        Arc::clone(&config),
    );
    scheduler
        .start()
        .await
        .context("Failed to start scheduler")?;
    tracing::info!("Capture scheduler started");

    // Start retention manager
    let retention_manager =
        RetentionManager::new(Arc::clone(&storage_adapter), Arc::clone(&config));
    retention_manager
        .start()
        .await
        .context("Failed to start retention manager")?;
    tracing::info!("Retention manager started");

    // Start activity analysis scheduler (if enabled)
    let activity_scheduler_handle = if config.activity.enabled {
        start_activity_scheduler(Arc::clone(&storage_adapter), Arc::clone(&config)).await
    } else {
        tracing::info!("Activity analysis is disabled in config");
        None
    };

    // Wait for shutdown signal
    let mut shutdown_rx = daemon_result.shutdown_rx;
    tokio::select! {
        _ = signal::ctrl_c() => {
            println!("\nReceived Ctrl+C, shutting down...");
            tracing::info!("Received Ctrl+C signal");
        }
        _ = shutdown_rx.changed() => {
            println!("Received shutdown signal, shutting down...");
            tracing::info!("Received shutdown signal from daemon controller");
        }
    }

    // Graceful shutdown
    tracing::info!("Stopping capture scheduler...");
    if let Err(e) = scheduler.stop().await {
        tracing::warn!("Error stopping scheduler: {}", e);
    }

    tracing::info!("Stopping retention manager...");
    if let Err(e) = retention_manager.stop().await {
        tracing::warn!("Error stopping retention manager: {}", e);
    }

    // Stop activity scheduler if it was started
    if let Some(handle) = activity_scheduler_handle {
        tracing::info!("Stopping activity analysis scheduler...");
        stop_activity_scheduler(handle).await;
    }

    // Clean up PID file
    daemon_controller.unregister_daemon()?;
    println!("Herald daemon stopped");
    tracing::info!("Herald daemon stopped");

    Ok(())
}

/// Stop the Herald daemon
///
/// Sends a shutdown signal to the running daemon process.
pub async fn stop() -> Result<()> {
    // Load configuration
    let config = load_config().context("Failed to load configuration")?;

    let pid_file_path = get_pid_file_path(&config);
    let mut daemon_controller = DaemonController::new(pid_file_path);

    match daemon_controller.stop_daemon() {
        Ok(pid) => {
            println!("Sent stop signal to Herald daemon (PID: {})", pid);

            // Wait a bit for the daemon to stop
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Check if it stopped
            if daemon_controller.is_running()?.is_none() {
                println!("Herald daemon stopped successfully");
            } else {
                println!("Daemon may still be shutting down...");
            }
            Ok(())
        }
        Err(herald_core::DaemonError::NotRunning) => {
            println!("Herald daemon is not running");
            Ok(())
        }
        Err(e) => Err(e).context("Failed to stop daemon"),
    }
}

// ============================================================================
// Activity Scheduler Helpers
// ============================================================================

/// Handle for a running activity scheduler (type-erased)
enum ActivitySchedulerHandle {
    Claude(
        ActivityAnalysisScheduler<
            herald_adapters::AIActivityAnalyzer<herald_adapters::ClaudeAdapter>,
            herald_adapters::SqliteAdapter,
        >,
    ),
    Gemini(
        ActivityAnalysisScheduler<
            herald_adapters::AIActivityAnalyzer<herald_adapters::GeminiAdapter>,
            herald_adapters::SqliteAdapter,
        >,
    ),
}

/// Get API key from config or environment variable
fn get_api_key(config: &Config, env_var_name: &str) -> Option<String> {
    // First, try to get from config
    if let Some(ref key) = config.ai.api_key {
        if !key.is_empty() {
            return Some(key.clone());
        }
    }

    // Fallback to environment variable
    std::env::var(env_var_name).ok()
}

/// Start the activity analysis scheduler based on configuration
async fn start_activity_scheduler(
    storage: Arc<herald_adapters::SqliteAdapter>,
    config: Arc<Config>,
) -> Option<ActivitySchedulerHandle> {
    // Try to get API key and create scheduler based on provider
    match config.ai.default_provider.as_str() {
        "gemini" => {
            let api_key = match get_api_key(&config, "GEMINI_API_KEY") {
                Some(key) => key,
                None => {
                    tracing::warn!(
                        "Gemini API key not set (config.ai.api_key or GEMINI_API_KEY), activity analysis disabled"
                    );
                    return None;
                }
            };

            let provider = Arc::new(herald_adapters::GeminiAdapter::new(
                api_key,
                config.ai.model.clone(),
            ));
            let analyzer = Arc::new(herald_adapters::AIActivityAnalyzer::new(provider));
            let scheduler = ActivityAnalysisScheduler::new(analyzer, storage, Arc::clone(&config));

            match scheduler.start().await {
                Ok(()) => {
                    println!(
                        "Activity analysis enabled (interval: {} seconds)",
                        config.activity.analyze_interval_seconds
                    );
                    tracing::info!("Activity analysis scheduler started with Gemini");
                    Some(ActivitySchedulerHandle::Gemini(scheduler))
                }
                Err(e) => {
                    tracing::warn!("Failed to start activity scheduler: {}", e);
                    None
                }
            }
        }
        _ => {
            // Default to Claude
            let api_key = match get_api_key(&config, "ANTHROPIC_API_KEY") {
                Some(key) => key,
                None => {
                    tracing::warn!(
                        "Claude API key not set (config.ai.api_key or ANTHROPIC_API_KEY), activity analysis disabled"
                    );
                    return None;
                }
            };

            let provider = Arc::new(herald_adapters::ClaudeAdapter::new(
                api_key,
                config.ai.model.clone(),
            ));
            let analyzer = Arc::new(herald_adapters::AIActivityAnalyzer::new(provider));
            let scheduler = ActivityAnalysisScheduler::new(analyzer, storage, Arc::clone(&config));

            match scheduler.start().await {
                Ok(()) => {
                    println!(
                        "Activity analysis enabled (interval: {} seconds)",
                        config.activity.analyze_interval_seconds
                    );
                    tracing::info!("Activity analysis scheduler started with Claude");
                    Some(ActivitySchedulerHandle::Claude(scheduler))
                }
                Err(e) => {
                    tracing::warn!("Failed to start activity scheduler: {}", e);
                    None
                }
            }
        }
    }
}

/// Stop the activity analysis scheduler
async fn stop_activity_scheduler(handle: ActivitySchedulerHandle) {
    match handle {
        ActivitySchedulerHandle::Claude(scheduler) => {
            if let Err(e) = scheduler.stop().await {
                tracing::warn!("Error stopping Claude activity scheduler: {}", e);
            }
        }
        ActivitySchedulerHandle::Gemini(scheduler) => {
            if let Err(e) = scheduler.stop().await {
                tracing::warn!("Error stopping Gemini activity scheduler: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_get_pid_file_path() {
        let mut config = Config::default();
        config.storage.data_dir = PathBuf::from("/home/user/.herald");

        let path = get_pid_file_path(&config);
        assert_eq!(path, PathBuf::from("/home/user/.herald/daemon.pid"));
    }

    #[tokio::test]
    async fn test_stop_when_not_running() {
        let temp_dir = TempDir::new().unwrap();
        let pid_file_path = temp_dir.path().join("daemon.pid");
        let mut controller = DaemonController::new(pid_file_path);

        // Should not error when daemon is not running
        let result = controller.stop_daemon();
        assert!(matches!(result, Err(herald_core::DaemonError::NotRunning)));
    }
}
