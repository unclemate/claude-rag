//! Statistics collection for progress reporting.

use std::collections::HashMap;
use std::time::Instant;

/// Statistics collected during indexing operations.
#[derive(Debug, Clone, Default)]
pub struct ProgressStats {
    /// When indexing started.
    pub started_at: Option<Instant>,
    /// When indexing finished.
    pub finished_at: Option<Instant>,
    /// Total items to process.
    pub total_items: usize,
    /// Successfully processed items.
    pub succeeded_items: usize,
    /// Failed items.
    pub failed_items: usize,
    /// Cache hits count.
    pub cache_hits: usize,
    /// Embeddings generated count.
    pub embeddings_generated: usize,
    /// Per-phase statistics.
    pub phases: HashMap<String, PhaseStats>,
}

impl ProgressStats {
    /// Calculate processing rate (items per second).
    pub fn processing_rate(&self) -> Option<f64> {
        let duration = self.total_duration_secs()?;
        if duration > 0.0 {
            Some(self.succeeded_items as f64 / duration)
        } else {
            None
        }
    }

    /// Calculate cache hit rate (0.0 to 1.0).
    pub fn cache_hit_rate(&self) -> Option<f64> {
        let total = self.cache_hits + self.embeddings_generated;
        if total > 0 {
            Some(self.cache_hits as f64 / total as f64)
        } else {
            None
        }
    }

    /// Total duration in seconds.
    pub fn total_duration_secs(&self) -> Option<f64> {
        match (self.started_at, self.finished_at) {
            (Some(start), Some(end)) => Some(end.duration_since(start).as_secs_f64()),
            (Some(start), None) => Some(start.elapsed().as_secs_f64()),
            _ => None,
        }
    }

    /// Record a cache hit.
    pub fn record_cache_hit(&mut self) {
        self.cache_hits += 1;
    }

    /// Record an embedding generation.
    pub fn record_embedding(&mut self) {
        self.embeddings_generated += 1;
    }

    /// Record a successful item.
    pub fn record_success(&mut self) {
        self.succeeded_items += 1;
    }

    /// Record a failed item.
    pub fn record_failure(&mut self) {
        self.failed_items += 1;
    }

    /// Set total items.
    pub fn set_total(&mut self, total: usize) {
        self.total_items = total;
    }

    /// Mark indexing as started.
    pub fn mark_started(&mut self) {
        self.started_at = Some(Instant::now());
    }

    /// Mark indexing as finished.
    pub fn mark_finished(&mut self) {
        self.finished_at = Some(Instant::now());
    }

    /// Get or create phase stats.
    pub fn phase(&mut self, name: &str) -> &mut PhaseStats {
        self.phases
            .entry(name.to_string())
            .or_insert_with(PhaseStats::default)
    }

    /// Complete a phase.
    pub fn complete_phase(&mut self, name: String, duration_secs: f64) {
        let phase = self.phase(&name);
        phase.completed = true;
        phase.duration_secs = Some(duration_secs);
    }
}

/// Statistics for a single phase.
#[derive(Debug, Clone, Default)]
pub struct PhaseStats {
    /// Whether this phase is completed.
    pub completed: bool,
    /// Phase duration in seconds.
    pub duration_secs: Option<f64>,
    /// Items processed in this phase.
    pub items_processed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_hit_rate() {
        let mut stats = ProgressStats::default();
        stats.embeddings_generated = 100;
        stats.cache_hits = 50;

        // Cache hit rate = cache_hits / (cache_hits + embeddings_generated)
        // = 50 / (50 + 100) = 50 / 150 = 0.333...
        assert_eq!(stats.cache_hit_rate(), Some(50.0 / 150.0));
    }

    #[test]
    fn test_cache_hit_rate_no_data() {
        let stats = ProgressStats::default();
        assert_eq!(stats.cache_hit_rate(), None);
    }

    #[test]
    fn test_cache_hit_rate_all_hits() {
        let mut stats = ProgressStats::default();
        stats.cache_hits = 100;
        stats.embeddings_generated = 0;

        assert_eq!(stats.cache_hit_rate(), Some(1.0));
    }

    #[test]
    fn test_processing_rate() {
        let mut stats = ProgressStats::default();
        stats.started_at = Some(Instant::now());
        stats.succeeded_items = 100;
        // Simulate 10 seconds elapsed
        stats.finished_at = Some(stats.started_at.unwrap() + std::time::Duration::from_secs(10));

        assert_eq!(stats.processing_rate(), Some(10.0));
    }

    #[test]
    fn test_processing_rate_no_data() {
        let stats = ProgressStats::default();
        assert_eq!(stats.processing_rate(), None);
    }

    #[test]
    fn test_processing_rate_zero_duration() {
        let mut stats = ProgressStats::default();
        let now = Instant::now();
        stats.started_at = Some(now);
        stats.finished_at = Some(now);
        stats.succeeded_items = 100;

        // Zero duration should return None
        assert_eq!(stats.processing_rate(), None);
    }

    #[test]
    fn test_total_duration() {
        let mut stats = ProgressStats::default();
        stats.started_at = Some(Instant::now());
        std::thread::sleep(std::time::Duration::from_millis(10));
        stats.finished_at = Some(Instant::now());

        let duration = stats.total_duration_secs().unwrap();
        assert!(duration >= 0.01);
        assert!(duration < 1.0); // Should be much less than 1 second
    }

    #[test]
    fn test_total_duration_not_finished() {
        let mut stats = ProgressStats::default();
        stats.started_at = Some(Instant::now());

        let duration = stats.total_duration_secs().unwrap();
        assert!(duration >= 0.0);
    }

    #[test]
    fn test_total_duration_no_start() {
        let stats = ProgressStats::default();
        assert_eq!(stats.total_duration_secs(), None);
    }

    #[test]
    fn test_record_operations() {
        let mut stats = ProgressStats::default();

        stats.record_cache_hit();
        stats.record_cache_hit();
        stats.record_embedding();
        stats.record_embedding();
        stats.record_embedding();
        stats.record_success();
        stats.record_success();
        stats.record_failure();

        assert_eq!(stats.cache_hits, 2);
        assert_eq!(stats.embeddings_generated, 3);
        assert_eq!(stats.succeeded_items, 2);
        assert_eq!(stats.failed_items, 1);
    }

    #[test]
    fn test_set_total() {
        let mut stats = ProgressStats::default();
        stats.set_total(42);

        assert_eq!(stats.total_items, 42);
    }

    #[test]
    fn test_mark_timing() {
        let mut stats = ProgressStats::default();

        assert!(stats.started_at.is_none());
        assert!(stats.finished_at.is_none());

        stats.mark_started();
        assert!(stats.started_at.is_some());
        assert!(stats.finished_at.is_none());

        stats.mark_finished();
        assert!(stats.started_at.is_some());
        assert!(stats.finished_at.is_some());
    }

    #[test]
    fn test_phase_stats() {
        let mut stats = ProgressStats::default();

        let phase = stats.phase("files");
        phase.items_processed = 10;

        assert_eq!(stats.phases.len(), 1);
        assert_eq!(stats.phases["files"].items_processed, 10);
    }

    #[test]
    fn test_complete_phase() {
        let mut stats = ProgressStats::default();

        stats.complete_phase("files".to_string(), 5.5);

        let phase = &stats.phases["files"];
        assert!(phase.completed);
        assert_eq!(phase.duration_secs, Some(5.5));
    }
}
