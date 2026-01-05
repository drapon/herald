//! Storage adapter implementations
//!
//! Contains the SQLite adapter for persistent storage.

use async_trait::async_trait;
use herald_core::ports::storage::{CaptureMetadata, StorageError, StoragePort, StorageStatistics};
use std::path::Path;
use tokio_rusqlite::Connection;

/// SQL statements for database initialization
const CREATE_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS captures (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp INTEGER NOT NULL,
    file_path TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
)
"#;

const CREATE_TIMESTAMP_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_timestamp ON captures(timestamp)";

const CREATE_CREATED_AT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_created_at ON captures(created_at)";

/// SQLite adapter implementing StoragePort
pub struct SqliteAdapter {
    conn: Connection,
}

impl SqliteAdapter {
    /// Creates a new SQLite adapter and initializes the database schema
    ///
    /// # Arguments
    /// * `db_path` - Path to the SQLite database file
    ///
    /// # Errors
    /// Returns `StorageError::ConnectionError` if database connection fails
    pub async fn new(db_path: &Path) -> Result<Self, StorageError> {
        let path_str = db_path.to_string_lossy().to_string();

        let conn = Connection::open(&path_str)
            .await
            .map_err(|e| StorageError::ConnectionError(e.to_string()))?;

        // Initialize database schema
        Self::initialize_schema(&conn).await?;

        tracing::info!("SQLite database initialized at {}", path_str);
        Ok(Self { conn })
    }

    /// Creates a new in-memory SQLite adapter for testing
    #[cfg(test)]
    pub async fn new_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open(":memory:")
            .await
            .map_err(|e| StorageError::ConnectionError(e.to_string()))?;

        Self::initialize_schema(&conn).await?;

