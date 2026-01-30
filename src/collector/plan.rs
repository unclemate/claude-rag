//! Plan collection from Claude Code design documents.

use crate::error::{RagError, Result};
use crate::models::Plan;
use crate::plan_parser::PlanParser;
use crate::progress::ProgressReporter;
use crate::progress::ProgressEvent;
use crate::storage::sled::StorageManager;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Collection statistics for plans.
#[derive(Debug, Clone, Default)]
pub struct PlanCollectionStats {
    /// Number of plans scanned
    pub plans_scanned: usize,
    /// Number of plans collected
    pub plans_collected: usize,
    /// Number of plan chunks indexed
    pub plan_chunks_indexed: usize,
    /// Number of errors
    pub errors: usize,
}

/// Plan collector for gathering Claude Code design documents.
pub struct PlanCollector {
    /// Plan parser
    parser: PlanParser,
}

impl PlanCollector {
    /// Create a new plan collector.
    ///
    /// # Arguments
    /// * `config_dir` - Optional custom Claude config directory
    /// * `project_path` - Current project path
    pub fn new(config_dir: Option<PathBuf>, project_path: &Path) -> Self {
        debug!(
            "Creating PlanCollector with config_dir: {:?}, project_path: {}",
            config_dir,
            project_path.display()
        );
        let parser = PlanParser::new(config_dir, project_path);
        Self { parser }
    }

    /// Set custom config directory.
    pub fn with_config_dir(mut self, config_dir: PathBuf) -> Self {
        self.parser.config_dir = config_dir;
        self
    }

    /// Collect plans for the current project.
    ///
    /// # Returns
    /// * `Vec<Plan>` - List of parsed plans
    pub fn collect_plans(&self) -> Result<Vec<Plan>> {
        debug!("Collecting plans for current project");
        self.parser.collect_project_plans()
    }

    /// Collect plans incrementally based on storage state.
    ///
    /// # Arguments
    /// * `storage` - Storage manager to check index state
    ///
    /// # Returns
    /// * `Vec<Plan>` - List of plans that need indexing
    pub fn collect_incremental(&self, storage: &StorageManager) -> Result<Vec<Plan>> {
        debug!("Collecting plans incrementally");
        let all_plans = self.collect_plans()?;
        let mut needs_indexing = Vec::new();

        for plan in all_plans {
            // Check if plan exists in storage
            if let Some(_existing) = storage.get_plan(&plan.id)? {
                // Check modification time
                let last_indexed = storage.get_index_state(&plan.id)?;
                let indexed_time = last_indexed
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                // Get plan file path
                let plan_file = self.get_plan_file(&plan.id)?;
                if self.parser.needs_indexing(&plan_file, indexed_time)? {
                    debug!("Plan {} needs indexing (modified)", plan.id);
                    needs_indexing.push(plan);
                }
            } else {
                // New plan
                debug!("Plan {} is new, needs indexing", plan.id);
                needs_indexing.push(plan);
            }
        }

        info!(
            "Incremental collection: {} plans need indexing",
            needs_indexing.len()
        );
        Ok(needs_indexing)
    }

    /// Get the plan file path for a plan ID.
    fn get_plan_file(&self, plan_id: &str) -> Result<PathBuf> {
        let plans_dir = self.parser.config_dir.join("plans");
        let plan_file = plans_dir.join(format!("{}.md", plan_id));

        if plan_file.exists() {
            Ok(plan_file)
        } else {
            Err(RagError::NotFound(format!("Plan file for {}", plan_id)))
        }
    }

    /// Store collected plans to storage.
    ///
    /// # Arguments
    /// * `plans` - Plans to store
    /// * `storage` - Storage manager
    ///
    /// # Returns
    /// * `PlanCollectionStats` - Collection statistics
    pub fn store_plans(
        &self,
        plans: &[Plan],
        storage: &StorageManager,
    ) -> Result<PlanCollectionStats> {
        let mut stats = PlanCollectionStats {
            plans_scanned: plans.len(),
            ..Default::default()
        };

        for plan in plans {
            // Store plan
            match storage.store_plan(plan) {
                Ok(_) => {
                    stats.plans_collected += 1;
                }
                Err(e) => {
                    warn!(
                        plan_id = %plan.id,
                        error = %e,
                        "Error storing plan"
                    );
                    stats.errors += 1;
                    continue;
                }
            }

            // Mark as indexed
            let _ = storage.set_index_state(&plan.id, &Utc::now().to_rfc3339());
        }

        Ok(stats)
    }

