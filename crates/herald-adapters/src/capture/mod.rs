//! Screen capture adapter implementations
//!
//! Contains the ScreenCaptureKit adapter for macOS screen capture.

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::ScreenCaptureKitAdapter;

// Provide a stub for non-macOS platforms
#[cfg(not(target_os = "macos"))]
pub struct ScreenCaptureKitAdapter;

#[cfg(not(target_os = "macos"))]
impl ScreenCaptureKitAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_capture_module_exists() {
        assert!(true);
    }
}
