//! Storage adapter implementations
//!
//! Contains the SQLite adapter for persistent storage.

use async_trait::async_trait;
use herald_core::ports::activity::{
    Activity, ActivityCategory, ActivityStorageError, ActivityStoragePort, ApplicationStats,
    CaptureForAnalysis, CategoryStats,
};
use herald_core::ports::storage::{CaptureMetadata, StorageError, StoragePort, StorageStatistics};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_rusqlite::Connection;

/// SQL statements for database initialization - captures table
const CREATE_CAPTURES_TABLE_SQL: &str = r#"
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

/// SQL statements for database initialization - activities table
const CREATE_ACTIVITIES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS activities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_id INTEGER NOT NULL UNIQUE,
    timestamp INTEGER NOT NULL,
    application_name TEXT NOT NULL,
    activity_description TEXT NOT NULL,
    category TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (capture_id) REFERENCES captures(id) ON DELETE CASCADE
)
"#;

const CREATE_ACTIVITIES_TIMESTAMP_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_activities_timestamp ON activities(timestamp)";

const CREATE_ACTIVITIES_CATEGORY_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_activities_category ON activities(category)";

const CREATE_ACTIVITIES_APPLICATION_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_activities_application ON activities(application_name)";

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

            // Enable foreign key support
            conn.execute_batch("PRAGMA foreign_keys=ON;")?;

            // Create captures table
            conn.execute(CREATE_CAPTURES_TABLE_SQL, [])?;

            // Create captures indexes
            conn.execute(CREATE_TIMESTAMP_INDEX_SQL, [])?;
            conn.execute(CREATE_CREATED_AT_INDEX_SQL, [])?;

            // Create activities table
            conn.execute(CREATE_ACTIVITIES_TABLE_SQL, [])?;

            // Create activities indexes
            conn.execute(CREATE_ACTIVITIES_TIMESTAMP_INDEX_SQL, [])?;
            conn.execute(CREATE_ACTIVITIES_CATEGORY_INDEX_SQL, [])?;
            conn.execute(CREATE_ACTIVITIES_APPLICATION_INDEX_SQL, [])?;

            Ok(())
        })
        .await
        .map_err(|e| StorageError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Returns a reference to the database connection for activity storage
    pub fn connection(&self) -> &Connection {
        &self.conn
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

// ============================================================================
// ActivityStoragePort Implementation
// ============================================================================

/// Helper function to get the start of today in Unix timestamp
fn get_today_start() -> i64 {
    use chrono::{Datelike, Local, TimeZone};
    let now = Local::now();
    Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .unwrap()
        .timestamp()
}

/// Helper function to get the end of today in Unix timestamp
fn get_today_end() -> i64 {
    get_today_start() + 86400 // 24 hours in seconds
}

#[async_trait]
impl ActivityStoragePort for SqliteAdapter {
    async fn save_activity(&self, activity: Activity) -> Result<i64, ActivityStorageError> {
        let category_str = activity.category.as_str().to_string();
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn
            .call(move |conn| {
                // Use INSERT OR REPLACE to handle upsert based on capture_id UNIQUE constraint
                conn.execute(
                    r#"
                    INSERT INTO activities (capture_id, timestamp, application_name, activity_description, category, created_at)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    ON CONFLICT(capture_id) DO UPDATE SET
                        timestamp = excluded.timestamp,
                        application_name = excluded.application_name,
                        activity_description = excluded.activity_description,
                        category = excluded.category
                    "#,
                    rusqlite::params![
                        activity.capture_id,
                        activity.timestamp,
                        activity.application_name,
                        activity.activity_description,
                        category_str,
                        created_at,
                    ],
                )?;

                let id = conn.last_insert_rowid();
                Ok(id)
            })
            .await
            .map_err(|e| ActivityStorageError::DatabaseError(e.to_string()))
    }

    async fn get_activities_by_date_range(
        &self,
        from: i64,
        to: i64,
    ) -> Result<Vec<Activity>, ActivityStorageError> {
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT id, capture_id, timestamp, application_name, activity_description, category, created_at
                    FROM activities
                    WHERE timestamp >= ?1 AND timestamp < ?2
                    ORDER BY timestamp ASC
                    "#,
                )?;

                let rows = stmt.query_map([from, to], |row| {
                    let category_str: String = row.get(5)?;
                    Ok(Activity {
                        id: Some(row.get(0)?),
                        capture_id: row.get(1)?,
                        timestamp: row.get(2)?,
                        application_name: row.get(3)?,
                        activity_description: row.get(4)?,
                        category: ActivityCategory::from_str(&category_str),
                        created_at: row.get(6)?,
                    })
                })?;

                let mut activities = Vec::new();
                for row in rows {
                    activities.push(row?);
                }
                Ok(activities)
            })
            .await
            .map_err(|e| ActivityStorageError::DatabaseError(e.to_string()))
    }

    async fn get_today_activities(&self) -> Result<Vec<Activity>, ActivityStorageError> {
        let from = get_today_start();
        let to = get_today_end();
        self.get_activities_by_date_range(from, to).await
    }

    async fn get_unanalyzed_capture_ids(&self) -> Result<Vec<i64>, ActivityStorageError> {
        self.conn
            .call(|conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT c.id
                    FROM captures c
                    LEFT JOIN activities a ON c.id = a.capture_id
                    WHERE a.id IS NULL
                    ORDER BY c.timestamp ASC
                    "#,
                )?;

                let rows = stmt.query_map([], |row| row.get(0))?;

                let mut ids = Vec::new();
                for row in rows {
                    ids.push(row?);
                }
                Ok(ids)
            })
            .await
            .map_err(|e| ActivityStorageError::DatabaseError(e.to_string()))
    }

    async fn get_unanalyzed_captures(
        &self,
        limit: usize,
    ) -> Result<Vec<CaptureForAnalysis>, ActivityStorageError> {
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT c.id, c.timestamp, c.file_path
                    FROM captures c
                    LEFT JOIN activities a ON c.id = a.capture_id
                    WHERE a.id IS NULL
                    ORDER BY c.timestamp ASC
                    LIMIT ?1
                    "#,
                )?;

                let rows = stmt.query_map([limit as i64], |row| {
                    Ok(CaptureForAnalysis {
                        id: row.get(0)?,
                        timestamp: row.get(1)?,
                        file_path: row.get(2)?,
                    })
                })?;

                let mut captures = Vec::new();
                for row in rows {
                    captures.push(row?);
                }
                Ok(captures)
            })
            .await
            .map_err(|e| ActivityStorageError::DatabaseError(e.to_string()))
    }

    async fn get_stats_by_application(
        &self,
        from: i64,
        to: i64,
    ) -> Result<Vec<ApplicationStats>, ActivityStorageError> {
        self.conn
            .call(move |conn| {
                // First, get the capture interval from config or use default (60 seconds)
                // For now, we assume each activity represents the capture interval worth of time
                let capture_interval_minutes = 1u64; // Default 60 seconds = 1 minute

                let mut stmt = conn.prepare(
                    r#"
                    SELECT application_name, COUNT(*) as count
                    FROM activities
                    WHERE timestamp >= ?1 AND timestamp < ?2
                    GROUP BY application_name
                    ORDER BY count DESC
                    "#,
                )?;

                let rows = stmt.query_map([from, to], |row| {
                    let count: usize = row.get(1)?;
                    Ok(ApplicationStats {
                        application_name: row.get(0)?,
                        count,
                        estimated_minutes: count as u64 * capture_interval_minutes,
                    })
                })?;

                let mut stats = Vec::new();
                for row in rows {
                    stats.push(row?);
                }
                Ok(stats)
            })
            .await
            .map_err(|e| ActivityStorageError::DatabaseError(e.to_string()))
    }

    async fn get_stats_by_category(
        &self,
        from: i64,
        to: i64,
    ) -> Result<Vec<CategoryStats>, ActivityStorageError> {
        self.conn
            .call(move |conn| {
                let capture_interval_minutes = 1u64;

                let mut stmt = conn.prepare(
                    r#"
                    SELECT category, COUNT(*) as count
                    FROM activities
                    WHERE timestamp >= ?1 AND timestamp < ?2
                    GROUP BY category
                    ORDER BY count DESC
                    "#,
                )?;

                let rows = stmt.query_map([from, to], |row| {
                    let category_str: String = row.get(0)?;
                    let count: usize = row.get(1)?;
                    Ok(CategoryStats {
                        category: ActivityCategory::from_str(&category_str),
                        count,
                        estimated_minutes: count as u64 * capture_interval_minutes,
                    })
                })?;

                let mut stats = Vec::new();
                for row in rows {
                    stats.push(row?);
                }
                Ok(stats)
            })
            .await
            .map_err(|e| ActivityStorageError::DatabaseError(e.to_string()))
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

    // === ActivityStoragePort Tests ===

    #[tokio::test]
    async fn test_activities_table_exists() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        let table_exists = adapter
            .conn
            .call(|conn| {
                let count: i32 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='activities'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count > 0)
            })
            .await
            .expect("Failed to query");

        assert!(table_exists, "activities table should exist");
    }

    #[tokio::test]
    async fn test_activities_indexes_exist() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        for index_name in &[
            "idx_activities_timestamp",
            "idx_activities_category",
            "idx_activities_application",
        ] {
            let has_index = adapter
                .conn
                .call(move |conn| {
                    let count: i32 = conn.query_row(
                        &format!(
                            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='{}'",
                            index_name
                        ),
                        [],
                        |row| row.get(0),
                    )?;
                    Ok(count > 0)
                })
                .await
                .expect("Failed to query index");

            assert!(has_index, "{} index should exist", index_name);
        }
    }

    #[tokio::test]
    async fn test_save_and_retrieve_activity() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // First, create a capture to reference
        let capture = CaptureMetadata {
            id: None,
            timestamp: 1704326400,
            file_path: "/captures/test.png".to_string(),
            created_at: 1704326400,
        };
        adapter
            .save_metadata(capture)
            .await
            .expect("Failed to save capture");

        let captures = adapter
            .get_recent_captures(1)
            .await
            .expect("Failed to get captures");
        let capture_id = captures[0].id.unwrap();

        // Now save an activity
        let activity = Activity {
            id: None,
            capture_id,
            timestamp: 1704326400,
            application_name: "VS Code".to_string(),
            activity_description: "Writing Rust code".to_string(),
            category: ActivityCategory::Coding,
            created_at: 1704326400,
        };

        let activity_id = adapter
            .save_activity(activity)
            .await
            .expect("Failed to save activity");
        assert!(activity_id > 0);

        // Retrieve activities
        let activities = adapter
            .get_activities_by_date_range(1704326000, 1704327000)
            .await
            .expect("Failed to get activities");

        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].application_name, "VS Code");
        assert_eq!(activities[0].category, ActivityCategory::Coding);
    }

    #[tokio::test]
    async fn test_activity_upsert_on_same_capture_id() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // Create a capture
        let capture = CaptureMetadata {
            id: None,
            timestamp: 1704326400,
            file_path: "/captures/test.png".to_string(),
            created_at: 1704326400,
        };
        adapter
            .save_metadata(capture)
            .await
            .expect("Failed to save capture");

        let captures = adapter
            .get_recent_captures(1)
            .await
            .expect("Failed to get captures");
        let capture_id = captures[0].id.unwrap();

        // Save first activity
        let activity1 = Activity {
            id: None,
            capture_id,
            timestamp: 1704326400,
            application_name: "Chrome".to_string(),
            activity_description: "Browsing".to_string(),
            category: ActivityCategory::Browsing,
            created_at: 1704326400,
        };
        adapter
            .save_activity(activity1)
            .await
            .expect("Failed to save first activity");

        // Save second activity with same capture_id (should update)
        let activity2 = Activity {
            id: None,
            capture_id,
            timestamp: 1704326400,
            application_name: "VS Code".to_string(),
            activity_description: "Coding".to_string(),
            category: ActivityCategory::Coding,
            created_at: 1704326400,
        };
        adapter
            .save_activity(activity2)
            .await
            .expect("Failed to save second activity");

        // Should only have one activity (updated)
        let activities = adapter
            .get_activities_by_date_range(1704326000, 1704327000)
            .await
            .expect("Failed to get activities");

        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].application_name, "VS Code");
        assert_eq!(activities[0].category, ActivityCategory::Coding);
    }

    #[tokio::test]
    async fn test_get_unanalyzed_capture_ids() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // Create 3 captures
        for i in 0..3 {
            let capture = CaptureMetadata {
                id: None,
                timestamp: 1704326400 + i * 60,
                file_path: format!("/captures/test_{}.png", i),
                created_at: 1704326400 + i * 60,
            };
            adapter
                .save_metadata(capture)
                .await
                .expect("Failed to save capture");
        }

        // All 3 should be unanalyzed
        let unanalyzed = adapter
            .get_unanalyzed_capture_ids()
            .await
            .expect("Failed to get unanalyzed");
        assert_eq!(unanalyzed.len(), 3);

        // Analyze one capture
        let activity = Activity {
            id: None,
            capture_id: unanalyzed[0],
            timestamp: 1704326400,
            application_name: "Test".to_string(),
            activity_description: "Testing".to_string(),
            category: ActivityCategory::Other("test".to_string()),
            created_at: 1704326400,
        };
        adapter
            .save_activity(activity)
            .await
            .expect("Failed to save activity");

        // Now only 2 should be unanalyzed
        let unanalyzed_after = adapter
            .get_unanalyzed_capture_ids()
            .await
            .expect("Failed to get unanalyzed");
        assert_eq!(unanalyzed_after.len(), 2);
    }

    #[tokio::test]
    async fn test_get_stats_by_application() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // Create captures and activities
        let apps = vec!["VS Code", "VS Code", "Chrome", "VS Code", "Slack"];
        for (i, app) in apps.iter().enumerate() {
            let capture = CaptureMetadata {
                id: None,
                timestamp: 1704326400 + (i as i64) * 60,
                file_path: format!("/captures/test_{}.png", i),
                created_at: 1704326400 + (i as i64) * 60,
            };
            adapter
                .save_metadata(capture)
                .await
                .expect("Failed to save capture");

            let captures = adapter
                .get_recent_captures(1)
                .await
                .expect("Failed to get captures");
            let capture_id = captures[0].id.unwrap();

            let activity = Activity {
                id: None,
                capture_id,
                timestamp: 1704326400 + (i as i64) * 60,
                application_name: app.to_string(),
                activity_description: "Working".to_string(),
                category: ActivityCategory::Coding,
                created_at: 1704326400,
            };
            adapter
                .save_activity(activity)
                .await
                .expect("Failed to save activity");
        }

        let stats = adapter
            .get_stats_by_application(1704326000, 1704330000)
            .await
            .expect("Failed to get stats");

        // VS Code should be first (3 occurrences)
        assert_eq!(stats[0].application_name, "VS Code");
        assert_eq!(stats[0].count, 3);

        // Chrome and Slack should have 1 each
        assert!(stats
            .iter()
            .any(|s| s.application_name == "Chrome" && s.count == 1));
        assert!(stats
            .iter()
            .any(|s| s.application_name == "Slack" && s.count == 1));
    }

    #[tokio::test]
    async fn test_get_stats_by_category() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        let categories = vec![
            ActivityCategory::Coding,
            ActivityCategory::Coding,
            ActivityCategory::Communication,
            ActivityCategory::Coding,
            ActivityCategory::Documentation,
        ];

        for (i, cat) in categories.iter().enumerate() {
            let capture = CaptureMetadata {
                id: None,
                timestamp: 1704326400 + (i as i64) * 60,
                file_path: format!("/captures/test_{}.png", i),
                created_at: 1704326400 + (i as i64) * 60,
            };
            adapter
                .save_metadata(capture)
                .await
                .expect("Failed to save capture");

            let captures = adapter
                .get_recent_captures(1)
                .await
                .expect("Failed to get captures");
            let capture_id = captures[0].id.unwrap();

            let activity = Activity {
                id: None,
                capture_id,
                timestamp: 1704326400 + (i as i64) * 60,
                application_name: "App".to_string(),
                activity_description: "Working".to_string(),
                category: cat.clone(),
                created_at: 1704326400,
            };
            adapter
                .save_activity(activity)
                .await
                .expect("Failed to save activity");
        }

        let stats = adapter
            .get_stats_by_category(1704326000, 1704330000)
            .await
            .expect("Failed to get stats");

        // Coding should be first (3 occurrences)
        assert_eq!(stats[0].category, ActivityCategory::Coding);
        assert_eq!(stats[0].count, 3);

        // Communication and Documentation should have 1 each
        assert!(stats
            .iter()
            .any(|s| s.category == ActivityCategory::Communication && s.count == 1));
        assert!(stats
            .iter()
            .any(|s| s.category == ActivityCategory::Documentation && s.count == 1));
    }

    #[tokio::test]
    async fn test_get_unanalyzed_captures() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // Create 3 captures
        for i in 0..3 {
            let capture = CaptureMetadata {
                id: None,
                timestamp: 1704326400 + i * 60,
                file_path: format!("/captures/test_{}.png", i),
                created_at: 1704326400 + i * 60,
            };
            adapter
                .save_metadata(capture)
                .await
                .expect("Failed to save capture");
        }

        // All 3 should be unanalyzed with full metadata
        let unanalyzed = adapter
            .get_unanalyzed_captures(10)
            .await
            .expect("Failed to get unanalyzed");
        assert_eq!(unanalyzed.len(), 3);
        assert!(unanalyzed[0].file_path.contains("test_0"));

        // Analyze one capture
        let activity = Activity {
            id: None,
            capture_id: unanalyzed[0].id,
            timestamp: 1704326400,
            application_name: "Test".to_string(),
            activity_description: "Testing".to_string(),
            category: ActivityCategory::Other("test".to_string()),
            created_at: 1704326400,
        };
        adapter
            .save_activity(activity)
            .await
            .expect("Failed to save activity");

        // Now only 2 should be unanalyzed
        let unanalyzed_after = adapter
            .get_unanalyzed_captures(10)
            .await
            .expect("Failed to get unanalyzed");
        assert_eq!(unanalyzed_after.len(), 2);
    }

    #[tokio::test]
    async fn test_get_unanalyzed_captures_respects_limit() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // Create 5 captures
        for i in 0..5 {
            let capture = CaptureMetadata {
                id: None,
                timestamp: 1704326400 + i * 60,
                file_path: format!("/captures/test_{}.png", i),
                created_at: 1704326400 + i * 60,
            };
            adapter
                .save_metadata(capture)
                .await
                .expect("Failed to save capture");
        }

        // Request only 2
        let unanalyzed = adapter
            .get_unanalyzed_captures(2)
            .await
            .expect("Failed to get unanalyzed");
        assert_eq!(unanalyzed.len(), 2);

        // Should be ordered by timestamp ASC (oldest first)
        assert!(unanalyzed[0].timestamp < unanalyzed[1].timestamp);
    }

    #[tokio::test]
    async fn test_activity_cascade_delete() {
        let adapter = SqliteAdapter::new_in_memory()
            .await
            .expect("Failed to create adapter");

        // Create a capture
        let capture = CaptureMetadata {
            id: None,
            timestamp: 1704326400,
            file_path: "/captures/test.png".to_string(),
            created_at: 1704326400,
        };
        adapter
            .save_metadata(capture)
            .await
            .expect("Failed to save capture");

        let captures = adapter
            .get_recent_captures(1)
            .await
            .expect("Failed to get captures");
        let capture_id = captures[0].id.unwrap();

        // Create an activity
        let activity = Activity {
            id: None,
            capture_id,
            timestamp: 1704326400,
            application_name: "Test".to_string(),
            activity_description: "Testing".to_string(),
            category: ActivityCategory::Coding,
            created_at: 1704326400,
        };
        adapter
            .save_activity(activity)
            .await
            .expect("Failed to save activity");

        // Verify activity exists
        let activities = adapter
            .get_activities_by_date_range(1704326000, 1704327000)
            .await
            .expect("Failed to get activities");
        assert_eq!(activities.len(), 1);

        // Delete the capture
        adapter
            .delete_capture(capture_id)
            .await
            .expect("Failed to delete capture");

        // Activity should be deleted due to CASCADE
        let activities_after = adapter
            .get_activities_by_date_range(1704326000, 1704327000)
            .await
            .expect("Failed to get activities");
        assert_eq!(activities_after.len(), 0);
    }
}
