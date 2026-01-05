//! Capture port definition

use async_trait::async_trait;
use thiserror::Error;

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
}

/// Port for screen capture operations
#[async_trait]
pub trait CapturePort: Send + Sync {
    /// Capture a screenshot of the main display
    async fn capture_screen(&self) -> Result<CapturedImage, CaptureError>;

    /// Check if screen recording permission is granted
    async fn check_permission(&self) -> Result<bool, CaptureError>;
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
}