        Ok(Self { conn })
    }

    /// Initializes the database schema
    async fn initialize_schema(conn: &Connection) -> Result<(), StorageError> {
        conn.call(|conn| {
            // Enable WAL mode for better write performance
            conn.execute_batch("PRAGMA journal_mode=WAL;")?;

            // Create captures table
            conn.execute(CREATE_TABLE_SQL, [])?;

            // Create indexes
            conn.execute(CREATE_TIMESTAMP_INDEX_SQL, [])?;
            conn.execute(CREATE_CREATED_AT_INDEX_SQL, [])?;

            Ok(())
        })
        .await
        .map_err(|e| StorageError::DatabaseError(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl StoragePort for SqliteAdapter {
    async fn save_metadata(&self, metadata: CaptureMetadata) -> Result<(), StorageError> {
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO captures (timestamp, file_path, created_at) VALUES (?1, ?2, ?3)",
                    rusqlite::params![metadata.timestamp, metadata.file_path, metadata.created_at],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| StorageError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn get_recent_captures(
        &self,
        limit: usize,
    ) -> Result<Vec<CaptureMetadata>, StorageError> {
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, timestamp, file_path, created_at FROM captures ORDER BY timestamp DESC LIMIT ?1",
                )?;

                let rows = stmt.query_map([limit as i64], |row| {
                    Ok(CaptureMetadata {
                        id: Some(row.get(0)?),
                        timestamp: row.get(1)?,
                        file_path: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                })?;

                let mut captures = Vec::new();
                for row in rows {
                    captures.push(row?);
                }
                Ok(captures)
            })
            .await
            .map_err(|e| StorageError::DatabaseError(e.to_string()))
    }

    async fn get_old_captures(
        &self,
        cutoff_timestamp: i64,
    ) -> Result<Vec<CaptureMetadata>, StorageError> {
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, timestamp, file_path, created_at FROM captures WHERE timestamp < ?1 ORDER BY timestamp ASC",
                )?;

                let rows = stmt.query_map([cutoff_timestamp], |row| {
                    Ok(CaptureMetadata {
                        id: Some(row.get(0)?),
                        timestamp: row.get(1)?,
                        file_path: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                })?;

                let mut captures = Vec::new();
                for row in rows {
                    captures.push(row?);
                }
                Ok(captures)
            })
            .await
            .map_err(|e| StorageError::DatabaseError(e.to_string()))
    }

    async fn delete_capture(&self, id: i64) -> Result<(), StorageError> {
        let rows_affected = self
            .conn
            .call(move |conn| {
                let affected = conn.execute("DELETE FROM captures WHERE id = ?1", [id])?;
                Ok(affected)
            })
            .await
            .map_err(|e| StorageError::DatabaseError(e.to_string()))?;

        if rows_affected == 0 {
            return Err(StorageError::NotFound(id));
        }

        Ok(())
    }

    async fn get_statistics(&self) -> Result<StorageStatistics, StorageError> {
        self.conn
            .call(|conn| {
                let total_captures: usize =
                    conn.query_row("SELECT COUNT(*) FROM captures", [], |row| row.get(0))?;

                let oldest_timestamp: Option<i64> = conn
                    .query_row("SELECT MIN(timestamp) FROM captures", [], |row| row.get(0))
                    .ok();

                let newest_timestamp: Option<i64> = conn
                    .query_row("SELECT MAX(timestamp) FROM captures", [], |row| row.get(0))
                    .ok();

                // Note: total_size_bytes requires summing file sizes from the filesystem
                // For now, we return 0 as it will be calculated by the caller
                Ok(StorageStatistics {
                    total_captures,
                    total_size_bytes: 0,
                    oldest_timestamp,
                    newest_timestamp,
                })
            })
            .await
            .map_err(|e| StorageError::DatabaseError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // === Task 3.2 TDD Tests ===

    #[tokio::test]
    async fn test_create_database_file() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("herald.db");

        assert!(!db_path.exists());

        let _adapter = SqliteAdapter::new(&db_path)
            .await
            .expect("Failed to create adapter");

        assert!(db_path.exists());
    }

    #[tokio::test]
    async fn test_schema_creation() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // Verify table exists by attempting to query it
        let result = adapter.get_statistics().await;
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert_eq!(stats.total_captures, 0);
    }

    #[tokio::test]
    async fn test_save_and_retrieve_metadata() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        let metadata = CaptureMetadata {
            id: None,
            timestamp: 1704326400,
            file_path: "/home/user/.herald/captures/2024-01-04_00-00-00.png".to_string(),
            created_at: 1704326400,
        };

        adapter
            .save_metadata(metadata.clone())
            .await
            .expect("Failed to save");

        let captures = adapter
            .get_recent_captures(10)
            .await
            .expect("Failed to retrieve");
        assert_eq!(captures.len(), 1);
        assert!(captures[0].id.is_some());
        assert_eq!(captures[0].timestamp, 1704326400);
        assert_eq!(captures[0].file_path, metadata.file_path);
    }

    #[tokio::test]
    async fn test_get_recent_captures_respects_limit() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // Insert 5 captures
        for i in 0..5 {
            let metadata = CaptureMetadata {
                id: None,
                timestamp: 1704326400 + i * 60,
                file_path: format!("/captures/capture_{}.png", i),
                created_at: 1704326400 + i * 60,
            };
            adapter
                .save_metadata(metadata)
                .await
                .expect("Failed to save");
        }

        // Request only 3
        let captures = adapter
            .get_recent_captures(3)
            .await
            .expect("Failed to retrieve");
        assert_eq!(captures.len(), 3);

        // Should be ordered by timestamp DESC (newest first)
        assert!(captures[0].timestamp > captures[1].timestamp);
        assert!(captures[1].timestamp > captures[2].timestamp);
    }

    #[tokio::test]
    async fn test_get_old_captures() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // Insert captures with varying timestamps
        let base_time = 1704326400i64;
        for i in 0..5 {
            let metadata = CaptureMetadata {
                id: None,
                timestamp: base_time + i * 3600, // 1 hour apart
                file_path: format!("/captures/capture_{}.png", i),
                created_at: base_time + i * 3600,
            };
            adapter
                .save_metadata(metadata)
                .await
                .expect("Failed to save");
        }

        // Get captures older than base_time + 2 hours
        let cutoff = base_time + 7200;
        let old_captures = adapter
            .get_old_captures(cutoff)
            .await
            .expect("Failed to retrieve");

        // Should return captures with timestamp 0 and 1 (before cutoff)
        assert_eq!(old_captures.len(), 2);
        for capture in &old_captures {
            assert!(capture.timestamp < cutoff);
        }
    }

    #[tokio::test]
    async fn test_delete_capture() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        let metadata = CaptureMetadata {
            id: None,
            timestamp: 1704326400,
            file_path: "/captures/test.png".to_string(),
            created_at: 1704326400,
        };

        adapter
            .save_metadata(metadata)
            .await
            .expect("Failed to save");

        let captures = adapter
            .get_recent_captures(10)
            .await
            .expect("Failed to retrieve");
        let id = captures[0].id.expect("Should have ID");

        // Delete the capture
        adapter.delete_capture(id).await.expect("Failed to delete");

        // Verify it's gone
        let captures_after = adapter
            .get_recent_captures(10)
            .await
            .expect("Failed to retrieve");
        assert_eq!(captures_after.len(), 0);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_capture_returns_not_found() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        let result = adapter.delete_capture(99999).await;
        assert!(matches!(result, Err(StorageError::NotFound(99999))));
    }

    #[tokio::test]
    async fn test_unique_file_path_constraint() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        let metadata = CaptureMetadata {
            id: None,
            timestamp: 1704326400,
            file_path: "/captures/duplicate.png".to_string(),
            created_at: 1704326400,
        };

        adapter
            .save_metadata(metadata.clone())
            .await
            .expect("First save should succeed");

        // Second save with same file_path should fail
        let result = adapter.save_metadata(metadata).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_statistics() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // Empty database
        let stats = adapter.get_statistics().await.expect("Failed to get stats");
        assert_eq!(stats.total_captures, 0);
        assert!(stats.oldest_timestamp.is_none() || stats.oldest_timestamp == Some(0));
        assert!(stats.newest_timestamp.is_none() || stats.newest_timestamp == Some(0));

        // Insert some captures
        for i in 0..3 {
            let metadata = CaptureMetadata {
                id: None,
                timestamp: 1704326400 + i * 60,
                file_path: format!("/captures/capture_{}.png", i),
                created_at: 1704326400 + i * 60,
            };
            adapter
                .save_metadata(metadata)
                .await
                .expect("Failed to save");
        }

        let stats = adapter.get_statistics().await.expect("Failed to get stats");
        assert_eq!(stats.total_captures, 3);
        assert_eq!(stats.oldest_timestamp, Some(1704326400));
        assert_eq!(stats.newest_timestamp, Some(1704326400 + 120));
    }

    #[tokio::test]
    async fn test_wal_mode_enabled() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("herald.db");

        let adapter = SqliteAdapter::new(&db_path)
            .await
            .expect("Failed to create adapter");

        // WAL mode creates additional files
        // After first write, WAL file should be created
        let metadata = CaptureMetadata {
            id: None,
            timestamp: 1704326400,
            file_path: "/captures/test.png".to_string(),
            created_at: 1704326400,
        };
        adapter
            .save_metadata(metadata)
            .await
            .expect("Failed to save");

        // Note: WAL file might not be created if there's no checkpoint yet
        // Instead, verify by querying the journal_mode
        let result = adapter
            .conn
            .call(|conn| {
                let mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
                Ok(mode)
            })
            .await
            .expect("Failed to query journal mode");

        assert_eq!(result.to_lowercase(), "wal");
    }

    #[tokio::test]
    async fn test_index_on_timestamp_exists() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        let has_index = adapter
            .conn
            .call(|conn| {
                let count: i32 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_timestamp'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count > 0)
            })
            .await
            .expect("Failed to query index");

        assert!(has_index, "idx_timestamp index should exist");
    }

    #[tokio::test]
    async fn test_index_on_created_at_exists() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        let has_index = adapter
            .conn
            .call(|conn| {
                let count: i32 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_created_at'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count > 0)
            })
            .await
            .expect("Failed to query index");

        assert!(has_index, "idx_created_at index should exist");
    }

    #[tokio::test]
    async fn test_database_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("herald.db");

        // Create adapter and insert data
        {
            let adapter = SqliteAdapter::new(&db_path)
                .await
                .expect("Failed to create adapter");

            let metadata = CaptureMetadata {
                id: None,
                timestamp: 1704326400,
                file_path: "/captures/persistent.png".to_string(),
                created_at: 1704326400,
            };
            adapter
                .save_metadata(metadata)
                .await
                .expect("Failed to save");
        }

        // Re-open database and verify data persists
        {
            let adapter = SqliteAdapter::new(&db_path)
                .await
                .expect("Failed to reopen");
            let captures = adapter
                .get_recent_captures(10)
                .await
                .expect("Failed to retrieve");

            assert_eq!(captures.len(), 1);
            assert_eq!(captures[0].file_path, "/captures/persistent.png");
        }
    }
}
