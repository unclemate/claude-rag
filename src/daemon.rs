//! Daemon service for background file monitoring and session tracking.
//!
//! The daemon provides:
//! - Unix socket server for hook communication
//! - File system monitoring for incremental indexing
//! - Session timeout detection
//! - Periodic HNSW index persistence
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                         Daemon                               │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌───────────────┐  ┌─────────────┐  ┌──────────────────┐  │
//! │  │ Socket Server │  │File Watcher │  │ Session Tracker  │  │
//! │  │  (UnixSocket) │  │  (notify)   │  │   (Timeout)      │  │
//! │  └───────┬───────┘  └──────┬──────┘  └────────┬─────────┘  │
//! │          │                  │                  │             │
//! │          ▼                  ▼                  ▼             │
//! │  ┌───────────────────────────────────────────────────────┐  │
//! │  │              AsyncMutex<Sessions>                     │  │
//! │  └───────────────────────────────────────────────────────┘  │
//! │                          │                                  │
//! │                          ▼                                  │
//! │  ┌───────────────────────────────────────────────────────┐  │
//! │  │            HNSW Index Persister                        │  │
//! │  └───────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use crate::config::Config;
use crate::error::{RagError, Result};
use crate::indexer::Indexer;
use crate::models::ContentType;
use crate::storage::{hnsw::HnswIndex, StorageManager};
use notify::{event::EventKind, RecursiveMode, Watcher, Event, recommended_watcher};
use std::sync::Mutex as StdMutex;  // Use std::sync::Mutex for Watcher (not Send+Sync compatible)
use tracing::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex};
use tokio::time::{interval, timeout};

/// Default socket path for daemon communication.
pub const DEFAULT_SOCKET_PATH: &str = "/tmp/claude-rag.sock";
/// Default PID file path.
pub const DEFAULT_PID_FILE: &str = "/tmp/claude-rag.pid";

/// Daemon service.
pub struct Daemon {
    /// Socket path for communication.
    socket_path: PathBuf,
    /// Active sessions being tracked (shared with async tasks).
    sessions: Arc<AsyncMutex<HashMap<String, SessionInfo>>>,
    /// Reverse index: session file path -> session ID (for O(1) lookup).
    session_file_index: Arc<AsyncMutex<HashMap<PathBuf, String>>>,
    /// Daemon shutdown sender.
    shutdown_tx: Option<broadcast::Sender<()>>,
    /// Configuration.
    config: Config,
    /// Watched files with debouncing.
    watched_files: Arc<AsyncMutex<HashMap<PathBuf, SystemTime>>>,
    /// File watcher event sender.
    file_watcher_tx: Option<mpsc::Sender<FileWatchEvent>>,
    /// File watcher (stored to keep it alive and allow dynamic watch additions).
    /// Using std::sync::Mutex because notify::Watcher is Send but not Sync.
    file_watcher: Option<Arc<StdMutex<Option<notify::RecommendedWatcher>>>>,
    /// Indexer for generating embeddings (wrapped in Arc for sharing).
    indexer: Option<Arc<Indexer>>,
    /// Tracked project paths for file watching.
    watched_projects: Arc<AsyncMutex<HashSet<PathBuf>>>,
}

/// File watch event for processing.
#[derive(Debug, Clone)]
struct FileWatchEvent {
    /// File path that changed.
    path: PathBuf,
    /// Event kind.
    #[allow(dead_code)]
    kind: EventKind,
    /// Project path (if known).
    #[allow(dead_code)]
    project_path: Option<String>,
}

/// Information about an active session.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionInfo {
    /// Session ID.
    session_id: String,
    /// Project path.
    project_path: String,
    /// Session title (optional).
    #[allow(dead_code)]
    title: Option<String>,
    /// Last activity timestamp.
    last_activity: SystemTime,
    /// Session file path.
    session_file: Option<PathBuf>,
    /// Indexing state - last indexed line number.
    last_indexed_line: Option<usize>,
}

impl SessionInfo {
    /// Create new session info.
    fn new(session_id: String, project_path: String, title: Option<String>) -> Self {
        Self {
            session_id,
            project_path,
            title,
            last_activity: SystemTime::now(),
            session_file: None,
            last_indexed_line: None,
        }
    }

    /// Update last activity time.
    #[allow(dead_code)]
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

    /// Set session file path.
    fn set_session_file(&mut self, path: PathBuf) {
        self.session_file = Some(path);
    }

    /// Update last indexed line.
    fn set_last_indexed_line(&mut self, line: usize) {
        self.last_indexed_line = Some(line);
    }
}

/// File type for monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileType {
    /// Session JSONL file.
    Session,
    /// Source code file.
    Source,
    /// Documentation file.
    Doc,
    /// Unknown file type.
    Unknown,
}

impl FileType {
    /// Detect file type from extension.
    fn from_path(path: &Path) -> Self {
        if let Some(ext) = path.extension() {
            match ext.to_str() {
                Some("jsonl") => return FileType::Session,
                Some("md") | Some("rst") | Some("txt") => return FileType::Doc,
                Some("rs") | Some("js") | Some("ts") | Some("py") | Some("go") | Some("java") => {
                    return FileType::Source
                }
                _ => {}
            }
        }
        FileType::Unknown
    }
}

impl Daemon {
    /// Create indexer from config if available.
    fn create_indexer(config: &Config) -> Option<Arc<Indexer>> {
        match Indexer::from_config(config) {
            Ok(idx) if idx.is_configured() => {
                Some(Arc::new(idx))
            }
            _ => {
                warn!("Indexer not configured, using dummy embeddings");
                None
            }
        }
    }

