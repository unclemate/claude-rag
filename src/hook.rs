//! Hook system for session-start integration.
//!
//! This module provides functionality to install hooks that trigger
//! when Claude Code starts a new session, enabling automatic
//! notification of the daemon for tracking.

use crate::error::{RagError, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Default Claude hooks directory name.
const SESSION_START_HOOK: &str = "session-start";

/// Hook manager for installing and managing Claude Code hooks.
pub struct HookManager {
    /// Claude hooks directory.
    hooks_dir: PathBuf,
}

impl HookManager {
    /// Create a new hook manager.
    pub fn new() -> Result<Self> {
        let hooks_dir = Self::default_hooks_dir()?;

        Ok(Self { hooks_dir })
    }

    /// Create hook manager with custom hooks directory.
    pub fn with_dir(hooks_dir: PathBuf) -> Self {
        Self { hooks_dir }
    }

    /// Get default Claude hooks directory.
    fn default_hooks_dir() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| RagError::Config("Cannot determine home directory".to_string()))?;

        Ok(home.join(".claude").join("hooks"))
    }

    /// Install hooks to Claude Code hooks directory.
    ///
    /// This creates the hooks directory if it doesn't exist and
    /// writes the session-start hook script.
    pub fn install_hooks(&self) -> Result<()> {
        // Create hooks directory if it doesn't exist
        fs::create_dir_all(&self.hooks_dir)
            .map_err(|e| RagError::Io(e))?;

        // Install session-start hook
        self.install_session_start_hook()?;

        Ok(())
    }

    /// Install the session-start hook script.
    fn install_session_start_hook(&self) -> Result<()> {
        let hook_path = self.hooks_dir.join(SESSION_START_HOOK);
        let hook_content = self.generate_session_start_hook();

        fs::write(&hook_path, hook_content)
            .map_err(|e| RagError::Io(e))?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&hook_path)
                .map_err(|e| RagError::Io(e))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&hook_path, perms)
                .map_err(|e| RagError::Io(e))?;
        }

        Ok(())
    }

    /// Generate session-start hook script.
    ///
    /// The hook script extracts session information from environment
    /// variables and notifies the daemon if it's running.
    pub fn generate_session_start_hook(&self) -> String {
        r#"#!/bin/bash
# Claude RAG Session-Start Hook
#
# This hook is triggered when Claude Code starts a new session.
# It notifies the claude-rag daemon about the new session for tracking.

set -e

# Extract session information from environment variables
SESSION_ID="${CLAUDE_SESSION_ID:-unknown}"
PROJECT_PATH="${CLAUDE_PROJECT_PATH:-$(pwd)}"
SESSION_TITLE="${CLAUDE_SESSION_TITLE:-}"

# Notify daemon if it's running
DAEMON_SOCKET="${XDG_RUNTIME_DIR:-/tmp}/claude-rag.sock"

if [ -S "$DAEMON_SOCKET" ]; then
    # Send session start notification to daemon
    echo "{\"type\":\"session_start\",\"session_id\":\"$SESSION_ID\",\"project_path\":\"$PROJECT_PATH\",\"title\":\"$SESSION_TITLE\"}" | \
        socat - UNIX-CONNECT:"$DAEMON_SOCKET" 2>/dev/null || true
fi

# Optional: Also log to file for later processing
LOG_DIR="${HOME}/.claude/rag/logs"
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/sessions.log"

echo "$(date -Iseconds) [START] Session: $SESSION_ID | Project: $PROJECT_PATH | Title: $SESSION_TITLE" >> "$LOG_FILE"
"#.to_string()
    }

    /// Generate session-end hook script.
    ///
    /// This is called when a Claude session ends.
    pub fn generate_session_end_hook(&self) -> String {
        r#"#!/bin/bash
# Claude RAG Session-End Hook
#
# This hook is triggered when Claude Code ends a session.
# It notifies the claude-rag daemon to finalize tracking.

set -e

# Extract session information from environment variables
SESSION_ID="${CLAUDE_SESSION_ID:-unknown}"
PROJECT_PATH="${CLAUDE_PROJECT_PATH:-$(pwd)}"

# Notify daemon if it's running
DAEMON_SOCKET="${XDG_RUNTIME_DIR:-/tmp}/claude-rag.sock"

if [ -S "$DAEMON_SOCKET" ]; then
    echo "{\"type\":\"session_end\",\"session_id\":\"$SESSION_ID\",\"project_path\":\"$PROJECT_PATH\"}" | \
        socat - UNIX-CONNECT:"$DAEMON_SOCKET" 2>/dev/null || true
fi

# Log to file
LOG_DIR="${HOME}/.claude/rag/logs"
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/sessions.log"

echo "$(date -Iseconds) [END] Session: $SESSION_ID | Project: $PROJECT_PATH" >> "$LOG_FILE"
"#.to_string()
    }

    /// Check if hooks are installed.
    pub fn is_installed(&self) -> bool {
        self.hooks_dir.join(SESSION_START_HOOK).exists()
    }

    /// Get the hooks directory path.
    pub fn hooks_dir(&self) -> &Path {
        &self.hooks_dir
    }

    /// Uninstall hooks by removing the hooks directory.
    ///
    /// This removes all installed hooks.
    pub fn uninstall_hooks(&self) -> Result<()> {
        if self.hooks_dir.exists() {
            fs::remove_dir_all(&self.hooks_dir)
                .map_err(|e| RagError::Io(e))?;
        }
        Ok(())
    }
}

