//! Claude Code Skills generator.
//!
//! This module generates and installs Claude Code skills for RAG queries.

use crate::error::{RagError, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Skills installer.
pub struct SkillsInstaller {
    /// Claude skills directory.
    skills_dir: PathBuf,
}

impl SkillsInstaller {
    /// Create a new skills installer.
    pub fn new() -> Result<Self> {
        let skills_dir = Self::default_skills_dir()?;

        Ok(Self { skills_dir })
    }

    /// Create installer with custom skills directory.
    pub fn with_dir(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }

    /// Get default Claude skills directory.
    fn default_skills_dir() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| RagError::Config("Cannot determine home directory".to_string()))?;

        Ok(home.join(".claude").join("skills"))
    }

    /// Install skills to Claude Code skills directory.
    pub fn install_skills(&self) -> Result<()> {
        // Create skills directory if it doesn't exist
        fs::create_dir_all(&self.skills_dir)
            .map_err(RagError::Io)?;

        // Install each skill
        self.install_skill("rag-query", &self.generate_rag_query_skill())?;
        self.install_skill("rag-code", &self.generate_rag_code_skill())?;
        self.install_skill("rag-docs", &self.generate_rag_docs_skill())?;
        self.install_skill("rag-session", &self.generate_rag_session_skill())?;
        self.install_skill("rag-timeline", &self.generate_rag_timeline_skill())?;

        Ok(())
    }

    /// Install a single skill script.
    fn install_skill(&self, name: &str, content: &str) -> Result<()> {
        let skill_path = self.skills_dir.join(name);

        fs::write(&skill_path, content)
            .map_err(RagError::Io)?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&skill_path)
                .map_err(RagError::Io)?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&skill_path, perms)
                .map_err(RagError::Io)?;
        }

        Ok(())
    }

    /// Generate rag-query skill script.
    ///
    /// This skill queries all indexed content (sessions, files, git history).
    pub fn generate_rag_query_skill(&self) -> String {
        r#"#!/bin/bash
# Claude Code RAG Query Skill
# Usage: /rag-query "your query here"

set -e

query="$1"

if [ -z "$query" ]; then
    echo "Usage: /rag-query <query>"
    echo "Query the RAG knowledge base for relevant context."
    exit 1
fi

# Get current project path
PROJECT_PATH="${CLAUDE_PROJECT_PATH:-$(pwd)}"

# Execute RAG query
claude-rag query "$query" \
    --top-k 5 \
    --format markdown
"#.to_string()
    }

    /// Generate rag-code skill script.
    ///
    /// This skill queries only source code content.
    pub fn generate_rag_code_skill(&self) -> String {
        r#"#!/bin/bash
# Claude Code RAG Code Search Skill
# Usage: /rag-code "functionality you're looking for"

set -e

query="$1"

if [ -z "$query" ]; then
    echo "Usage: /rag-code <query>"
    echo "Search source code for relevant implementations."
    exit 1
fi

# Get current project path
PROJECT_PATH="${CLAUDE_PROJECT_PATH:-$(pwd)}"

# Execute RAG query filtered to source code
claude-rag query "$query" \
    --type source \
    --top-k 10 \
    --format markdown
"#.to_string()
    }

    /// Generate rag-docs skill script.
    ///
    /// This skill queries only documentation content.
    pub fn generate_rag_docs_skill(&self) -> String {
        r#"#!/bin/bash
# Claude Code RAG Documentation Search Skill
# Usage: /rag-docs "topic or question"

set -e

query="$1"

if [ -z "$query" ]; then
    echo "Usage: /rag-docs <query>"
    echo "Search documentation for relevant information."
    exit 1
fi

# Get current project path
PROJECT_PATH="${CLAUDE_PROJECT_PATH:-$(pwd)}"

# Execute RAG query filtered to documentation
claude-rag query "$query" \
    --type docs \
    --top-k 5 \
    --format markdown
"#.to_string()
    }

    /// Generate rag-session skill script.
    ///
    /// This skill queries previous Claude sessions.
    pub fn generate_rag_session_skill(&self) -> String {
        r#"#!/bin/bash
# Claude Code RAG Session Search Skill
# Usage: /rag-session "topic discussed previously"

set -e

query="$1"

if [ -z "$query" ]; then
    echo "Usage: /rag-session <query>"
    echo "Search previous Claude sessions for relevant discussions."
    exit 1
fi

# Get current project path
PROJECT_PATH="${CLAUDE_PROJECT_PATH:-$(pwd)}"

# Execute RAG query filtered to sessions
claude-rag query "$query" \
    --type session \
    --top-k 5 \
    --format markdown
"#.to_string()
    }

    /// Generate rag-timeline skill script.
    ///
    /// This skill builds a feature timeline from git history and sessions.
    pub fn generate_rag_timeline_skill(&self) -> String {
        r#"#!/bin/bash
# Claude Code RAG Timeline Skill
# Usage: /rag-timeline [feature-name]

set -e

feature="$1"

# Get current project path
PROJECT_PATH="${CLAUDE_PROJECT_PATH:-$(pwd)}"

# Execute timeline query
if [ -n "$feature" ]; then
    echo "Building timeline for feature: $feature"
    claude-rag query "$feature" \
        --timeline \
        --top-k 20 \
        --format markdown
else
    echo "Building general project timeline..."
    claude-rag query "project history and changes" \
        --timeline \
        --top-k 50 \
        --format markdown
fi
"#.to_string()
    }

    /// Get the skills directory path.
    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }

    /// Check if skills are already installed.
    pub fn is_installed(&self) -> bool {
        self.skills_dir.join("rag-query").exists()
    }
}

