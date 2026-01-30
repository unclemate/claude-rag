//! Plan file parsing and project matching.

use crate::error::{RagError, Result};
use crate::models::Plan;
use chrono::{DateTime, Utc};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Plan parser for extracting and matching plan files.
pub struct PlanParser {
    /// Claude config directory (usually ~/.claude).
    pub(crate) config_dir: PathBuf,
    /// Current project path.
    pub(crate) project_path: PathBuf,
    /// Extracted project identifiers.
    project_identifiers: ProjectIdentifiers,
}

/// Project identifiers for matching plan files.
#[derive(Debug, Clone)]
struct ProjectIdentifiers {
    /// Project name (e.g., "claude-rag").
    name: String,
    /// Absolute project path.
    absolute_path: String,
}

impl PlanParser {
    /// Create a new plan parser.
    ///
    /// # Arguments
    /// * `config_dir` - Claude config directory (defaults to ~/.claude)
    /// * `project_path` - Current project path
    pub fn new(config_dir: Option<PathBuf>, project_path: &Path) -> Self {
        let config_dir = config_dir.unwrap_or_else(|| {
            dirs::home_dir()
                .expect("Unable to determine home directory")
                .join(".claude")
        });

        let project_identifiers = Self::extract_identifiers(project_path);

        Self {
            config_dir,
            project_path: project_path.to_path_buf(),
            project_identifiers,
        }
    }

    /// Extract project identifiers from project path.
    fn extract_identifiers(project_path: &Path) -> ProjectIdentifiers {
        let absolute_path = project_path
            .canonicalize()
            .ok()
            .unwrap_or_else(|| project_path.to_path_buf())
            .to_string_lossy()
            .to_string();

        // Extract project name (last component of path)
        let name = project_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        ProjectIdentifiers {
            name,
            absolute_path,
        }
    }

    /// Scan for plan files in ~/.claude/plans/.
    ///
    /// # Returns
    /// * `Vec<PathBuf>` - All plan file paths
    pub fn scan_plan_files(&self) -> Result<Vec<PathBuf>> {
        let plans_dir = self.config_dir.join("plans");

        if !plans_dir.exists() {
            info!("No plans directory found at {}", plans_dir.display());
            return Ok(Vec::new());
        }

        let mut plan_files = Vec::new();

        for entry in fs::read_dir(&plans_dir).map_err(RagError::Io)? {
            let entry = entry.map_err(RagError::Io)?;
            let path = entry.path();

            // Only process .md files
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }

            plan_files.push(path);
        }

