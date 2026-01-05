//! Directory management for Herald
//!
//! Handles initialization and management of the Herald data directory structure.

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::error::ConfigError;

/// Required subdirectories within the Herald data directory
const SUBDIRECTORIES: &[&str] = &["captures", "logs"];

/// Directory permission mode (owner read/write/execute only)
#[cfg(unix)]
const DIR_PERMISSION_MODE: u32 = 0o700;

/// Manages the Herald data directory structure
#[derive(Debug, Clone)]
pub struct DirectoryManager {
    /// Base data directory (e.g., ~/.herald/)
    data_dir: PathBuf,
}

impl DirectoryManager {
    /// Creates a new DirectoryManager with the specified data directory
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Creates a DirectoryManager with the default data directory (~/.herald/)
    pub fn with_default_dir() -> Self {
        let data_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".herald");
        Self::new(data_dir)
    }

    /// Returns the base data directory path
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Returns the captures directory path
    pub fn captures_dir(&self) -> PathBuf {
        self.data_dir.join("captures")
    }

    /// Returns the logs directory path
    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// Returns the database file path
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("herald.db")
    }

    /// Returns the config file path
    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join("config.toml")
    }

    /// Returns the PID file path
    pub fn pid_file_path(&self) -> PathBuf {
        self.data_dir.join("daemon.pid")
    }

    /// Initializes the directory structure
    ///
    /// Creates the base data directory and all required subdirectories
    /// with appropriate permissions (700 on Unix systems).
    ///
    /// # Errors
    /// Returns `ConfigError::Io` if directory creation fails
    pub fn initialize(&self) -> Result<(), ConfigError> {
        // Create base directory
        self.create_directory_with_permissions(&self.data_dir)?;

        // Create subdirectories
        for subdir in SUBDIRECTORIES {
            let path = self.data_dir.join(subdir);
            self.create_directory_with_permissions(&path)?;
        }

        tracing::info!("Initialized Herald data directory at {:?}", self.data_dir);
        Ok(())
    }

    /// Creates a directory with appropriate permissions
    fn create_directory_with_permissions(&self, path: &Path) -> Result<(), ConfigError> {
        if path.exists() {
            // Directory already exists, ensure permissions are correct
            #[cfg(unix)]
            self.set_unix_permissions(path)?;
            return Ok(());
        }

        // Create directory
        fs::create_dir_all(path)?;

        // Set permissions on Unix
        #[cfg(unix)]
        self.set_unix_permissions(path)?;

        tracing::debug!("Created directory: {:?}", path);
        Ok(())
    }

    /// Sets Unix permissions (700) on a directory
    #[cfg(unix)]
    fn set_unix_permissions(&self, path: &Path) -> Result<(), ConfigError> {
        let permissions = fs::Permissions::from_mode(DIR_PERMISSION_MODE);
        fs::set_permissions(path, permissions)?;
        Ok(())
    }

    /// Checks if the directory structure is properly initialized
    pub fn is_initialized(&self) -> bool {
        if !self.data_dir.exists() {
            return false;
        }

        for subdir in SUBDIRECTORIES {
            if !self.data_dir.join(subdir).exists() {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // === Task 3.1 TDD Tests ===

    #[test]
    fn test_new_directory_manager() {
        let path = PathBuf::from("/tmp/test-herald");
        let manager = DirectoryManager::new(path.clone());
        assert_eq!(manager.data_dir(), path);
    }

    #[test]
    fn test_with_default_dir() {
        let manager = DirectoryManager::with_default_dir();
        let path = manager.data_dir();
        assert!(path.ends_with(".herald"));
    }

    #[test]
    fn test_captures_dir_path() {
        let path = PathBuf::from("/tmp/test-herald");
        let manager = DirectoryManager::new(path);
        assert_eq!(
            manager.captures_dir(),
            PathBuf::from("/tmp/test-herald/captures")
        );
    }

    #[test]
    fn test_logs_dir_path() {
        let path = PathBuf::from("/tmp/test-herald");
        let manager = DirectoryManager::new(path);
        assert_eq!(manager.logs_dir(), PathBuf::from("/tmp/test-herald/logs"));
    }

    #[test]
    fn test_database_path() {
        let path = PathBuf::from("/tmp/test-herald");
        let manager = DirectoryManager::new(path);
        assert_eq!(
            manager.database_path(),
            PathBuf::from("/tmp/test-herald/herald.db")
        );
    }

    #[test]
    fn test_config_path() {
        let path = PathBuf::from("/tmp/test-herald");
        let manager = DirectoryManager::new(path);
        assert_eq!(
            manager.config_path(),
            PathBuf::from("/tmp/test-herald/config.toml")
        );
    }

    #[test]
    fn test_pid_file_path() {
        let path = PathBuf::from("/tmp/test-herald");
        let manager = DirectoryManager::new(path);
        assert_eq!(
            manager.pid_file_path(),
            PathBuf::from("/tmp/test-herald/daemon.pid")
        );
    }

    #[test]
    fn test_initialize_creates_directories() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join(".herald");
        let manager = DirectoryManager::new(data_dir.clone());

        // Before initialization
        assert!(!data_dir.exists());
        assert!(!manager.is_initialized());

        // Initialize
        manager
            .initialize()
            .expect("Failed to initialize directories");

        // After initialization
        assert!(data_dir.exists());
        assert!(data_dir.join("captures").exists());
        assert!(data_dir.join("logs").exists());
        assert!(manager.is_initialized());
    }

    #[test]
    fn test_initialize_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join(".herald");
        let manager = DirectoryManager::new(data_dir.clone());

        // Initialize twice - should not error
        manager.initialize().expect("First initialization failed");
        manager.initialize().expect("Second initialization failed");

        assert!(manager.is_initialized());
    }

    #[test]
    fn test_initialize_skips_existing_directories() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join(".herald");

        // Pre-create directories manually
        fs::create_dir_all(data_dir.join("captures")).unwrap();

        let manager = DirectoryManager::new(data_dir.clone());
        manager
            .initialize()
            .expect("Failed to initialize with existing dir");

        // All directories should exist
        assert!(data_dir.join("captures").exists());
        assert!(data_dir.join("logs").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_initialize_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join(".herald");
        let manager = DirectoryManager::new(data_dir.clone());

        manager
            .initialize()
            .expect("Failed to initialize directories");

        // Check permissions (700 = owner read/write/execute only)
        let metadata = fs::metadata(&data_dir).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "Base directory should have 700 permissions");

        let captures_metadata = fs::metadata(data_dir.join("captures")).unwrap();
        let captures_mode = captures_metadata.permissions().mode() & 0o777;
        assert_eq!(
            captures_mode, 0o700,
            "Captures directory should have 700 permissions"
        );

        let logs_metadata = fs::metadata(data_dir.join("logs")).unwrap();
        let logs_mode = logs_metadata.permissions().mode() & 0o777;
        assert_eq!(
            logs_mode, 0o700,
            "Logs directory should have 700 permissions"
        );
    }

    #[test]
    fn test_is_initialized_false_when_missing_subdirs() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join(".herald");

        // Create only base directory
        fs::create_dir_all(&data_dir).unwrap();

        let manager = DirectoryManager::new(data_dir);
        assert!(!manager.is_initialized());
    }

    #[test]
    fn test_is_initialized_false_when_no_directory() {
        let manager = DirectoryManager::new(PathBuf::from("/nonexistent/path/.herald"));
        assert!(!manager.is_initialized());
    }
}