impl Default for SkillsInstaller {
    fn default() -> Self {
        Self::new().expect("Failed to create SkillsInstaller")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_skills_installer_new() {
        let installer = SkillsInstaller::new().unwrap();
        assert!(installer.skills_dir().ends_with(".claude/skills"));
    }

    #[test]
    fn test_skills_installer_with_dir() {
        let temp_dir = TempDir::new().unwrap();
        let custom_dir = temp_dir.path().join("custom_skills");
        let installer = SkillsInstaller::with_dir(custom_dir.clone());

        assert_eq!(installer.skills_dir(), custom_dir);
    }

    #[test]
    fn test_generate_rag_query_skill() {
        let installer = SkillsInstaller::new().unwrap();
        let skill = installer.generate_rag_query_skill();

        assert!(skill.contains("#!/bin/bash"));
        assert!(skill.contains("claude-rag query"));
        assert!(skill.contains("--top-k 5"));
    }

    #[test]
    fn test_generate_rag_code_skill() {
        let installer = SkillsInstaller::new().unwrap();
        let skill = installer.generate_rag_code_skill();

        assert!(skill.contains("--type source"));
        assert!(skill.contains("--top-k 10"));
    }

    #[test]
    fn test_generate_rag_docs_skill() {
        let installer = SkillsInstaller::new().unwrap();
        let skill = installer.generate_rag_docs_skill();

        assert!(skill.contains("--type docs"));
    }

    #[test]
    fn test_generate_rag_session_skill() {
        let installer = SkillsInstaller::new().unwrap();
        let skill = installer.generate_rag_session_skill();

        assert!(skill.contains("--type session"));
    }

    #[test]
    fn test_generate_rag_timeline_skill() {
        let installer = SkillsInstaller::new().unwrap();
        let skill = installer.generate_rag_timeline_skill();

        assert!(skill.contains("--timeline"));
        assert!(skill.contains("project history"));
    }

    #[test]
    fn test_install_skills() {
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        let installer = SkillsInstaller::with_dir(skills_dir.clone());

        installer.install_skills().unwrap();

        // Check that all skills were installed
        assert!(skills_dir.join("rag-query").exists());
        assert!(skills_dir.join("rag-code").exists());
        assert!(skills_dir.join("rag-docs").exists());
        assert!(skills_dir.join("rag-session").exists());
        assert!(skills_dir.join("rag-timeline").exists());

        // Verify content of one skill
        let content = fs::read_to_string(skills_dir.join("rag-query")).unwrap();
        assert!(content.contains("claude-rag query"));
    }

    #[test]
    fn test_is_installed() {
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        let installer = SkillsInstaller::with_dir(skills_dir.clone());

        // Initially not installed
        assert!(!installer.is_installed());

        // After installation
        installer.install_skills().unwrap();
        assert!(installer.is_installed());
    }
}
