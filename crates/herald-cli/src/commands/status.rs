//! Status command
//!
//! Handles `herald status` command to show daemon state and capture statistics.

use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use herald_core::ports::StoragePort;
use herald_core::{load_config, DaemonController};
use std::path::Path;

/// Show daemon status and capture statistics
///
/// Displays:
/// - Daemon running state (Running/Stopped)
/// - Last capture time
/// - Total capture count
/// - Storage usage
pub async fn run() -> Result<()> {
    // Load configuration
    let config = load_config().context("Failed to load configuration")?;

    // Check daemon status
    let pid_file_path = config.storage.data_dir.join("daemon.pid");
    let daemon_controller = DaemonController::new(pid_file_path);

    let daemon_status = match daemon_controller.is_running()? {
        Some(pid) => format!("Running (PID: {})", pid),
        None => "Stopped".to_string(),
    };

    println!("Herald Status");
    println!("=============");
    println!();
    println!("Daemon: {}", daemon_status);
    println!();

    // Try to get statistics from database
    let db_path = config.storage.data_dir.join("herald.db");
    if db_path.exists() {
        match herald_adapters::SqliteAdapter::new(&db_path).await {
            Ok(storage) => {
                show_statistics(&storage, &config.storage.data_dir).await?;
            }
            Err(e) => {
                println!("Database: Error connecting ({})", e);
            }
        }
    } else {
        println!("Database: Not initialized");
        println!("  Run 'herald daemon start' or 'herald capture' to initialize.");
    }

    println!();
    println!("Configuration");
    println!("-------------");
    println!(
        "  Data directory: {}",
        config.storage.data_dir.to_string_lossy()
    );
    println!(
        "  Capture interval: {} seconds",
        config.capture.interval_seconds
    );
    println!(
        "  Retention period: {} hours",
        config.storage.retention_seconds / 3600
    );
    println!("  AI provider: {}", config.ai.default_provider);

    Ok(())
}

/// Show storage statistics
async fn show_statistics<S: StoragePort>(storage: &S, data_dir: &Path) -> Result<()> {
    let stats = storage
        .get_statistics()
        .await
        .context("Failed to get statistics")?;

    println!("Captures");
    println!("--------");
    println!("  Total count: {}", stats.total_captures);

    // Calculate actual storage size from captures directory
    let captures_dir = data_dir.join("captures");
    let storage_size = calculate_directory_size(&captures_dir).unwrap_or(0);
    println!("  Storage used: {}", format_storage_size(storage_size));

    // Show time range
    if let Some(oldest) = stats.oldest_timestamp {
        let oldest_time = format_timestamp(oldest);
        println!("  Oldest capture: {}", oldest_time);
    }

    if let Some(newest) = stats.newest_timestamp {
        let newest_time = format_timestamp(newest);
        println!("  Latest capture: {}", newest_time);
    }

    Ok(())
}

/// Calculate total size of files in a directory
fn calculate_directory_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0;

    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_file() {
                total += metadata.len();
            }
        }
    }

    Ok(total)
}

/// Format timestamp as human-readable string
fn format_timestamp(timestamp: i64) -> String {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| timestamp.to_string())
}

/// Format storage size in human-readable format
fn format_storage_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_storage_size_bytes() {
        assert_eq!(format_storage_size(0), "0 bytes");
        assert_eq!(format_storage_size(500), "500 bytes");
    }

    #[test]
    fn test_format_storage_size_kb() {
        assert_eq!(format_storage_size(1024), "1.00 KB");
        assert_eq!(format_storage_size(5120), "5.00 KB");
    }

    #[test]
    fn test_format_storage_size_mb() {
        assert_eq!(format_storage_size(1048576), "1.00 MB");
        assert_eq!(format_storage_size(104857600), "100.00 MB");
    }

    #[test]
    fn test_format_storage_size_gb() {
        assert_eq!(format_storage_size(1073741824), "1.00 GB");
        assert_eq!(format_storage_size(1610612736), "1.50 GB");
    }

    #[test]
    fn test_format_timestamp() {
        // 2024-01-04 00:00:00 UTC
        let ts = 1704326400;
        let formatted = format_timestamp(ts);
        assert_eq!(formatted, "2024-01-04 00:00:00 UTC");
    }

    #[test]
    fn test_calculate_directory_size_nonexistent() {
        let path = Path::new("/nonexistent/directory");
        let result = calculate_directory_size(path);
        assert!(result.is_err());
    }
}