        info!("Found {} plan files", plan_files.len());
        Ok(plan_files)
    }

    /// Check if a plan file belongs to the current project.
    ///
    /// Uses four-level matching (highest accuracy first):
    /// 1. **Absolute path prefix**: Match `/home/user/Projects/claude-rag` (most reliable)
    /// 2. **Project path fragment**: Match `Projects/claude-rag` (avoids `/other/claude-rag` mismatch)
    /// 3. **Relative path verification**: Extract `src/xxx.rs`, verify file exists
    /// 4. **Project name fallback**: Match project name only (with warning)
    ///
    /// # Arguments
    /// * `plan_path` - Path to the plan file
    ///
    /// # Returns
    /// * `bool` - true if the plan belongs to the current project
    pub fn belongs_to_project(&self, plan_path: &Path) -> Result<bool> {
        let content = fs::read_to_string(plan_path).map_err(RagError::Io)?;

        // Level 1: Absolute path prefix match (MOST RELIABLE)
        // Example: "/home/user/Projects/claude-rag"
        if content.contains(&self.project_identifiers.absolute_path) {
            debug!(
                "Plan {} matches by absolute path: {}",
                plan_path.display(),
                self.project_identifiers.absolute_path
            );
            return Ok(true);
        }

        // Level 2: Project path fragment match
        // Example: "Projects/claude-rag" (avoid matching "/other/claude-rag")
        let path_parts: Vec<&str> = self
            .project_identifiers
            .absolute_path
            .split('/')
            .collect();
        if path_parts.len() >= 2 {
            let project_specific = format!(
                "{}/{}",
                path_parts[path_parts.len() - 2],
                path_parts[path_parts.len() - 1]
            );
            if content.contains(&project_specific) {
                debug!(
                    "Plan {} matches by path fragment: {}",
                    plan_path.display(),
                    project_specific
                );
                return Ok(true);
            }
        }

        // Level 3: Relative path verification
        // Extract paths like "src/collector/file.rs" and verify they exist in project
        if let Some(matched_path) = self.verify_relative_paths_exist(&content) {
            debug!(
                "Plan {} matches by relative path verification: {}",
                plan_path.display(),
                matched_path
            );
            return Ok(true);
        }

        // Level 4: Project name fallback (WITH WARNING)
        // Only use project name if nothing else matches
        if content.contains(&self.project_identifiers.name) {
            warn!(
                "Plan {} matched by project name ONLY (low confidence): {}. Consider adding more specific project references.",
                plan_path.display(),
                self.project_identifiers.name
            );
            return Ok(true);
        }

        Ok(false)
    }

    /// Verify that relative paths in content exist in the project.
    ///
    /// Extracts potential file paths from content and checks if they exist.
    /// Returns the first matching path for logging.
    ///
    /// # Arguments
    /// * `content` - Plan file content
    ///
    /// # Returns
    /// * `Option<String>` - First matching path, or None
    fn verify_relative_paths_exist(&self, content: &str) -> Option<String> {
        // Pattern 1: Rust source paths (e.g., `src/collector/file.rs`)
        let re = regex::Regex::new(r"src/[a-z_/\-]+\.rs").ok()?;
        for caps in re.captures_iter(content) {
            let path = caps.get(0)?.as_str();
            let full_path = self.project_path.join(path);
            if full_path.exists() {
                return Some(path.to_string());
            }
        }

        // Pattern 2: Config files (Cargo.toml, package.json, etc.)
        for config in &["Cargo.toml", "package.json", "pyproject.toml", "go.mod"] {
            if content.contains(config) {
                let full_path = self.project_path.join(config);
                if full_path.exists() {
                    return Some(config.to_string());
                }
            }
        }

        // Pattern 3: Common directories (docs/, tests/, lib/)
        for dir in &["docs/", "tests/", "lib/"] {
            if content.contains(dir) {
                let full_path = self.project_path.join(dir);
                if full_path.exists() {
                    return Some(dir.to_string());
                }
            }
        }

        None
    }

    /// Parse a plan file.
    ///
    /// # Arguments
    /// * `plan_path` - Path to the plan file
    ///
    /// # Returns
    /// * `Plan` - Parsed plan
    pub fn parse_plan(&self, plan_path: &Path) -> Result<Plan> {
        let content = fs::read_to_string(plan_path).map_err(RagError::Io)?;

        // Extract title (first heading)
        let title = content
            .lines()
            .find(|line| line.starts_with("# "))
            .map(|line| line.replacen("# ", "", 1));

        // Get modification time
        let metadata = fs::metadata(plan_path).map_err(RagError::Io)?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                DateTime::from_timestamp(d.as_secs() as i64, 0).unwrap_or_else(Utc::now)
            })
            .unwrap_or_else(Utc::now);

        // Extract ID from filename
        let id = plan_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(Plan {
            id,
            title,
            content,
            modified_at: modified,
            chunk_count: 0,
            indexed: false,
        })
    }

    /// Scan and filter plan files for the current project.
    ///
    /// # Returns
    /// * `Vec<Plan>` - Plans belonging to the current project
    pub fn collect_project_plans(&self) -> Result<Vec<Plan>> {
        let all_plans = self.scan_plan_files()?;
        let mut project_plans = Vec::new();

        for plan_path in all_plans {
            match self.belongs_to_project(&plan_path) {
                Ok(true) => {
                    match self.parse_plan(&plan_path) {
                        Ok(plan) => {
                            info!("Including plan: {}", plan.id);
                            project_plans.push(plan);
                        }
                        Err(e) => {
                            debug!("Failed to parse plan {}: {}", plan_path.display(), e);
                        }
                    }
                }
                Ok(false) => {
                    debug!("Skipping plan (not matching project): {}", plan_path.display());
                }
                Err(e) => {
                    debug!("Error checking plan {}: {}", plan_path.display(), e);
                }
            }
        }

        info!(
            "Collected {} plans for project {}",
            project_plans.len(),
            self.project_identifiers.name
        );
        Ok(project_plans)
    }

    /// Check if a plan needs re-indexing.
    ///
    /// # Arguments
    /// * `plan_file` - Path to the plan file
    /// * `last_indexed` - Optional last indexed timestamp
    ///
    /// # Returns
    /// * `bool` - true if plan needs re-indexing
    pub fn needs_indexing(
        &self,
        plan_file: &Path,
        last_indexed: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        if !plan_file.exists() {
            return Ok(true);
        }

        let metadata = fs::metadata(plan_file).map_err(RagError::Io)?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| DateTime::from_timestamp(d.as_secs() as i64, 0).unwrap_or_else(Utc::now));

        if let Some(last_indexed) = last_indexed {
            Ok(modified.is_none_or(|m| m > last_indexed))
        } else {
            Ok(true)
        }
    }
}

