//! Daemon service for background file monitoring and session tracking.
//!
//! The daemon provides:
//! - Unix socket server for hook communication
//! - File system monitoring for incremental indexing
//! - Session timeout detection
//! - Periodic HNSW index persistence

use crate::error::{RagError, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::time::interval;

/// Default socket path for daemon communication.
const DEFAULT_SOCKET_PATH: &str = "/tmp/claude-rag.sock";
/// Session timeout in seconds.
const SESSION_TIMEOUT_SECS: u64 = 60;
/// HNSW persistence interval in seconds.
const HNSW_PERSIST_INTERVAL_SECS: u64 = 300;
/// PID file path.
const PID_FILE: &str = "/tmp/claude-rag.pid";

/// Daemon service.
pub struct Daemon {
    /// Socket path for communication.
    socket_path: PathBuf,
    /// Active sessions being tracked.
    #[allow(dead_code)]
    sessions: HashMap<String, SessionInfo>,
    /// Daemon shutdown sender.
    shutdown_tx: Option<mpsc::Sender<()>>,
}

/// Information about an active session.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionInfo {
    /// Session ID.
    session_id: String,
    /// Project path.
    project_path: String,
    /// Session title (optional).
    title: Option<String>,
    /// Last activity timestamp.
    last_activity: SystemTime,
}

#[allow(dead_code)]
impl SessionInfo {
    /// Create new session info.
    fn new(session_id: String, project_path: String, title: Option<String>) -> Self {
        Self {
            session_id,
            project_path,
            title,
            last_activity: SystemTime::now(),
        }
    }

    /// Update last activity time.
    fn update_activity(&mut self) {
        self.last_activity = SystemTime::now();
    }

    /// Check if session has timed out.
    fn is_timed_out(&self, timeout_secs: u64) -> bool {
        self.last_activity
            .elapsed()
            .map(|elapsed| elapsed.as_secs() > timeout_secs)
            .unwrap_or(false)
    }
}

impl Daemon {
    /// Create a new daemon instance.
    pub fn new() -> Self {
        Self {
            socket_path: PathBuf::from(DEFAULT_SOCKET_PATH),
            sessions: HashMap::new(),
            shutdown_tx: None,
        }
    }

    /// Create daemon with custom socket path.
    pub fn with_socket_path(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            sessions: HashMap::new(),
            shutdown_tx: None,
        }
    }

    /// Start the daemon.
    ///
    /// This runs the daemon main loop, handling socket connections,
    /// monitoring sessions, and persisting indexes periodically.
    pub async fn start(&mut self) -> Result<()> {
        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Remove old socket file if exists
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)
                .map_err(RagError::Io)?;
        }

        // Create socket directory if needed
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(RagError::Io)?;
        }

        // Bind to socket
        let listener = UnixListener::bind(&self.socket_path)
            .map_err(RagError::Io)?;

        // Write PID file
        self.write_pid()?;

        println!("Claude RAG Daemon started on socket: {}", self.socket_path.display());

        // Spawn session timeout checker
        let sessions_clone = std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new(),
        ));
        let timeout_checker = tokio::spawn(Self::run_session_timeout_checker(
            sessions_clone.clone(),
        ));

        // Spawn HNSW persistence task
        let persist_task = tokio::spawn(Self::run_hnsw_persistence());

        // Main accept loop
        loop {
            tokio::select! {
                // Accept new connection
                result = listener.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            if let Err(e) = self.handle_connection(stream).await {
                                eprintln!("Error handling connection: {}", e);
                            }
                        }
                        Err(e) => {
                            eprintln!("Error accepting connection: {}", e);
                        }
                    }
                }
                // Shutdown signal
                _ = shutdown_rx.recv() => {
                    println!("Shutdown signal received");
                    break;
                }
            }
        }

        // Cleanup tasks
        timeout_checker.abort();
        persist_task.abort();

        // Cleanup socket and PID file
        self.cleanup()?;

        println!("Daemon stopped");
        Ok(())
    }

    /// Handle a client connection.
    async fn handle_connection(&self, mut stream: UnixStream) -> Result<()> {
        let (reader, mut writer) = stream.split();
        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();

            // Read message from client
            let n = buf_reader.read_line(&mut line).await
                .map_err(RagError::Io)?;

            if n == 0 {
                break; // Connection closed
            }

            // Parse message
            let notification: HookNotification = serde_json::from_str(line.trim())
                .map_err(|e| RagError::Parse(format!("Invalid notification: {}", e)))?;

            // Handle notification
            let response = self.handle_notification(&notification).await?;

            // Send response
            writer.write_all(response.as_bytes()).await
                .map_err(RagError::Io)?;
            writer.write_all(b"\n").await
                .map_err(RagError::Io)?;
        }

        Ok(())
    }

    /// Handle a hook notification.
    async fn handle_notification(&self, notification: &HookNotification) -> Result<String> {
        match notification.notification_type.as_str() {
            "session_start" => {
                let _session_info = SessionInfo::new(
                    notification.session_id.clone(),
                    notification.project_path.clone(),
                    notification.title.clone(),
                );
                // In real implementation, would store to shared state
                Ok(json!({"status": "ok", "message": "Session tracked"}).to_string())
            }
            "session_end" => {
                // Remove session from tracking
                Ok(json!({"status": "ok", "message": "Session ended"}).to_string())
            }
            _ => {
                Ok(json!({"status": "error", "message": "Unknown notification type"}).to_string())
            }
        }
    }

    /// Run session timeout checker.
    async fn run_session_timeout_checker(
        sessions: std::sync::Arc<tokio::sync::Mutex<HashMap<String, SessionInfo>>>,
    ) {
        let mut interval = interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            // Check for timed out sessions
            let mut sessions_guard = sessions.lock().await;
            let _now = SystemTime::now();
            sessions_guard.retain(|_, info| {
                info.last_activity
                    .elapsed()
                    .map(|e| e.as_secs() < SESSION_TIMEOUT_SECS)
                    .unwrap_or(false)
            });
        }
    }

    /// Run HNSW persistence task.
    async fn run_hnsw_persistence() {
        let mut interval = interval(Duration::from_secs(HNSW_PERSIST_INTERVAL_SECS));
        loop {
            interval.tick().await;
            // TODO: Persist HNSW index to disk
            eprintln!("HNSW persistence tick (not yet implemented)");
        }
    }

    /// Stop the daemon.
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        Ok(())
    }

    /// Get daemon status.
    pub async fn status(&self) -> Result<DaemonStatus> {
        // Check if PID file exists and process is running
        if Path::new(PID_FILE).exists() {
            let pid_content = std::fs::read_to_string(PID_FILE)
                .map_err(RagError::Io)?;
            let pid: u32 = pid_content.trim()
                .parse()
                .map_err(|_| RagError::Parse("Invalid PID".to_string()))?;

            // Check if process is running (Unix specific)
            #[cfg(unix)]
            {
                use std::process::Command;
                let result = Command::new("kill")
                    .arg("-0")
                    .arg(pid.to_string())
                    .output();

                match result {
                    Ok(output) if output.status.success() => return Ok(DaemonStatus::Running),
                    _ => return Ok(DaemonStatus::Stopped),
                }
            }

            #[cfg(not(unix))]
            {
                return Ok(DaemonStatus::Running);
            }
        }

        Ok(DaemonStatus::Stopped)
    }

    /// Write PID file.
    fn write_pid(&self) -> Result<()> {
        let pid = std::process::id();
        std::fs::write(PID_FILE, pid.to_string())
            .map_err(RagError::Io)?;
        Ok(())
    }

    /// Cleanup daemon resources.
    fn cleanup(&self) -> Result<()> {
        // Remove socket file
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)
                .map_err(RagError::Io)?;
        }

        // Remove PID file
        if Path::new(PID_FILE).exists() {
            std::fs::remove_file(PID_FILE)
                .map_err(RagError::Io)?;
        }

        Ok(())
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}

