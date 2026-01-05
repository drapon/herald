//! Storage port definition

use async_trait::async_trait;
use thiserror::Error;

/// Metadata for a captured screenshot
#[derive(Debug, Clone)]
pub struct CaptureMetadata {
    /// Database ID (auto-increment)
    pub id: Option<i64>,
    /// Unix timestamp of capture
    pub timestamp: i64,
    /// Path to the image file
    pub file_path: String,
    /// Record creation timestamp
    pub created_at: i64,
}

/// Statistics about stored captures
#[derive(Debug, Clone)]
pub struct StorageStatistics {
    /// Total number of captures
    pub total_captures: usize,
    /// Total storage size in bytes
    pub total_size_bytes: u64,
    /// Oldest capture timestamp
    pub oldest_timestamp: Option<i64>,
    /// Newest capture timestamp
    pub newest_timestamp: Option<i64>,
}

/// Errors that can occur during storage operations
#[derive(Debug, Error)]
pub enum StorageError {
    /// Database operation failed
    #[error("Database error: {0}")]
    DatabaseError(String),

    /// Connection to database failed
    #[error("Connection error: {0}")]
    ConnectionError(String),

    /// Record not found
    #[error("Record not found: id={0}")]
    NotFound(i64),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Port for storage operations
#[async_trait]
pub trait StoragePort: Send + Sync {
    /// Save capture metadata to storage
    async fn save_metadata(&self, metadata: CaptureMetadata) -> Result<(), StorageError>;

    /// Get the most recent captures
    async fn get_recent_captures(&self, limit: usize) -> Result<Vec<CaptureMetadata>, StorageError>;

    /// Get captures older than the given timestamp
    async fn get_old_captures(&self, cutoff_timestamp: i64)
        -> Result<Vec<CaptureMetadata>, StorageError>;

    /// Delete a capture by ID
    async fn delete_capture(&self, id: i64) -> Result<(), StorageError>;

    /// Get storage statistics
    async fn get_statistics(&self) -> Result<StorageStatistics, StorageError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_metadata() {
        let metadata = CaptureMetadata {
            id: Some(1),
            timestamp: 1704326400,
            file_path: "/home/user/.herald/captures/2024-01-04_00-00-00.png".to_string(),
            created_at: 1704326400,
        };
        assert_eq!(metadata.id, Some(1));
        assert!(metadata.file_path.contains("captures"));
    }

    #[test]
    fn test_storage_statistics() {
        let stats = StorageStatistics {
            total_captures: 100,
            total_size_bytes: 100_000_000,
            oldest_timestamp: Some(1704240000),
            newest_timestamp: Some(1704326400),
        };
        assert_eq!(stats.total_captures, 100);
        assert_eq!(stats.total_size_bytes, 100_000_000);
    }

    #[test]
    fn test_storage_error_display() {
        let err = StorageError::DatabaseError("connection timeout".to_string());
        assert!(err.to_string().contains("connection timeout"));
    }
}