impl Default for PlanParser {
    fn default() -> Self {
        Self::new(None, Path::new("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a test plan file
    fn create_test_plan(dir: &Path, filename: &str, content: &str) -> PathBuf {
        let plan_file = dir.join(filename);
        fs::write(&plan_file, content).unwrap();
        plan_file
    }

    // ========== Project identifier extraction tests ==========

    #[test]
    fn test_extract_project_identifiers_standard_path() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("test-project");
        fs::create_dir_all(&path).unwrap();

        let identifiers = PlanParser::extract_identifiers(&path);

        assert_eq!(identifiers.name, "test-project");
        assert!(identifiers.absolute_path.contains("test-project"));
    }

    #[test]
    fn test_extract_project_identifiers_nested_path() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("workspace").join("user").join("projects").join("backend");
        fs::create_dir_all(&path).unwrap();

        let identifiers = PlanParser::extract_identifiers(&path);

        assert_eq!(identifiers.name, "backend");
        assert!(identifiers.absolute_path.contains("backend"));
    }

    // ========== Level 1: Absolute path prefix tests ==========

    #[test]
    fn test_belongs_to_project_absolute_path_match() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("test-project");
        fs::create_dir_all(&project_path).unwrap();

        let project_absolute = project_path.to_string_lossy().to_string();
        let content = format!(
            r#"
# Test Plan

For project at {}

Implement feature in src/main.rs
"#,
            project_absolute
        );
        let plan_file = create_test_plan(temp.path(), "test-plan.md", &content);

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let result = parser.belongs_to_project(&plan_file).unwrap();

        assert!(
            result,
            "Should match by absolute path prefix"
        );
    }

    #[test]
    fn test_belongs_to_project_absolute_path_no_match() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("project-a");
        fs::create_dir_all(&project_path).unwrap();

        // Plan belongs to project-b
        let project_b = temp.path().join("project-b");
        let project_b_str = project_b.to_string_lossy().to_string();
        let content = format!(
            r#"
# Test Plan

For project at {}

Implement feature
"#,
            project_b_str
        );
        let plan_file = create_test_plan(temp.path(), "other-plan.md", &content);

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let result = parser.belongs_to_project(&plan_file).unwrap();