impl Default for HookManager {
    fn default() -> Self {
        Self::new().expect("Failed to create HookManager")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_hook_manager_new() {
        let manager = HookManager::new().unwrap();
        assert!(manager.hooks_dir().ends_with(".claude/hooks"));
    }

    #[test]
    fn test_hook_manager_with_dir() {
        let temp_dir = TempDir::new().unwrap();
        let custom_dir = temp_dir.path().join("custom_hooks");
        let manager = HookManager::with_dir(custom_dir.clone());

        assert_eq!(manager.hooks_dir(), custom_dir);
    }

    #[test]
    fn test_generate_session_start_hook() {
        let manager = HookManager::new().unwrap();
        let hook = manager.generate_session_start_hook();

        assert!(hook.contains("#!/bin/bash"));
        assert!(hook.contains("CLAUDE_SESSION_ID"));
        assert!(hook.contains("CLAUDE_PROJECT_PATH"));
        assert!(hook.contains("session_start"));
    }

    #[test]
    fn test_generate_session_end_hook() {
        let manager = HookManager::new().unwrap();
        let hook = manager.generate_session_end_hook();

        assert!(hook.contains("#!/bin/bash"));
        assert!(hook.contains("session_end"));
    }

    #[test]
    fn test_install_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let hooks_dir = temp_dir.path().join("hooks");
        let manager = HookManager::with_dir(hooks_dir.clone());

        manager.install_hooks().unwrap();

        // Check that session-start hook was installed
        assert!(hooks_dir.join(SESSION_START_HOOK).exists());

        // Verify content
        let content = fs::read_to_string(hooks_dir.join(SESSION_START_HOOK)).unwrap();
        assert!(content.contains("session_start"));
    }

    #[test]
    fn test_is_installed() {
        let temp_dir = TempDir::new().unwrap();
        let hooks_dir = temp_dir.path().join("hooks");
        let manager = HookManager::with_dir(hooks_dir.clone());

        // Initially not installed
        assert!(!manager.is_installed());

        // After installation
        manager.install_hooks().unwrap();
        assert!(manager.is_installed());
    }

    #[test]
    fn test_uninstall_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let hooks_dir = temp_dir.path().join("hooks");
        let manager = HookManager::with_dir(hooks_dir.clone());

        // Install hooks
        manager.install_hooks().unwrap();
        assert!(hooks_dir.exists());

        // Uninstall hooks
        manager.uninstall_hooks().unwrap();
        assert!(!hooks_dir.exists());
    }
}