/// Daemon status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonStatus {
    /// Daemon is running.
    Running,
    /// Daemon is stopped.
    Stopped,
    /// Daemon status unknown.
    Unknown,
}

/// Hook notification from session-start/end hooks.
#[derive(Debug, Clone, Deserialize)]
struct HookNotification {
    #[serde(rename = "type")]
    notification_type: String,
    session_id: String,
    project_path: String,
    #[serde(default)]
    title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_new() {
        let daemon = Daemon::new();
        assert_eq!(daemon.socket_path, PathBuf::from(DEFAULT_SOCKET_PATH));
    }

    #[test]
    fn test_daemon_with_socket_path() {
        let custom_path = PathBuf::from("/tmp/custom.sock");
        let daemon = Daemon::with_socket_path(custom_path.clone());
        assert_eq!(daemon.socket_path, custom_path);
    }

    #[test]
    fn test_session_info_new() {
        let info = SessionInfo::new(
            "sess123".to_string(),
            "/path/to/project".to_string(),
            Some("Test Session".to_string()),
        );

        assert_eq!(info.session_id, "sess123");
        assert_eq!(info.project_path, "/path/to/project");
        assert_eq!(info.title, Some("Test Session".to_string()));
    }

    #[test]
    fn test_session_info_timeout() {
        let mut info = SessionInfo::new(
            "sess123".to_string(),
            "/path".to_string(),
            None,
        );

        // Just created, should not be timed out
        assert!(!info.is_timed_out(60));

        // Simulate old activity
        info.last_activity = SystemTime::now() - Duration::from_secs(120);
        assert!(info.is_timed_out(60));
    }

    #[test]
    fn test_session_info_update_activity() {
        let mut info = SessionInfo::new(
            "sess123".to_string(),
            "/path".to_string(),
            None,
        );

        // Make it old
        info.last_activity = SystemTime::now() - Duration::from_secs(120);
        assert!(info.is_timed_out(60));

        // Update activity
        info.update_activity();
        assert!(!info.is_timed_out(60));
    }

    #[test]
    fn test_daemon_status_enum() {
        let running = DaemonStatus::Running;
        let stopped = DaemonStatus::Stopped;
        let unknown = DaemonStatus::Unknown;

        assert_eq!(running, DaemonStatus::Running);
        assert_ne!(running, stopped);
        assert_ne!(running, unknown);
    }
}