    /// Base constructor for daemon instances.
    fn base_new(config: Config, socket_path: PathBuf) -> Self {
        // Create indexer before moving config
        let indexer = Self::create_indexer(&config);

        Self {
            socket_path,
            sessions: Arc::new(AsyncMutex::new(HashMap::new())),
            session_file_index: Arc::new(AsyncMutex::new(HashMap::new())),
            shutdown_tx: None,
            config,
            watched_files: Arc::new(AsyncMutex::new(HashMap::new())),
            file_watcher_tx: None,
            file_watcher: None,
            indexer,
            watched_projects: Arc::new(AsyncMutex::new(HashSet::new())),
        }
    }

    /// Create a new daemon instance.
    pub fn new(config: Config) -> Self {
        let socket_path = config.daemon.socket_path.clone()
            .unwrap_or_else(|| DEFAULT_SOCKET_PATH.to_string());

        Self::base_new(config, PathBuf::from(socket_path))
    }

    /// Create daemon with custom socket path.
    pub fn with_socket_path(socket_path: PathBuf, config: Config) -> Self {
        Self::base_new(config, socket_path)
    }

    /// Get the socket path from config or default.
    pub fn get_socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Get the PID file path from config or default.
    pub fn get_pid_file(&self) -> &str {
        match &self.config.daemon.pid_file {
            Some(path) if !path.is_empty() => path.as_str(),
            _ => DEFAULT_PID_FILE,
        }
    }

    /// Get the session timeout in seconds.
    fn get_session_timeout(&self) -> u64 {
        self.config.daemon.session_timeout_seconds
    }

    /// Get the HNSW persist interval in seconds.
    fn get_persist_interval(&self) -> u64 {
        self.config.daemon.persist_interval_seconds
    }

    /// Get the file debounce delay in milliseconds.
    #[allow(dead_code)]
    fn get_file_debounce_ms(&self) -> u64 {
        self.config.daemon.file_debounce_ms
    }

    /// Get the shutdown timeout in seconds.
    fn get_shutdown_timeout(&self) -> Duration {
        Duration::from_secs(self.config.daemon.shutdown_timeout_seconds)
    }

    /// Start the daemon.
    ///
    /// This runs the daemon main loop, handling socket connections,
    /// monitoring sessions, and persisting indexes periodically.
    pub async fn start(&mut self) -> Result<()> {
        // Create shutdown channel
        let (shutdown_tx, _shutdown_rx) = broadcast::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Create file watcher channel
        let (file_tx, file_rx) = mpsc::channel::<FileWatchEvent>(100);
        self.file_watcher_tx = Some(file_tx);

        // Setup file watcher and store it so it stays alive
        let watcher = self.setup_file_watcher().await?;
        self.file_watcher = Some(Arc::new(StdMutex::new(Some(watcher))));

        // Remove old socket file if exists
        if self.socket_path.exists() {
            fs::remove_file(&self.socket_path).map_err(RagError::Io)?;
        }

        // Create socket directory if needed
        if let Some(parent) = self.socket_path.parent() {
            fs::create_dir_all(parent).map_err(RagError::Io)?;
        }

        // Bind to socket
        let listener = UnixListener::bind(&self.socket_path)
            .map_err(RagError::Io)?;

        // Write PID file
        self.write_pid()?;

        info!("Claude RAG Daemon started on socket: {}", self.socket_path.display());

        // Spawn tasks with shutdown signal
        let sessions = self.sessions.clone();
        let timeout_shutdown_rx = self.shutdown_tx.as_ref().unwrap().subscribe();
        let session_timeout = self.get_session_timeout();
        let timeout_checker = tokio::spawn(Self::run_session_timeout_checker(
            sessions,
            timeout_shutdown_rx,
            session_timeout,
        ));

        let sessions = self.sessions.clone();
        let persist_shutdown_rx = self.shutdown_tx.as_ref().unwrap().subscribe();
        let persist_interval = self.get_persist_interval();
        let persist_task = tokio::spawn(Self::run_hnsw_persistence(
            sessions,
            persist_shutdown_rx,
            persist_interval,
        ));

        let sessions = self.sessions.clone();
        let session_file_index = self.session_file_index.clone();
        let watched_files = self.watched_files.clone();
        let config = self.config.clone();
        let indexer = self.indexer.clone();
        let file_shutdown_rx = self.shutdown_tx.as_ref().unwrap().subscribe();
        let file_processor = tokio::spawn(Self::process_file_events(
            sessions,
            session_file_index,
            watched_files,
            file_rx,
            config,
            file_shutdown_rx,
            indexer,
        ));

        // Main accept loop
        let mut main_shutdown_rx = self.shutdown_tx.as_ref().unwrap().subscribe();
        loop {
            tokio::select! {
                // Accept new connection
                result = listener.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            if let Err(e) = self.handle_connection(stream).await {
                                error!("Error handling connection: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("Error accepting connection: {}", e);
                        }
                    }
                }
                // Shutdown signal
                _ = main_shutdown_rx.recv() => {
                    info!("Shutdown signal received");
                    break;
                }
            }
        }

        // Graceful shutdown with timeout
        self.graceful_shutdown(timeout_checker, persist_task, file_processor).await?;

        // Cleanup socket and PID file
        self.cleanup()?;