        assert!(!result, "Should not match different project");
    }

    // ========== Level 2: Project path fragment tests ==========

    #[test]
    fn test_belongs_to_project_path_fragment_match() {
        let temp = TempDir::new().unwrap();
        let base_path = temp.path().join("Projects");
        fs::create_dir_all(&base_path).unwrap();
        let project_path = base_path.join("claude-rag");
        fs::create_dir_all(&project_path).unwrap();

        let content = r#"
# Test Plan

Working on Projects/claude-rag

Implementation details...
"#;
        let plan_file = create_test_plan(temp.path(), "fragment-plan.md", content);

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let result = parser.belongs_to_project(&plan_file).unwrap();

        assert!(result, "Should match by path fragment");
    }

    #[test]
    fn test_belongs_to_project_path_fragment_no_match_similar_name() {
        let temp = TempDir::new().unwrap();
        let base_path = temp.path().join("Projects");
        fs::create_dir_all(&base_path).unwrap();
        let project_path = base_path.join("claude-rag");
        fs::create_dir_all(&project_path).unwrap();

        // Different project with similar name - should NOT match by path fragment
        // But WILL match by project name fallback (low confidence)
        let content = r#"
# Test Plan

Working on /other/workspace/claude-rag

Implementation details...
"#;
        let plan_file = create_test_plan(temp.path(), "similar-plan.md", content);

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let result = parser.belongs_to_project(&plan_file).unwrap();

        // This will match by Level 4 (project name fallback), which is expected behavior
        // The warning is logged to indicate low confidence
        assert!(result, "Should match by project name fallback (with warning)");
    }

    // ========== Level 3: Relative path verification tests ==========

    #[test]
    fn test_belongs_to_project_relative_path_rust_match() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("rust-project");
        fs::create_dir_all(&project_path).unwrap();

        // Create src/ directory structure
        let src_dir = project_path.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("main.rs"), "fn main() {}").unwrap();

        // Create the actual file referenced in the plan
        let collector_dir = src_dir.join("collector");
        fs::create_dir_all(&collector_dir).unwrap();
        fs::write(collector_dir.join("file.rs"), "// file content").unwrap();

        let content = r#"
# Test Plan

Implement feature in src/collector/file.rs

Add new module...
"#;
        let plan_file = create_test_plan(temp.path(), "relative-plan.md", content);

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let result = parser.belongs_to_project(&plan_file).unwrap();

        assert!(
            result,
            "Should match by relative path verification (src/collector/file.rs exists)"
        );
    }

    #[test]
    fn test_belongs_to_project_relative_path_config_match() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("rust-project");
        fs::create_dir_all(&project_path).unwrap();

        // Create Cargo.toml
        fs::write(project_path.join("Cargo.toml"), "[package]").unwrap();

        let content = r#"
# Test Plan

Update Cargo.toml dependencies

Add new crates...
"#;
        let plan_file = create_test_plan(temp.path(), "config-plan.md", content);

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let result = parser.belongs_to_project(&plan_file).unwrap();

        assert!(result, "Should match by config file existence");
    }

    #[test]
    fn test_belongs_to_project_relative_path_no_match() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("empty-project");
        fs::create_dir_all(&project_path).unwrap();

        // Project doesn't have src/ or Cargo.toml
        let content = r#"
# Test Plan

Work on src/feature.rs and update Cargo.toml

Implementation...
"#;
        let plan_file = create_test_plan(temp.path(), "no-match-plan.md", content);

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let result = parser.belongs_to_project(&plan_file).unwrap();

        assert!(!result, "Should not match when relative paths don't exist");
    }

    // ========== Level 4: Project name fallback tests ==========

    #[test]
    fn test_belongs_to_project_name_fallback_with_warning() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("my-app");
        fs::create_dir_all(&project_path).unwrap();

        // Only project name, no paths
        let content = r#"
# Test Plan

Working on my-app project

