//! Daemon management for Herald
//!
//! Handles PID file management, daemon start/stop operations, and process state tracking.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::sync::watch;

/// Errors that can occur during daemon operations
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Failed to read PID file
    #[error("Failed to read PID file: {0}")]
    PidFileReadError(String),

    /// Failed to write PID file
    #[error("Failed to write PID file: {0}")]
    PidFileWriteError(String),

    /// Failed to delete PID file
    #[error("Failed to delete PID file: {0}")]
    PidFileDeleteError(String),

    /// Invalid PID value in file
    #[error("Invalid PID value: {0}")]
    InvalidPid(String),

    /// Daemon is already running
    #[error("Daemon is already running (PID: {0})")]
    AlreadyRunning(u32),

    /// Daemon is not running
    #[error("Daemon is not running")]
    NotRunning,

    /// Failed to send signal to process
    #[error("Failed to send signal to process: {0}")]
    SignalError(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

/// Manages PID file operations for the Herald daemon
#[derive(Debug, Clone)]
pub struct PidManager {
    /// Path to the PID file
    pid_file_path: PathBuf,
}

impl PidManager {
    /// Creates a new PidManager with the specified PID file path
    pub fn new(pid_file_path: PathBuf) -> Self {
        Self { pid_file_path }
    }

    /// Returns the path to the PID file
    pub fn pid_file_path(&self) -> &Path {
        &self.pid_file_path
    }

    /// Writes the current process ID to the PID file
    ///
    /// # Arguments
    /// * `pid` - The process ID to write
    ///
    /// # Errors
    /// Returns `DaemonError::PidFileWriteError` if the file cannot be written
    pub fn write_pid(&self, pid: u32) -> Result<(), DaemonError> {
        let mut file = fs::File::create(&self.pid_file_path).map_err(|e| {
            DaemonError::PidFileWriteError(format!(
                "Failed to create PID file {:?}: {}",
                self.pid_file_path, e
            ))
        })?;

        write!(file, "{}", pid).map_err(|e| {
            DaemonError::PidFileWriteError(format!("Failed to write PID to file: {}", e))
        })?;

        tracing::debug!("Wrote PID {} to {:?}", pid, self.pid_file_path);
        Ok(())
    }

    /// Reads the process ID from the PID file
    ///
    /// # Returns
    /// * `Ok(Some(pid))` - PID file exists and contains a valid PID
    /// * `Ok(None)` - PID file does not exist
    /// * `Err(DaemonError)` - Error reading or parsing the PID file
    pub fn read_pid(&self) -> Result<Option<u32>, DaemonError> {
        if !self.pid_file_path.exists() {
            return Ok(None);
        }

        let mut file = fs::File::open(&self.pid_file_path).map_err(|e| {
            DaemonError::PidFileReadError(format!(
                "Failed to open PID file {:?}: {}",
                self.pid_file_path, e
            ))
        })?;

        let mut content = String::new();
        file.read_to_string(&mut content).map_err(|e| {
            DaemonError::PidFileReadError(format!("Failed to read PID file content: {}", e))
        })?;

        let pid: u32 = content.trim().parse().map_err(|e| {
            DaemonError::InvalidPid(format!("Invalid PID '{}': {}", content.trim(), e))
        })?;

        Ok(Some(pid))
    }

    /// Deletes the PID file
    ///
    /// # Returns
    /// * `Ok(true)` - PID file was deleted
    /// * `Ok(false)` - PID file did not exist
    /// * `Err(DaemonError)` - Error deleting the PID file
    pub fn delete_pid(&self) -> Result<bool, DaemonError> {
        if !self.pid_file_path.exists() {
            return Ok(false);
        }

        fs::remove_file(&self.pid_file_path).map_err(|e| {
            DaemonError::PidFileDeleteError(format!(
                "Failed to delete PID file {:?}: {}",
                self.pid_file_path, e
            ))
        })?;

        tracing::debug!("Deleted PID file {:?}", self.pid_file_path);
        Ok(true)
    }

    /// Checks if the PID file exists
    pub fn pid_file_exists(&self) -> bool {
        self.pid_file_path.exists()
    }

    /// Checks if a process with the given PID is running
    ///
    /// Uses `kill(pid, 0)` on Unix systems to check process existence
    /// without sending an actual signal.
    #[cfg(unix)]
    pub fn is_process_running(&self, pid: u32) -> bool {
        // Use kill(pid, 0) to check if process exists
        // This sends no signal but returns error if process doesn't exist
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    #[cfg(not(unix))]
    pub fn is_process_running(&self, _pid: u32) -> bool {
        // Non-Unix platforms: assume process is running if PID file exists
        true
    }

    /// Checks if the daemon is currently running
    ///
    /// Reads the PID file and verifies the process is actually running.
    /// If the PID file exists but the process is not running (stale PID file),
    /// the PID file is automatically cleaned up.
    ///
    /// # Returns
    /// * `Ok(Some(pid))` - Daemon is running with the given PID
    /// * `Ok(None)` - Daemon is not running
    /// * `Err(DaemonError)` - Error checking daemon status
    pub fn is_daemon_running(&self) -> Result<Option<u32>, DaemonError> {
        match self.read_pid()? {
            Some(pid) => {
                if self.is_process_running(pid) {
                    Ok(Some(pid))
                } else {
                    // Stale PID file - clean it up
                    tracing::info!(
                        "Found stale PID file (process {} not running), cleaning up",
                        pid
                    );
                    self.delete_pid()?;
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    /// Sends a signal to the process with the given PID
    ///
    /// # Arguments
    /// * `pid` - The process ID to signal
    /// * `signal` - The signal number to send (e.g., SIGTERM = 15)
    ///
    /// # Returns
    /// * `Ok(())` - Signal sent successfully
    /// * `Err(DaemonError)` - Failed to send signal
    #[cfg(unix)]
    pub fn send_signal(&self, pid: u32, signal: i32) -> Result<(), DaemonError> {
        let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
        if result == 0 {
            Ok(())
        } else {
            let errno = std::io::Error::last_os_error();
            Err(DaemonError::SignalError(format!(
                "Failed to send signal {} to process {}: {}",
                signal, pid, errno
            )))
        }
    }

    #[cfg(not(unix))]
    pub fn send_signal(&self, _pid: u32, _signal: i32) -> Result<(), DaemonError> {
        Err(DaemonError::SignalError(
            "Signal sending not supported on this platform".to_string(),
        ))
    }
}

/// Result of a daemon start operation
#[derive(Debug)]
pub struct DaemonStartResult {
    /// The PID of the started daemon
    pub pid: u32,
    /// Receiver for shutdown signal
    pub shutdown_rx: watch::Receiver<bool>,
}

/// Sender for shutdown signal
pub type ShutdownSender = watch::Sender<bool>;

/// Controls the Herald daemon lifecycle
#[derive(Debug)]
pub struct DaemonController {
    /// PID file manager
    pid_manager: PidManager,
    /// Shutdown signal sender (only present when daemon is running in this process)
    shutdown_tx: Option<ShutdownSender>,
}

impl DaemonController {
    /// Creates a new DaemonController with the specified PID file path
    pub fn new(pid_file_path: PathBuf) -> Self {
        Self {
            pid_manager: PidManager::new(pid_file_path),
            shutdown_tx: None,
        }
    }

    /// Creates a DaemonController from an existing PidManager
    pub fn from_pid_manager(pid_manager: PidManager) -> Self {
        Self {
            pid_manager,
            shutdown_tx: None,
        }
    }

    /// Returns a reference to the PID manager
    pub fn pid_manager(&self) -> &PidManager {
        &self.pid_manager
    }

    /// Checks if the daemon is currently running
    pub fn is_running(&self) -> Result<Option<u32>, DaemonError> {
        self.pid_manager.is_daemon_running()
    }

    /// Registers the current process as the daemon
    ///
    /// This should be called when starting the daemon to:
    /// 1. Check if another daemon is already running
    /// 2. Create the PID file with the current process ID
    /// 3. Set up the shutdown signal channel
    ///
    /// # Returns
    /// * `Ok(DaemonStartResult)` - Daemon registered successfully
    /// * `Err(DaemonError::AlreadyRunning)` - Another daemon is already running
    /// * `Err(DaemonError)` - Other errors
    pub fn register_daemon(&mut self) -> Result<DaemonStartResult, DaemonError> {
        // Check if daemon is already running
        if let Some(pid) = self.pid_manager.is_daemon_running()? {
            return Err(DaemonError::AlreadyRunning(pid));
        }

        // Get current process ID
        let current_pid = std::process::id();

        // Write PID file
        self.pid_manager.write_pid(current_pid)?;

        // Create shutdown signal channel
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        tracing::info!("Daemon registered with PID {}", current_pid);

        Ok(DaemonStartResult {
            pid: current_pid,
            shutdown_rx,
        })
    }

    /// Signals the daemon to shut down gracefully
    ///
    /// If the daemon is running in this process, uses the shutdown channel.
    /// Otherwise, sends SIGTERM to the daemon process.
    ///
    /// # Returns
    /// * `Ok(pid)` - Shutdown signal sent to daemon with given PID
    /// * `Err(DaemonError::NotRunning)` - Daemon is not running
    /// * `Err(DaemonError)` - Other errors
    pub fn stop_daemon(&mut self) -> Result<u32, DaemonError> {
        // Check if daemon is running
        let pid = match self.pid_manager.is_daemon_running()? {
            Some(pid) => pid,
            None => return Err(DaemonError::NotRunning),
        };

        // If we have the shutdown sender, use it (daemon running in this process)
        if let Some(ref shutdown_tx) = self.shutdown_tx {
            let _ = shutdown_tx.send(true);
            tracing::info!("Sent shutdown signal to daemon (PID: {})", pid);
        } else {
            // Send SIGTERM to the external process
            #[cfg(unix)]
            {
                self.pid_manager.send_signal(pid, libc::SIGTERM)?;
                tracing::info!("Sent SIGTERM to daemon (PID: {})", pid);
            }
            #[cfg(not(unix))]
            {
                return Err(DaemonError::SignalError(
                    "Cannot stop external daemon on this platform".to_string(),
                ));
            }
        }

        Ok(pid)
    }

    /// Unregisters the daemon and cleans up the PID file
    ///
    /// This should be called when the daemon shuts down.
    pub fn unregister_daemon(&mut self) -> Result<(), DaemonError> {
        self.shutdown_tx = None;
        self.pid_manager.delete_pid()?;
        tracing::info!("Daemon unregistered");
        Ok(())
    }

    /// Creates a shutdown receiver that can be used to listen for shutdown signals
    ///
    /// This is useful for passing to background tasks that need to respond to shutdown.
    pub fn shutdown_receiver(&self) -> Option<watch::Receiver<bool>> {
        self.shutdown_tx.as_ref().map(|tx| tx.subscribe())
    }

    /// Triggers a graceful shutdown (for use when daemon is running in this process)
    pub fn trigger_shutdown(&self) -> bool {
        if let Some(ref shutdown_tx) = self.shutdown_tx {
            let _ = shutdown_tx.send(true);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // === Task 10.1 TDD Tests: PID File Management ===

    #[test]
    fn test_pid_manager_new() {
        let path = PathBuf::from("/tmp/herald/daemon.pid");
        let manager = PidManager::new(path.clone());
        assert_eq!(manager.pid_file_path(), path);
    }

    #[test]
    fn test_write_and_read_pid() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path.clone());

        // Write PID
        let test_pid = 12345u32;
        manager.write_pid(test_pid).expect("Failed to write PID");

        // Verify file exists
        assert!(pid_path.exists());

        // Read PID
        let read_pid = manager.read_pid().expect("Failed to read PID");
        assert_eq!(read_pid, Some(test_pid));
    }

    #[test]
    fn test_read_pid_file_not_exists() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("nonexistent.pid");
        let manager = PidManager::new(pid_path);

        let result = manager.read_pid().expect("Read should not error");
        assert_eq!(result, None);
    }

    #[test]
    fn test_read_pid_invalid_content() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");

        // Write invalid content
        fs::write(&pid_path, "not_a_number").unwrap();

        let manager = PidManager::new(pid_path);
        let result = manager.read_pid();

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DaemonError::InvalidPid(_)));
    }

    #[test]
    fn test_delete_pid_existing_file() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path.clone());

        // Create PID file
        manager.write_pid(12345).expect("Failed to write PID");
        assert!(pid_path.exists());

        // Delete PID file
        let deleted = manager.delete_pid().expect("Failed to delete PID");
        assert!(deleted);
        assert!(!pid_path.exists());
    }

    #[test]
    fn test_delete_pid_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path);

        // Try to delete non-existent file
        let deleted = manager.delete_pid().expect("Delete should not error");
        assert!(!deleted);
    }

    #[test]
    fn test_pid_file_exists() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path.clone());

        // Initially doesn't exist
        assert!(!manager.pid_file_exists());

        // After writing
        manager.write_pid(12345).expect("Failed to write PID");
        assert!(manager.pid_file_exists());

        // After deleting
        manager.delete_pid().expect("Failed to delete PID");
        assert!(!manager.pid_file_exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_is_process_running_current_process() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path);

        // Current process should be running
        let current_pid = std::process::id();
        assert!(manager.is_process_running(current_pid));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_process_running_nonexistent_process() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path);

        // Very high PID unlikely to exist
        // Note: This may fail if running as root
        let fake_pid = 999999u32;
        assert!(!manager.is_process_running(fake_pid));
    }

    #[test]
    fn test_is_daemon_running_not_running() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path);

        // No PID file - daemon not running
        let result = manager.is_daemon_running().expect("Check should not error");
        assert_eq!(result, None);
    }

    #[cfg(unix)]
    #[test]
    fn test_is_daemon_running_with_current_process() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path);

        // Write current process PID
        let current_pid = std::process::id();
        manager.write_pid(current_pid).expect("Failed to write PID");

        // Should detect as running
        let result = manager.is_daemon_running().expect("Check should not error");
        assert_eq!(result, Some(current_pid));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_daemon_running_stale_pid_file_cleanup() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path.clone());

        // Write a fake PID (non-existent process)
        let fake_pid = 999999u32;
        manager.write_pid(fake_pid).expect("Failed to write PID");
        assert!(pid_path.exists());

        // Should detect as not running and clean up stale file
        let result = manager.is_daemon_running().expect("Check should not error");
        assert_eq!(result, None);
        assert!(!pid_path.exists(), "Stale PID file should be cleaned up");
    }

    #[test]
    fn test_write_pid_overwrites_existing() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path);

        // Write first PID
        manager.write_pid(11111).expect("Failed to write first PID");
        assert_eq!(manager.read_pid().unwrap(), Some(11111));

        // Write second PID (should overwrite)
        manager
            .write_pid(22222)
            .expect("Failed to write second PID");
        assert_eq!(manager.read_pid().unwrap(), Some(22222));
    }

    #[test]
    fn test_daemon_error_display() {
        let err = DaemonError::AlreadyRunning(12345);
        assert!(err.to_string().contains("12345"));

        let err = DaemonError::NotRunning;
        assert!(err.to_string().contains("not running"));

        let err = DaemonError::InvalidPid("abc".to_string());
        assert!(err.to_string().contains("abc"));
    }

    // === Task 10.2 & 10.3 TDD Tests: Daemon Controller ===

    #[test]
    fn test_daemon_controller_new() {
        let path = PathBuf::from("/tmp/herald/daemon.pid");
        let controller = DaemonController::new(path.clone());
        assert_eq!(controller.pid_manager().pid_file_path(), path);
    }

    #[test]
    fn test_daemon_controller_from_pid_manager() {
        let path = PathBuf::from("/tmp/herald/daemon.pid");
        let pid_manager = PidManager::new(path.clone());
        let controller = DaemonController::from_pid_manager(pid_manager);
        assert_eq!(controller.pid_manager().pid_file_path(), path);
    }

    #[test]
    fn test_daemon_controller_is_running_when_not_running() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let controller = DaemonController::new(pid_path);

        let result = controller.is_running().expect("Check should not error");
        assert_eq!(result, None);
    }

    #[test]
    fn test_register_daemon_success() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let mut controller = DaemonController::new(pid_path.clone());

        // Register daemon
        let result = controller
            .register_daemon()
            .expect("Registration should succeed");

        // Verify PID
        assert_eq!(result.pid, std::process::id());

        // Verify PID file was created
        assert!(pid_path.exists());

        // Verify shutdown receiver
        assert!(!*result.shutdown_rx.borrow());

        // Verify controller has shutdown sender
        assert!(controller.shutdown_receiver().is_some());
    }

    #[test]
    fn test_register_daemon_already_running() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let mut controller = DaemonController::new(pid_path);

        // Register first time
        controller
            .register_daemon()
            .expect("First registration should succeed");

        // Try to register again (same process)
        let result = controller.register_daemon();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DaemonError::AlreadyRunning(_)
        ));
    }

    #[test]
    fn test_stop_daemon_not_running() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let mut controller = DaemonController::new(pid_path);

        // Try to stop when not running
        let result = controller.stop_daemon();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DaemonError::NotRunning));
    }

    #[tokio::test]
    async fn test_stop_daemon_with_internal_shutdown() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let mut controller = DaemonController::new(pid_path);

        // Register daemon
        let start_result = controller
            .register_daemon()
            .expect("Registration should succeed");
        let mut shutdown_rx = start_result.shutdown_rx;

        // Stop daemon
        let pid = controller.stop_daemon().expect("Stop should succeed");
        assert_eq!(pid, std::process::id());

        // Verify shutdown signal was sent
        assert!(shutdown_rx.changed().await.is_ok());
        assert!(*shutdown_rx.borrow());
    }

    #[test]
    fn test_unregister_daemon() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let mut controller = DaemonController::new(pid_path.clone());

        // Register daemon
        controller
            .register_daemon()
            .expect("Registration should succeed");
        assert!(pid_path.exists());
        assert!(controller.shutdown_receiver().is_some());

        // Unregister daemon
        controller
            .unregister_daemon()
            .expect("Unregister should succeed");
        assert!(!pid_path.exists());
        assert!(controller.shutdown_receiver().is_none());
    }

    #[test]
    fn test_trigger_shutdown_when_registered() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let mut controller = DaemonController::new(pid_path);

        // Register daemon
        let start_result = controller
            .register_daemon()
            .expect("Registration should succeed");
        let shutdown_rx = start_result.shutdown_rx;

        // Initially not shutting down
        assert!(!*shutdown_rx.borrow());

        // Trigger shutdown
        let triggered = controller.trigger_shutdown();
        assert!(triggered);

        // Verify shutdown signal
        assert!(*shutdown_rx.borrow());
    }

    #[test]
    fn test_trigger_shutdown_when_not_registered() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let controller = DaemonController::new(pid_path);

        // Trigger shutdown without registering
        let triggered = controller.trigger_shutdown();
        assert!(!triggered);
    }

    #[cfg(unix)]
    #[test]
    fn test_send_signal_to_current_process() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path);

        // Send signal 0 (check if process exists) to current process
        let result = manager.send_signal(std::process::id(), 0);
        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_send_signal_to_nonexistent_process() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let manager = PidManager::new(pid_path);

        // Send signal to non-existent process
        let result = manager.send_signal(999999, 0);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DaemonError::SignalError(_)));
    }

    #[test]
    fn test_daemon_start_result_fields() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let mut controller = DaemonController::new(pid_path);

        let result = controller
            .register_daemon()
            .expect("Registration should succeed");

        // Verify all fields are accessible
        let _pid: u32 = result.pid;
        let _rx: watch::Receiver<bool> = result.shutdown_rx;
    }

    #[test]
    fn test_multiple_shutdown_receivers() {
        let temp_dir = TempDir::new().unwrap();
        let pid_path = temp_dir.path().join("daemon.pid");
        let mut controller = DaemonController::new(pid_path);

        // Register daemon
        controller
            .register_daemon()
            .expect("Registration should succeed");

        // Get multiple receivers
        let rx1 = controller.shutdown_receiver().unwrap();
        let rx2 = controller.shutdown_receiver().unwrap();

        // Both should initially be false
        assert!(!*rx1.borrow());
        assert!(!*rx2.borrow());

        // Trigger shutdown
        controller.trigger_shutdown();

        // Both should now be true
        assert!(*rx1.borrow());
        assert!(*rx2.borrow());
    }
}
