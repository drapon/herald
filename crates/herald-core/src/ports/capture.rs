//! Capture port definition

use async_trait::async_trait;
use thiserror::Error;

/// Information about an available display
#[derive(Debug, Clone)]
pub struct DisplayInfo {
    /// Display index (0-based)
    pub index: u32,
    /// Display width in pixels
    pub width: u32,
    /// Display height in pixels
    pub height: u32,
}

/// Result of a screen capture operation
#[derive(Debug, Clone)]
pub struct CapturedImage {
    /// Raw PNG image data
    pub data: Vec<u8>,
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
    /// Unix timestamp of capture
    pub timestamp: i64,
}

/// Errors that can occur during capture operations
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Screen recording permission was denied
    #[error("Screen recording permission denied. Please enable in System Settings > Privacy & Security > Screen Recording")]
    PermissionDenied,

    /// OS version is not supported (requires macOS 12.3+)
    #[error("ScreenCaptureKit requires macOS 12.3 or later (current: {0})")]
    UnsupportedOS(String),

    /// Capture operation failed
    #[error("Capture failed: {0}")]
    CaptureFailed(String),

    /// IO error during capture
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Invalid display index specified
    #[error("Invalid display index: {0} (available: {1} displays)")]
    InvalidDisplayIndex(u32, usize),
}

/// Port for screen capture operations
#[async_trait]
pub trait CapturePort: Send + Sync {
    /// Capture a screenshot of the main display
    ///
    /// This is the original method for backward compatibility.
    /// It captures the first available display.
    async fn capture_screen(&self) -> Result<CapturedImage, CaptureError>;

    /// Check if screen recording permission is granted
    async fn check_permission(&self) -> Result<bool, CaptureError>;

    /// Get list of available displays
    ///
    /// Returns information about all connected displays, including their
    /// index, width, and height.
    async fn get_displays(&self) -> Result<Vec<DisplayInfo>, CaptureError>;

    /// Capture a specific display by index
    ///
    /// # Arguments
    /// * `index` - The 0-based index of the display to capture
    ///
    /// # Errors
    /// Returns `CaptureError::InvalidDisplayIndex` if the index is out of range
    async fn capture_display(&self, index: u32) -> Result<CapturedImage, CaptureError>;

    /// Capture all displays and combine them horizontally into one image
    ///
    /// The displays are arranged left-to-right in index order.
    /// If displays have different heights, they are aligned at the top.
    async fn capture_all_combined(&self) -> Result<CapturedImage, CaptureError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_captured_image_structure() {
        let image = CapturedImage {
            data: vec![0u8; 100],
            width: 1920,
            height: 1080,
            timestamp: 1704326400,
        };
        assert_eq!(image.width, 1920);
        assert_eq!(image.height, 1080);
        assert_eq!(image.data.len(), 100);
    }

    #[test]
    fn test_capture_error_messages() {
        let err = CaptureError::PermissionDenied;
        assert!(err.to_string().contains("Screen recording permission"));

        let err = CaptureError::UnsupportedOS("12.0".to_string());
        assert!(err.to_string().contains("macOS 12.3"));
    }

    #[test]
    fn test_display_info_structure() {
        let display = DisplayInfo {
            index: 0,
            width: 2560,
            height: 1440,
        };
        assert_eq!(display.index, 0);
        assert_eq!(display.width, 2560);
        assert_eq!(display.height, 1440);
    }

    #[test]
    fn test_invalid_display_index_error() {
        let err = CaptureError::InvalidDisplayIndex(5, 2);
        let msg = err.to_string();
        assert!(msg.contains("Invalid display index: 5"));
        assert!(msg.contains("available: 2 displays"));
    }
}