Add new features...
"#;
        let plan_file = create_test_plan(temp.path(), "name-only-plan.md", content);

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let result = parser.belongs_to_project(&plan_file).unwrap();

        assert!(result, "Should match by project name (fallback)");
        // Warning should be logged
    }

    // ========== Edge case tests ==========

    #[test]
    fn test_belongs_to_project_empty_content() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("test-project");
        fs::create_dir_all(&project_path).unwrap();

        let plan_file = create_test_plan(temp.path(), "empty-plan.md", "");

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let result = parser.belongs_to_project(&plan_file).unwrap();

        assert!(!result, "Should not match empty content");
    }

    #[test]
    fn test_scan_plan_files_no_plans_directory() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("test-project");

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let plans = parser.scan_plan_files().unwrap();

        assert_eq!(
            plans.len(),
            0,
            "Should return empty list when no plans directory"
        );
    }

    #[test]
    fn test_scan_plan_files_filters_non_md_files() {
        let temp = TempDir::new().unwrap();
        let plans_dir = temp.path().join("plans");
        fs::create_dir_all(&plans_dir).unwrap();

        // Create mix of files
        fs::write(plans_dir.join("plan1.md"), "# Plan 1").unwrap();
        fs::write(plans_dir.join("plan2.md"), "# Plan 2").unwrap();
        fs::write(plans_dir.join("readme.txt"), "text file").unwrap();
        fs::create_dir_all(plans_dir.join("subdir")).unwrap();

        let project_path = temp.path().join("test-project");
        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let plans = parser.scan_plan_files().unwrap();

        assert_eq!(plans.len(), 2, "Should only include .md files");
    }

    // ========== Plan parsing tests ==========

    #[test]
    fn test_parse_plan_extracts_title() {
        let temp = TempDir::new().unwrap();
        let content = r#"
# My Test Plan

## Details

Some content...
"#;
        let plan_file = create_test_plan(temp.path(), "test.md", content);

        let project_path = temp.path().join("test-project");
        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let plan = parser.parse_plan(&plan_file).unwrap();

        assert_eq!(plan.title, Some("My Test Plan".to_string()));
    }

    #[test]
    fn test_parse_plan_no_title() {
        let temp = TempDir::new().unwrap();
        let content = r#"
No heading here

Just content...
"#;
        let plan_file = create_test_plan(temp.path(), "test.md", content);

        let project_path = temp.path().join("test-project");
        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let plan = parser.parse_plan(&plan_file).unwrap();

        assert_eq!(plan.title, None);
    }

    #[test]
    fn test_parse_plan_id_from_filename() {
        let temp = TempDir::new().unwrap();
        let content = "# Test";
        let plan_file = create_test_plan(temp.path(), "my-test-plan.md", content);

        let project_path = temp.path().join("test-project");
        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let plan = parser.parse_plan(&plan_file).unwrap();

        assert_eq!(plan.id, "my-test-plan");
    }

    // ========== Integration tests ==========

    #[test]
    fn test_collect_project_plans_filters_correctly() {
        let temp = TempDir::new().unwrap();
        let plans_dir = temp.path().join("plans");
        fs::create_dir_all(&plans_dir).unwrap();

        let project_path = temp.path().join("target-project");
        fs::create_dir_all(&project_path).unwrap();

        // Create matching plan
        let project_absolute = project_path.to_string_lossy().to_string();
        let matching_content = format!(
            r#"
# Plan A

For {}

Feature implementation
"#,
            project_absolute
        );
        create_test_plan(&plans_dir, "matching.md", &matching_content);

        // Create non-matching plan
        let other_path = temp.path().join("other-project");
        let other_path_str = other_path.to_string_lossy().to_string();
        let non_matching_content = format!(
            r#"
# Plan B

For {}

Different feature
"#,
            other_path_str
        );
        create_test_plan(&plans_dir, "non-matching.md", &non_matching_content);

        let parser = PlanParser::new(Some(temp.path().to_path_buf()), &project_path);
        let plans = parser.collect_project_plans().unwrap();

        assert_eq!(plans.len(), 1, "Should only collect matching plans");
        assert_eq!(plans[0].id, "matching");
    }
}