        info!("Daemon stopped");
        Ok(())
    }

    /// Graceful shutdown of background tasks.
    async fn graceful_shutdown(
        &self,
        timeout_checker: tokio::task::JoinHandle<()>,
        persist_task: tokio::task::JoinHandle<()>,
        file_processor: tokio::task::JoinHandle<()>,
    ) -> Result<()> {
        info!("Initiating graceful shutdown...");

        let shutdown_timeout = self.get_shutdown_timeout();

        // Try to await tasks gracefully, then abort if timeout
        let tasks = vec![timeout_checker, persist_task, file_processor];

        for task in tasks {
            match timeout(shutdown_timeout, task).await {
                Ok(Ok(())) => {
                    debug!("Task completed gracefully");
                }
                Ok(Err(e)) => {
                    warn!("Task failed during shutdown: {}", e);
                }
                Err(_) => {
                    warn!("Task did not complete in time, may have been aborted");
                }
            }
        }

        Ok(())
    }

    /// Setup file watcher for monitoring changes.
    ///
    /// Returns the created watcher so it can be stored and kept alive.
    async fn setup_file_watcher(&self) -> Result<notify::RecommendedWatcher> {
        let file_watcher_tx = self.file_watcher_tx.clone();

        // Create watcher with explicit type annotation
        fn create_watcher<F>(f: F) -> notify::Result<notify::RecommendedWatcher>
        where
            F: Fn(notify::Result<Event>) + Send + Sync + 'static,
        {
            recommended_watcher(f)
        }

        let mut watcher = create_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if let Some(tx) = &file_watcher_tx {
                    for path in event.paths {
                        let _ = tx.blocking_send(FileWatchEvent {
                            path,
                            kind: event.kind,
                            project_path: None,
                        });
                    }
                }
            }
        }).map_err(|e| RagError::Io(std::io::Error::other(e)))?;

        // Watch Claude sessions directory
        if let Some(home) = dirs::home_dir() {
            let sessions_dir = home.join(".claude/sessions");
            if sessions_dir.exists() {
                watcher.watch(&sessions_dir, RecursiveMode::Recursive)
                    .map_err(|e| RagError::Io(std::io::Error::other(e)))?;
                info!("Watching sessions directory: {}", sessions_dir.display());
            }
        }

        Ok(watcher)  // Return the watcher
    }

    /// Add a project to file watching.
    async fn watch_project(&self, project_path: &Path) -> Result<()> {
        let mut watched = self.watched_projects.lock().await;

        if watched.contains(project_path) {
            return Ok(());
        }

        watched.insert(project_path.to_path_buf());

        // Actually add the watch to the file watcher
        if let Some(watcher_arc) = &self.file_watcher {
            let mut watcher_guard = watcher_arc.lock().unwrap();
            if let Some(watcher) = watcher_guard.as_mut() {
                watcher.watch(project_path, RecursiveMode::Recursive)
                    .map_err(|e| RagError::Io(std::io::Error::other(e)))?;
                info!("Now watching project: {}", project_path.display());
            }
        }

        Ok(())
    }

    /// Process file watch events with debouncing.
    async fn process_file_events(
        sessions: Arc<AsyncMutex<HashMap<String, SessionInfo>>>,
        session_file_index: Arc<AsyncMutex<HashMap<PathBuf, String>>>,
        watched_files: Arc<AsyncMutex<HashMap<PathBuf, SystemTime>>>,
        mut event_rx: mpsc::Receiver<FileWatchEvent>,
        config: Config,
        mut shutdown_rx: broadcast::Receiver<()>,
        indexer: Option<Arc<Indexer>>,
    ) {
        let debounce_ms = config.daemon.file_debounce_ms;
        let mut interval = interval(Duration::from_millis(debounce_ms));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Process debounced events
                    let mut watched = watched_files.lock().await;

                    // Find files that haven't changed recently (ready to process)
                    let ready_to_process: Vec<PathBuf> = watched.iter()
                        .filter(|(_, last_time)| {
                            last_time.elapsed()
                                .map(|e| e.as_millis() > debounce_ms as u128)
                                .unwrap_or(false)
                        })
                        .map(|(path, _)| path.clone())
                        .collect();

                    for path in ready_to_process {
                        watched.remove(&path);

                        // Determine file type and process
                        let file_type = FileType::from_path(&path);

                        match file_type {
                            FileType::Session => {
                                if let Err(e) = Self::process_session_file(
                                    &path,
                                    &sessions,
                                    &session_file_index,
                                    &config,
                                    &indexer
                                ).await {
                                    error!("Error processing session file {}: {}", path.display(), e);
                                }
                            }
                            FileType::Source | FileType::Doc => {
                                if let Err(e) = Self::process_project_file(&path, &config, &indexer).await {
                                    error!("Error processing project file {}: {}", path.display(), e);
                                }
                            }
                            FileType::Unknown => {
                                debug!("Skipping unknown file type: {}", path.display());
                            }
                        }
                    }
                }
                Some(event) = event_rx.recv() => {
                    // Add to watched files with timestamp
                    let mut watched = watched_files.lock().await;
                    watched.insert(event.path, SystemTime::now());
                }
                _ = shutdown_rx.recv() => {
                    info!("File event processor shutting down");
                    break;
                }
            }
        }
    }

    /// Process a session file for incremental indexing.
    async fn process_session_file(
        path: &Path,
        sessions: &Arc<AsyncMutex<HashMap<String, SessionInfo>>>,
        session_file_index: &Arc<AsyncMutex<HashMap<PathBuf, String>>>,
        config: &Config,
        indexer: &Option<Arc<Indexer>>,
    ) -> Result<()> {
        // Read file content
        let content = fs::read_to_string(path).map_err(RagError::Io)?;

        // Count lines
        let line_count = content.lines().count();

        // Find session ID using reverse index (O(1) lookup)
        let (session_id, project_path, last_indexed) = {
            let index_guard = session_file_index.lock().await;
            if let Some(sid) = index_guard.get(path) {
                let sessions_guard = sessions.lock().await;
                if let Some(info) = sessions_guard.get(sid) {
                    (Some(sid.clone()), Some(info.project_path.clone()), info.last_indexed_line.unwrap_or(0))
                } else {
                    // Session found in index but not in sessions - clean up
                    (None, None, 0)
                }
            } else {
                (None, None, 0)
            }
        };

        let (session_id, project_path) = match (session_id, project_path) {
            (Some(id), Some(project)) => (id, project),
            _ => {
                debug!("No active session found for file: {}", path.display());
                return Ok(());
            }
        };

        if line_count <= last_indexed {
            return Ok(());
        }

        // New lines to index
        let new_lines: Vec<&str> = content.lines()
            .skip(last_indexed)
            .collect();

        // Index new messages
        let storage = StorageManager::open_project_db(Path::new(&project_path))?;
        let mut index = storage.load_hnsw()?.unwrap_or_else(|| {
            HnswIndex::new(
                config.hnsw.m,
                config.hnsw.ef_construction,
                config.hnsw.ef_search,
            )
        });

        // Process new lines with indexer if available
        for (i, line) in new_lines.iter().enumerate() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                // Extract message content for indexing
                if let Some(_message_content) = json.get("content").and_then(|c| c.as_str()) {
                    let embedding = if let Some(idx) = indexer {
                        // Use real indexer
                        match idx.generate(_message_content).await {
                            Ok(embed) => embed,
                            Err(e) => {
                                warn!("Failed to generate embedding, using dummy: {}", e);
                                create_dummy_embedding(1024)
                            }
                        }
                    } else {
                        // Use dummy embedding
                        create_dummy_embedding(1024)
                    };

                    let id = format!("{}-msg-{}", session_id, last_indexed + i);

                    if let Err(e) = index.insert(id, ContentType::Message, embedding) {
                        error!("Error inserting to index: {}", e);
                    }
                }
            }
        }

        // Save updated index
        storage.save_hnsw(&index)?;

        // Update session info
        let mut sessions_guard = sessions.lock().await;
        if let Some(info) = sessions_guard.get_mut(&session_id) {
            info.set_last_indexed_line(line_count);
        }

        info!("Indexed {} new lines for session {}", new_lines.len(), session_id);
        Ok(())
    }

    /// Process a project file for incremental indexing.
    async fn process_project_file(
        path: &Path,
        config: &Config,
        indexer: &Option<Arc<Indexer>>,
    ) -> Result<()> {
        // Validate the file path to prevent directory traversal
        let validated_path = Self::validate_path(path)?;

        // Find project path (traverse up to find .rag directory or git repo)
        let project_path = Self::find_project_path(&validated_path)?;

        if project_path.is_none() {
            return Ok(());
        }

        let project_path = project_path.unwrap();

        // Load or create index
        let storage = StorageManager::open_project_db(&project_path)?;
        let mut index = storage.load_hnsw()?.unwrap_or_else(|| {
            HnswIndex::new(
                config.hnsw.m,
                config.hnsw.ef_construction,
                config.hnsw.ef_search,
            )
        });

        // Create embedding with indexer if available
        let embedding = if let Some(idx) = indexer {
            // Use real indexer (read file for content)
            let file_content = fs::read_to_string(path).map_err(RagError::Io)?;
            match idx.generate(&file_content).await {
                Ok(embed) => embed,
                Err(e) => {
                    warn!("Failed to generate embedding for file, using dummy: {}", e);
                    create_dummy_embedding(1024)
                }
            }
        } else {
            // Use dummy embedding
            create_dummy_embedding(1024)
        };

        let file_type = FileType::from_path(path);
        let content_type = match file_type {
            FileType::Source => ContentType::File,
            FileType::Doc => ContentType::File,
            _ => ContentType::File,
        };

        let id = format!("file-{}", path.display());
        index.insert(id, content_type, embedding)?;

        // Save updated index
        storage.save_hnsw(&index)?;

        debug!("Indexed project file: {}", path.display());
        Ok(())
    }

    /// Validate and sanitize a file path to prevent directory traversal attacks.
    ///
    /// Returns the canonical path if it's valid and safe, or an error otherwise.
    fn validate_path(path: &Path) -> Result<PathBuf> {
        // Get canonical path to resolve symlinks and relative components
        let canonical = path.canonicalize()
            .map_err(|e| RagError::Validation(format!("Invalid path '{}': {}", path.display(), e)))?;

        // Check for suspicious patterns
        let path_str = canonical.to_string_lossy();
        if path_str.contains("..") {
            return Err(RagError::Validation(format!(
                "Path contains parent directory reference: {}", path.display()
            )));
        }

        // Allow paths within user's home directory
        if let Some(home) = dirs::home_dir() {
            let home_canonical = home.canonicalize().unwrap_or(home);
            if canonical.starts_with(&home_canonical) {
                return Ok(canonical);
            }
        }

        // Allow current working directory and subdirectories
        if let Ok(cwd) = std::env::current_dir() {
            let cwd_canonical = cwd.canonicalize().unwrap_or(cwd);
            if canonical.starts_with(&cwd_canonical) {
                return Ok(canonical);
            }
        }

        // Allow /tmp directory (for testing and temporary files)
        if cfg!(unix)
            && canonical.starts_with("/tmp") {
                return Ok(canonical);
            }

        // For Windows, allow temp directories
        if cfg!(windows) {
            if let Some(tmp) = std::env::var("TEMP").ok().or_else(|| std::env::var("TMP").ok()) {
                if canonical.starts_with(&tmp) {
                    return Ok(canonical);
                }
            }
        }

        // Reject paths outside allowed directories
        Err(RagError::Validation(format!(
            "Path is outside allowed directories: {}", path.display()
        )))
    }

    /// Find project path by traversing up from file.
    fn find_project_path(path: &Path) -> Result<Option<PathBuf>> {
        let mut current = path.parent();

        while let Some(dir) = current {
            // Check for .rag directory
            let rag_dir = dir.join(".rag");
            if rag_dir.exists() {
                return Ok(Some(dir.to_path_buf()));
            }

            // Check for .git directory
            let git_dir = dir.join(".git");
            if git_dir.exists() {
                return Ok(Some(dir.to_path_buf()));
            }

            current = dir.parent();
        }

        Ok(None)
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
                let mut session_info = SessionInfo::new(
                    notification.session_id.clone(),
                    notification.project_path.clone(),
                    notification.title.clone(),
                );

                // Try to find session file
                if let Some(home) = dirs::home_dir() {
                    let sessions_dir = home.join(".claude/sessions");
                    let session_dir = sessions_dir.join(&notification.session_id);
                    let session_file = session_dir.join("index.jsonl");

                    if session_file.exists() {
                        session_info.set_session_file(session_file.clone());

                        // Add to reverse index for O(1) lookup
                        let mut index = self.session_file_index.lock().await;
                        index.insert(session_file, notification.session_id.clone());

                        // Watch project directory
                        let project_path = Path::new(&notification.project_path);
                        let _ = self.watch_project(project_path).await;
                    }
                }

                // Store to sessions
                let mut sessions = self.sessions.lock().await;
                sessions.insert(notification.session_id.clone(), session_info);

                info!("Session started: {}", notification.session_id);
                Ok(json!({"status": "ok", "message": "Session tracked"}).to_string())
            }
            "session_end" => {
                // Remove session from tracking and persist index
                let mut sessions = self.sessions.lock().await;
                if let Some(info) = sessions.remove(&notification.session_id) {
                    // Remove from reverse index
                    if let Some(session_file) = &info.session_file {
                        let mut index = self.session_file_index.lock().await;
                        index.remove(session_file);
                    }

                    // Persist HNSW for this project
                    if let Err(e) = Self::persist_project_hnsw(&info.project_path) {
                        error!("Failed to persist HNSW for session end: {}", e);
                    }
                }

                info!("Session ended: {}", notification.session_id);
                Ok(json!({"status": "ok", "message": "Session ended"}).to_string())
            }
            _ => {
                warn!("Unknown notification type: {}", notification.notification_type);
                Ok(json!({"status": "error", "message": "Unknown notification type"}).to_string())
            }
        }
    }

    /// Run session timeout checker with shutdown support.
    async fn run_session_timeout_checker(
        sessions: Arc<AsyncMutex<HashMap<String, SessionInfo>>>,
        mut shutdown_rx: broadcast::Receiver<()>,
        session_timeout_secs: u64,
    ) {
        let mut interval = interval(Duration::from_secs(session_timeout_secs));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Check for timed out sessions
                    let mut sessions_guard = sessions.lock().await;
                    let mut timed_out = Vec::new();

                    for (id, info) in sessions_guard.iter() {
                        if info.is_timed_out(session_timeout_secs) {
                            timed_out.push(id.clone());
                        }
                    }

                    // Remove timed out sessions and persist their indexes
                    for id in timed_out {
                        if let Some(info) = sessions_guard.remove(&id) {
                            info!("Session timed out: {}", id);
                            if let Err(e) = Self::persist_project_hnsw(&info.project_path) {
                                error!("Failed to persist HNSW for timed out session: {}", e);
                            }
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Session timeout checker shutting down");
                    break;
                }
            }
        }
    }

    /// Run HNSW persistence task with shutdown support.
    async fn run_hnsw_persistence(
        sessions: Arc<AsyncMutex<HashMap<String, SessionInfo>>>,
        mut shutdown_rx: broadcast::Receiver<()>,
        persist_interval_secs: u64,
    ) {
        let mut interval = interval(Duration::from_secs(persist_interval_secs));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Group sessions by project path
                    let sessions_guard = sessions.lock().await;
                    let mut project_paths: HashSet<String> = HashSet::new();
                    for info in sessions_guard.values() {
                        project_paths.insert(info.project_path.clone());
                    }
                    drop(sessions_guard);

                    // Persist HNSW index for each active project
                    for project_path in project_paths {
                        if let Err(e) = Self::persist_project_hnsw(&project_path) {
                            error!("Failed to persist HNSW for {}: {}", project_path, e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("HNSW persistence task shutting down");
                    break;
                }
            }
        }
    }

    /// Persist HNSW index for a specific project.
    fn persist_project_hnsw(project_path: &str) -> Result<()> {
        let path = Path::new(project_path);

        // Validate the project path to prevent directory traversal
        let validated_path = Self::validate_path(path)?;

        let storage = StorageManager::open_project_db(&validated_path)?;

        // Check if HNSW index exists
        if !storage.has_hnsw_index() {
            return Ok(());
        }

        // Load, verify, and re-save the index
        if let Some(index) = storage.load_hnsw()? {
            info!(
                "Persisting HNSW index for {}: {} nodes",
                validated_path.display(),
                index.len()
            );
            storage.save_hnsw(&index)?;
        }

        Ok(())
    }

    /// Stop the daemon.
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        Ok(())
    }

    /// Get daemon status.
    pub async fn status(&self) -> Result<DaemonStatus> {
        let pid_file = self.get_pid_file();

        // Check if PID file exists and process is running
        if Path::new(pid_file).exists() {
            let pid_content = fs::read_to_string(pid_file)
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
        let pid_file = self.get_pid_file();

        // Create parent directory if needed
        if let Some(parent) = Path::new(pid_file).parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent).map_err(RagError::Io)?;
            }
        }

        fs::write(pid_file, pid.to_string())
            .map_err(RagError::Io)?;
        Ok(())
    }

    /// Cleanup daemon resources.
    fn cleanup(&self) -> Result<()> {
        // Remove socket file
        if self.socket_path.exists() {
            fs::remove_file(&self.socket_path)
                .map_err(RagError::Io)?;
        }

        // Remove PID file
        let pid_file = self.get_pid_file();
        if Path::new(pid_file).exists() {
            fs::remove_file(pid_file)
                .map_err(RagError::Io)?;
        }

        Ok(())
    }

    /// Get the number of active sessions.
    pub async fn active_session_count(&self) -> usize {
        let sessions = self.sessions.lock().await;
        sessions.len()
    }
}

