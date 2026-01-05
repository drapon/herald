//! Manual capture command
//!
//! Handles `herald capture` command for immediate screenshot capture.

use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use herald_core::{capture_once, load_config, DirectoryManager};

/// Execute immediate screen capture
///
/// Captures a screenshot regardless of whether the daemon is running.
/// Outputs the saved file path and timestamp.
pub async fn run() -> Result<()> {
    // Load configuration
    let config = load_config().context("Failed to load configuration")?;

    // Ensure directories exist
    let dir_manager = DirectoryManager::new(config.storage.data_dir.clone());
    dir_manager
        .initialize()
        .context("Failed to create directories")?;

    // Initialize adapters
    #[cfg(target_os = "macos")]
    let capture_adapter = {
        use herald_adapters::ScreenCaptureKitAdapter;
        ScreenCaptureKitAdapter::new()
    };

    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!("Herald currently only supports macOS");
    }

    let db_path = config.storage.data_dir.join("herald.db");
    let storage_adapter = herald_adapters::SqliteAdapter::new(&db_path)
        .await
        .context("Failed to initialize database")?;

    // Perform capture
    println!("Capturing screen...");
    let result = capture_once(&capture_adapter, &storage_adapter, &config)
        .await
        .context("Failed to capture screen")?;

    // Format timestamp for display
    let datetime = Utc
        .timestamp_opt(result.timestamp, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| result.timestamp.to_string());

    // Format file size
    let size_display = format_file_size(result.size_bytes);

    println!("Capture successful!");
    println!("  File: {}", result.file_path.display());
    println!("  Time: {}", datetime);
    println!("  Size: {}", size_display);

    Ok(())
}

/// Format file size in human-readable format
fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;

    if bytes >= MB {
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
    fn test_format_file_size_bytes() {
        assert_eq!(format_file_size(500), "500 bytes");
        assert_eq!(format_file_size(1023), "1023 bytes");
    }

    #[test]
    fn test_format_file_size_kb() {
        assert_eq!(format_file_size(1024), "1.00 KB");
        assert_eq!(format_file_size(1536), "1.50 KB");
        assert_eq!(format_file_size(10240), "10.00 KB");
    }

    #[test]
    fn test_format_file_size_mb() {
        assert_eq!(format_file_size(1048576), "1.00 MB");
        assert_eq!(format_file_size(1572864), "1.50 MB");
        assert_eq!(format_file_size(10485760), "10.00 MB");
    }
}