    /// Store collected plans to storage with progress reporting.
    ///
    /// # Arguments
    /// * `plans` - Plans to store
    /// * `storage` - Storage manager
    /// * `reporter` - Progress reporter
    ///
    /// # Returns
    /// * `PlanCollectionStats` - Collection statistics
    pub fn store_plans_with_progress(
        &self,
        plans: &[Plan],
        storage: &StorageManager,
        reporter: &dyn ProgressReporter,
    ) -> Result<PlanCollectionStats> {
        let mut stats = PlanCollectionStats {
            plans_scanned: plans.len(),
            ..Default::default()
        };

        reporter.report(ProgressEvent::PhaseStarted {
            name: "plans".to_string(),
            total: plans.len(),
        });

        for (i, plan) in plans.iter().enumerate() {
            reporter.report(ProgressEvent::ItemProgress {
                current: i + 1,
                total: plans.len(),
                name: plan.title.clone().unwrap_or_else(|| plan.id.clone()),
            });

            match storage.store_plan(plan) {
                Ok(_) => {
                    stats.plans_collected += 1;
                    reporter.report(ProgressEvent::ItemCompleted {
                        name: plan.id.clone(),
                        success: true,
                    });
                }
                Err(e) => {
                    stats.errors += 1;
                    warn!(
                        plan_id = %plan.id,
                        error = %e,
                        "Error storing plan"
                    );
                    reporter.report(ProgressEvent::ItemCompleted {
                        name: plan.id.clone(),
                        success: false,
                    });
                    reporter.report(ProgressEvent::Error {
                        message: format!("Failed to store plan {}: {}", plan.id, e),
                    });
                    continue;
                }
            }

            let _ = storage.set_index_state(&plan.id, &Utc::now().to_rfc3339());
        }

        reporter.report(ProgressEvent::PhaseCompleted {
            name: "plans".to_string(),
            duration_secs: 0.0,
        });

        Ok(stats)
    }
}

impl Default for PlanCollector {
    fn default() -> Self {
        Self::new(None, Path::new("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create mock plans directory structure
    fn create_mock_plans(temp: &Path, project_path: &Path) {
        let plans_dir = temp.join("plans");
        fs::create_dir_all(&plans_dir).unwrap();

        let project_absolute = project_path.to_string_lossy();
        let content = format!(
            r##"
# Test Plan

Working on {}

Implementation...
"##,
            project_absolute
        );

        fs::write(plans_dir.join("test-plan.md"), content).unwrap();
    }

    #[test]
    fn test_collector_new() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("test-project");

        let collector = PlanCollector::new(Some(temp.path().to_path_buf()), &project_path);

        assert_eq!(collector.parser.project_path, project_path);
    }

    #[test]
    fn test_collector_default() {
        let collector = PlanCollector::default();
        assert_eq!(collector.parser.project_path, Path::new("."));
    }

    #[test]
    fn test_collect_plans_returns_plans() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("test-project");
        create_mock_plans(temp.path(), &project_path);

        let collector = PlanCollector::new(Some(temp.path().to_path_buf()), &project_path);
        let plans = collector.collect_plans().unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].id, "test-plan");
    }

    #[test]
    fn test_collect_plans_empty_directory() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path().join("test-project");

        let collector = PlanCollector::new(Some(temp.path().to_path_buf()), &project_path);
        let plans = collector.collect_plans().unwrap();

        assert_eq!(plans.len(), 0);
    }

    #[test]
    fn test_collection_stats_default() {
        let stats = PlanCollectionStats::default();
        assert_eq!(stats.plans_scanned, 0);
        assert_eq!(stats.plans_collected, 0);
        assert_eq!(stats.plan_chunks_indexed, 0);
        assert_eq!(stats.errors, 0);
    }
}