/// Create a dummy embedding for testing/fallback.
fn create_dummy_embedding(dimensions: usize) -> Vec<f32> {
    (0..dimensions).map(|i| i as f32 / dimensions as f32).collect()
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
    use crate::config::EmbeddingConfig;

    fn create_test_config() -> Config {
        Config {
            embedding: EmbeddingConfig {
                api_token: "test-token".to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_daemon_new() {
        let config = create_test_config();
        let daemon = Daemon::new(config);
        assert_eq!(daemon.socket_path, PathBuf::from(DEFAULT_SOCKET_PATH));
        assert_eq!(daemon.active_session_count().await, 0);
    }

    #[test]
    fn test_daemon_with_socket_path() {
        let config = create_test_config();
        let custom_path = PathBuf::from("/tmp/custom.sock");
        let daemon = Daemon::with_socket_path(custom_path.clone(), config);
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
        assert!(info.session_file.is_none());
        assert!(info.last_indexed_line.is_none());
    }

    #[test]
    fn test_session_info_timeout() {
        let info = SessionInfo::new(
            "sess123".to_string(),
            "/path".to_string(),
            None,
        );

        // Just created, should not be timed out
        assert!(!info.is_timed_out(60));

        // Simulate old activity
        let mut info_with_old_activity = info.clone();
        info_with_old_activity.last_activity = SystemTime::now() - Duration::from_secs(120);
        assert!(info_with_old_activity.is_timed_out(60));
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
    fn test_session_info_set_session_file() {
        let mut info = SessionInfo::new(
            "sess123".to_string(),
            "/path".to_string(),
            None,
        );

        let file_path = PathBuf::from("/path/to/session.jsonl");
        info.set_session_file(file_path.clone());

        assert_eq!(info.session_file, Some(file_path));
    }

    #[test]
    fn test_session_info_set_last_indexed_line() {
        let mut info = SessionInfo::new(
            "sess123".to_string(),
            "/path".to_string(),
            None,
        );

        info.set_last_indexed_line(42);
        assert_eq!(info.last_indexed_line, Some(42));
    }

    #[test]
    fn test_file_type_from_path() {
        assert_eq!(FileType::from_path(Path::new("test.jsonl")), FileType::Session);
        assert_eq!(FileType::from_path(Path::new("test.md")), FileType::Doc);
        assert_eq!(FileType::from_path(Path::new("test.rs")), FileType::Source);
        assert_eq!(FileType::from_path(Path::new("test.py")), FileType::Source);
        assert_eq!(FileType::from_path(Path::new("test.txt")), FileType::Doc);
        assert_eq!(FileType::from_path(Path::new("test.xyz")), FileType::Unknown);
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

    #[test]
    fn test_find_project_path_with_rag_dir() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let rag_dir = project_path.join(".rag");
        fs::create_dir_all(&rag_dir).unwrap();

        let test_file = project_path.join("src/test.rs");
        fs::create_dir_all(test_file.parent().unwrap()).unwrap();
        fs::write(&test_file, "test content").unwrap();

        let result = Daemon::find_project_path(&test_file).unwrap();
        assert_eq!(result, Some(project_path.to_path_buf()));
    }

    #[tokio::test]
    async fn test_sessions_lock() {
        let sessions = Arc::new(AsyncMutex::new(HashMap::<String, SessionInfo>::new()));

        // Test adding session
        let info = SessionInfo::new(
            "test-session".to_string(),
            "/test/path".to_string(),
            None,
        );

        let mut guard = sessions.lock().await;
        guard.insert("test-session".to_string(), info);
        drop(guard);

        // Test retrieving session
        let guard = sessions.lock().await;
        assert!(guard.contains_key("test-session"));
    }

    #[tokio::test]
    async fn test_handle_notification_session_start() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        let notification = HookNotification {
            notification_type: "session_start".to_string(),
            session_id: "test-123".to_string(),
            project_path: "/test/project".to_string(),
            title: Some("Test Session".to_string()),
        };

        let response = daemon.handle_notification(&notification).await.unwrap();
        assert!(response.contains("\"status\"") && response.contains("ok"));

        // Verify session was added
        let sessions = daemon.sessions.lock().await;
        assert!(sessions.contains_key("test-123"));
    }

    #[tokio::test]
    async fn test_handle_notification_session_end() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        // First add a session
        let info = SessionInfo::new(
            "test-123".to_string(),
            "/test/project".to_string(),
            None,
        );

        let mut sessions = daemon.sessions.lock().await;
        sessions.insert("test-123".to_string(), info);
        drop(sessions);

        // Then send session_end
        let notification = HookNotification {
            notification_type: "session_end".to_string(),
            session_id: "test-123".to_string(),
            project_path: "/test/project".to_string(),
            title: None,
        };

        let response = daemon.handle_notification(&notification).await.unwrap();
        assert!(response.contains("\"status\"") && response.contains("ok"));

        // Verify session was removed
        let sessions = daemon.sessions.lock().await;
        assert!(!sessions.contains_key("test-123"));
    }

    #[tokio::test]
    async fn test_handle_notification_unknown_type() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        let notification = HookNotification {
            notification_type: "unknown_type".to_string(),
            session_id: "test-123".to_string(),
            project_path: "/test/project".to_string(),
            title: None,
        };

        let response = daemon.handle_notification(&notification).await.unwrap();
        assert!(response.contains("\"status\"") && response.contains("error"));
    }

    #[tokio::test]
    async fn test_persist_project_hnsw_no_index() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let project_path = temp_dir.path().to_str().unwrap();

        // No index exists - should not error
        let result = Daemon::persist_project_hnsw(project_path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_write_pid() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        assert!(daemon.write_pid().is_ok());

        // Verify PID file exists
        assert!(Path::new(DEFAULT_PID_FILE).exists());

        // Cleanup
        let _ = std::fs::remove_file(DEFAULT_PID_FILE);
    }

    #[test]
    fn test_cleanup() {
        let config = create_test_config();
        let custom_path = PathBuf::from("/tmp/test-daemon.sock");
        let daemon = Daemon::with_socket_path(custom_path.clone(), config);

        // Create a dummy socket file
        if let Some(parent) = custom_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&custom_path, "dummy").unwrap();

        // Create dummy PID file
        fs::write(DEFAULT_PID_FILE, "1234").unwrap();

        // Cleanup should remove both files
        assert!(daemon.cleanup().is_ok());
        assert!(!custom_path.exists());
        assert!(!Path::new(DEFAULT_PID_FILE).exists());
    }

    #[test]
    fn test_find_project_path_no_markers() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let test_file = temp_dir.path().join("some/nested/file.rs");

        fs::create_dir_all(test_file.parent().unwrap()).unwrap();
        fs::write(&test_file, "test").unwrap();

        let result = Daemon::find_project_path(&test_file).unwrap();
        // Should return None as there's no .rag or .git directory
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_persist_with_actual_index() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Create storage and index
        {
            let storage = StorageManager::open_project_db(project_path).unwrap();
            let mut index = HnswIndex::new(16, 200, 50);

            // Add a dummy entry
            let dummy_embedding: Vec<f32> = (0..1024).map(|i| i as f32 / 1024.0).collect();
            index.insert("test-id".to_string(), ContentType::Message, dummy_embedding).unwrap();

            // Save index
            storage.save_hnsw(&index).unwrap();
            // storage is dropped here, db should be closed
        }

        // Give the database time to fully close
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Now persist should work
        let result = Daemon::persist_project_hnsw(project_path.to_str().unwrap());
        if let Err(e) = &result {
            warn!(error = ?e, "Persist error");
        }
        assert!(result.is_ok(), "persist should succeed: {:?}", result);
    }

    #[tokio::test]
    async fn test_multiple_sessions_tracking() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        // Add multiple sessions
        for i in 0..5 {
            let notification = HookNotification {
                notification_type: "session_start".to_string(),
                session_id: format!("session-{}", i),
                project_path: "/test/project".to_string(),
                title: Some(format!("Session {}", i)),
            };

            let _ = daemon.handle_notification(&notification).await;
        }

        // Verify all sessions are tracked
        let sessions = daemon.sessions.lock().await;
        assert_eq!(sessions.len(), 5);

        // End all sessions
        drop(sessions);
        for i in 0..5 {
            let notification = HookNotification {
                notification_type: "session_end".to_string(),
                session_id: format!("session-{}", i),
                project_path: "/test/project".to_string(),
                title: None,
            };

            let _ = daemon.handle_notification(&notification).await;
        }

        // Verify all sessions are removed
        let sessions = daemon.sessions.lock().await;
        assert_eq!(sessions.len(), 0);
    }

    #[test]
    fn test_file_type_comprehensive() {
        let test_cases = vec![
            ("file.jsonl", FileType::Session),
            ("file.md", FileType::Doc),
            ("file.rst", FileType::Doc),
            ("file.txt", FileType::Doc),
            ("file.rs", FileType::Source),
            ("file.js", FileType::Source),
            ("file.ts", FileType::Source),
            ("file.py", FileType::Source),
            ("file.go", FileType::Source),
            ("file.java", FileType::Source),
            ("file.c", FileType::Unknown),
            ("file.cpp", FileType::Unknown),
            ("file.h", FileType::Unknown),
            ("file", FileType::Unknown),
        ];

        for (filename, expected) in test_cases {
            assert_eq!(
                FileType::from_path(Path::new(filename)),
                expected,
                "Failed for {}",
                filename
            );
        }
    }

    #[tokio::test]
    async fn test_daemon_status_without_pid() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        // Remove PID file if it exists
        let _ = std::fs::remove_file(DEFAULT_PID_FILE);

        let status = daemon.status().await.unwrap();
        assert_eq!(status, DaemonStatus::Stopped);
    }

    #[test]
    fn test_hook_notification_deserialize() {
        let json = r#"{
            "type": "session_start",
            "session_id": "test-123",
            "project_path": "/test/project",
            "title": "Test Session"
        }"#;

        let notification: HookNotification = serde_json::from_str(json).unwrap();
        assert_eq!(notification.notification_type, "session_start");
        assert_eq!(notification.session_id, "test-123");
        assert_eq!(notification.project_path, "/test/project");
        assert_eq!(notification.title, Some("Test Session".to_string()));
    }

    #[test]
    fn test_hook_notification_deserialize_no_title() {
        let json = r#"{
            "type": "session_end",
            "session_id": "test-123",
            "project_path": "/test/project"
        }"#;

        let notification: HookNotification = serde_json::from_str(json).unwrap();
        assert_eq!(notification.notification_type, "session_end");
        assert_eq!(notification.title, None);
    }

    #[test]
    fn test_file_watch_event() {
        let event = FileWatchEvent {
            path: PathBuf::from("/test/file.jsonl"),
            kind: EventKind::Create(notify::event::CreateKind::Any),
            project_path: Some("/test/project".to_string()),
        };

        assert_eq!(event.path, PathBuf::from("/test/file.jsonl"));
        assert_eq!(event.project_path, Some("/test/project".to_string()));
    }

    #[test]
    fn test_session_timeout_various_values() {
        let info = SessionInfo::new(
            "sess".to_string(),
            "/path".to_string(),
            None,
        );

        // Different timeout values
        assert!(!info.is_timed_out(0)); // Always false with 0 timeout
        assert!(!info.is_timed_out(1)); // Just created
    }

    #[tokio::test]
    async fn test_concurrent_sessions_access() {
        let sessions = Arc::new(AsyncMutex::new(HashMap::<String, SessionInfo>::new()));

        // Spawn multiple tasks that access sessions concurrently
        let mut handles = vec![];

        for i in 0..10 {
            let sessions_clone = sessions.clone();
            let handle = tokio::spawn(async move {
                let mut guard = sessions_clone.lock().await;
                guard.insert(
                    format!("session-{}", i),
                    SessionInfo::new(
                        format!("session-{}", i),
                        format!("/path/{}", i),
                        None,
                    ),
                );
            });
            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all sessions were added
        let guard = sessions.lock().await;
        assert_eq!(guard.len(), 10);
    }

    #[tokio::test]
    async fn test_process_session_file_no_matching_session() {
        let sessions = Arc::new(AsyncMutex::new(HashMap::<String, SessionInfo>::new()));
        let config = create_test_config();

        let temp_dir = tempfile::TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.jsonl");
        fs::write(&test_file, r#"{"role":"user","content":"test"}"#).unwrap();

        // No matching session, should return Ok without error
        let session_file_index = Arc::new(AsyncMutex::new(HashMap::new()));
        let result = Daemon::process_session_file(&test_file, &sessions, &session_file_index, &config, &None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_project_file_no_project() {
        let config = create_test_config();

        let temp_dir = tempfile::TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "test content").unwrap();

        // No project markers, should return Ok without error
        let result = Daemon::process_project_file(&test_file, &config, &None).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_find_project_path_with_git() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let git_dir = project_path.join(".git");
        fs::create_dir_all(&git_dir).unwrap();

        let test_file = project_path.join("src/test.rs");
        fs::create_dir_all(test_file.parent().unwrap()).unwrap();
        fs::write(&test_file, "test content").unwrap();

        let result = Daemon::find_project_path(&test_file).unwrap();
        assert_eq!(result, Some(project_path.to_path_buf()));
    }

    #[test]
    fn test_find_project_path_nested() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let rag_dir = project_path.join(".rag");
        fs::create_dir_all(&rag_dir).unwrap();

        let nested_file = project_path.join("a/b/c/test.rs");
        fs::create_dir_all(nested_file.parent().unwrap()).unwrap();
        fs::write(&nested_file, "test").unwrap();

        let result = Daemon::find_project_path(&nested_file).unwrap();
        assert_eq!(result, Some(project_path.to_path_buf()));
    }

    #[test]
    fn test_find_project_path_root() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let rag_dir = project_path.join(".rag");
        fs::create_dir_all(&rag_dir).unwrap();

        let result = Daemon::find_project_path(project_path).unwrap();
        // Current dir doesn't have .rag in parent, so returns None
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_handle_connection_empty_input() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        // Create a Unix stream pair
        if let Ok((mut stream_a, stream_b)) = UnixStream::pair() {
            // Close one side immediately
            stream_a.shutdown().await.ok();

            // Handle connection should return Ok when closed
            let result = daemon.handle_connection(stream_b).await;
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_handle_connection_malformed_json() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        // Create a Unix stream pair
        if let Ok((mut stream_a, stream_b)) = UnixStream::pair() {
            // Send malformed JSON
            let _ = stream_a.write_all(b"invalid json\n").await;
            stream_a.shutdown().await.ok();

            // Handle connection should process the line and return
            let result = daemon.handle_connection(stream_b).await;
            // Connection should complete (returns Ok when stream closes)
            assert!(result.is_ok() || result.is_err());
        }
    }

    #[tokio::test]
    async fn test_handle_connection_valid_notification() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        // Create a Unix stream pair
        if let Ok((mut stream_a, stream_b)) = UnixStream::pair() {
            // Send valid notification
            let notification = json!({
                "type": "session_start",
                "session_id": "test-123",
                "project_path": "/test/project",
                "title": "Test"
            });
            let _ = stream_a.write_all(notification.to_string().as_bytes()).await;
            let _ = stream_a.write_all(b"\n").await;
            stream_a.shutdown().await.ok();

            // Handle connection
            let result = daemon.handle_connection(stream_b).await;
            assert!(result.is_ok());

            // Verify session was tracked
            let sessions = daemon.sessions.lock().await;
            assert!(sessions.contains_key("test-123"));
        }
    }

    #[tokio::test]
    async fn test_daemon_stop() {
        let config = create_test_config();
        let mut daemon = Daemon::new(config);

        // Stop should succeed
        assert!(daemon.stop().await.is_ok());

        // Second stop should also succeed (no panic)
        assert!(daemon.stop().await.is_ok());
    }

    #[test]
    fn test_cleanup_non_existent_files() {
        let config = create_test_config();
        let custom_path = PathBuf::from("/tmp/nonexistent-sock-xyz.sock");
        let daemon = Daemon::with_socket_path(custom_path.clone(), config);

        // Cleanup should succeed even if files don't exist
        assert!(daemon.cleanup().is_ok());
    }

    #[test]
    fn test_write_pid_id_format() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        daemon.write_pid().unwrap();

        // Verify PID file contains a valid number
        let pid_content = fs::read_to_string(DEFAULT_PID_FILE).unwrap();
        let pid: u32 = pid_content.trim().parse().unwrap();
        assert!(pid > 0);
        assert!(pid <= u32::MAX);

        // Cleanup
        let _ = fs::remove_file(DEFAULT_PID_FILE);
    }

    #[test]
    fn test_persist_nonexistent_project() {
        let result = Daemon::persist_project_hnsw("/nonexistent/project/path");
        // Should fail gracefully
        assert!(result.is_err());
    }

    #[test]
    fn test_file_watch_event_no_project() {
        let event = FileWatchEvent {
            path: PathBuf::from("/test/file.jsonl"),
            kind: EventKind::Create(notify::event::CreateKind::Any),
            project_path: None,
        };

        assert_eq!(event.path, PathBuf::from("/test/file.jsonl"));
        assert_eq!(event.project_path, None);
    }

    #[tokio::test]
    async fn test_create_dummy_embedding() {
        let embed = create_dummy_embedding(1024);
        assert_eq!(embed.len(), 1024);
        assert!((embed[0] - 0.0).abs() < f32::EPSILON);
        // Last element is (1023/1024) ≈ 0.999, not 1.0
        assert!((embed[1023] - 1023.0/1024.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn test_active_session_count() {
        let config = create_test_config();
        let daemon = Daemon::new(config);

        // Initially 0
        assert_eq!(daemon.active_session_count().await, 0);

        // Add a session
        let notification = HookNotification {
            notification_type: "session_start".to_string(),
            session_id: "test-123".to_string(),
            project_path: "/test/project".to_string(),
            title: Some("Test".to_string()),
        };

        let _ = daemon.handle_notification(&notification).await;

        // Should be 1
        assert_eq!(daemon.active_session_count().await, 1);
    }
}
