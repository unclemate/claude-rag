//! Timeline builder for feature evolution tracking.
//!
//! This module provides functionality to build feature timelines from indexed content,
//! helping users understand how a feature evolved over time through Git changes,
//! discussions, and implementations.

use crate::error::Result;
use crate::models::{Commit, Session};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, BTreeMap};

/// Feature timeline for tracking evolution.
pub struct FeatureTimeline {
    /// Cached events sorted by timestamp.
    events: Vec<TimelineEvent>,
    /// Events grouped by topic/feature.
    topics: HashMap<String, Vec<TimelineEvent>>,
    /// Current state markers.
    current_markers: HashMap<String, String>,
}

impl FeatureTimeline {
    /// Create a new timeline builder.
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            topics: HashMap::new(),
            current_markers: HashMap::new(),
        }
    }

    /// Build timeline from indexed content.
    ///
    /// # Arguments
    /// * `commits` - Git commits
    /// * `sessions` - Claude sessions
    /// * `topic` - Optional topic to filter by
    ///
    /// # Returns
    /// * `Vec<TimelineEvent>` - Sorted timeline events
    pub fn build(&self, commits: &[Commit], sessions: &[Session], topic: Option<&str>) -> Vec<TimelineEvent> {
        let mut events = Vec::new();

        // Add commit events
        for commit in commits {
            if let Some(t) = topic {
                if self.matches_topic(&commit.message, t) {
                    events.push(TimelineEvent {
                        timestamp: commit.commit_date,
                        event_type: TimelineEventType::GitChange,
                        description: self.format_commit_description(commit),
                    });
                }
            } else {
                events.push(TimelineEvent {
                    timestamp: commit.commit_date,
                    event_type: TimelineEventType::GitChange,
                    description: self.format_commit_description(commit),
                });
            }
        }

        // Add session events
        for session in sessions {
            if let Some(title) = &session.title {
                if let Some(t) = topic {
                    if self.matches_topic(title, t) {
                        events.push(TimelineEvent {
                            timestamp: session.started_at,
                            event_type: TimelineEventType::Discussion,
                            description: self.format_session_description(session),
                        });
                    }
                } else {
                    events.push(TimelineEvent {
                        timestamp: session.started_at,
                        event_type: TimelineEventType::Discussion,
                        description: self.format_session_description(session),
                    });
                }
            }
        }

        // Sort by timestamp
        events.sort_by_key(|e| e.timestamp);
        events.reverse(); // Most recent first

        events
    }

    /// Build timeline for a specific feature/topic.
    ///
    /// # Arguments
    /// * `topic` - Feature/topic name
    ///
    /// # Returns
    /// * `Vec<TimelineEvent>` - Timeline events for the topic
    pub fn build_for_topic(&self, _topic: &str) -> Result<Vec<TimelineEvent>> {
        // Return empty for now - would require storage integration
        Ok(Vec::new())
    }

    /// Cluster events by feature/topic.
    ///
    /// # Arguments
    /// * `events` - Timeline events
    ///
    /// # Returns
    /// * `HashMap<String, Vec<TimelineEvent>>` - Events grouped by topic
    pub fn cluster_by_topic(&self, events: &[TimelineEvent]) -> HashMap<String, Vec<TimelineEvent>> {
        let mut clusters: HashMap<String, Vec<TimelineEvent>> = HashMap::new();

        for event in events {
            let topic = self.extract_topic(&event.description);
            clusters.entry(topic).or_default().push(event.clone());
        }

        // Sort events within each topic
        for events in clusters.values_mut() {
            events.sort_by_key(|e| e.timestamp);
        }

        clusters
    }

    /// Identify current state vs historical events.
    ///
    /// # Arguments
    /// * `events` - Timeline events
    /// * `days_threshold` - Days to consider "current" (default 7)
    ///
    /// # Returns
    /// * `(Vec<TimelineEvent>, Vec<TimelineEvent>)` - (current, historical)
    pub fn separate_current_historical(
        &self,
        events: &[TimelineEvent],
        days_threshold: i64,
    ) -> (Vec<TimelineEvent>, Vec<TimelineEvent>) {
        let threshold = Utc::now() - chrono::Duration::days(days_threshold);

        let current: Vec<TimelineEvent> = events
            .iter()
            .filter(|e| e.timestamp > threshold)
            .cloned()
            .collect();

        let historical: Vec<TimelineEvent> = events
            .iter()
            .filter(|e| e.timestamp <= threshold)
            .cloned()
            .collect();

        (current, historical)
    }

    /// Mark deprecated items based on Git history.
    ///
    /// # Arguments
    /// * `commits` - Git commits to analyze
    /// * `file_path` - File path to check
    ///
    /// # Returns
    /// * `Option<String>` - Commit hash that deprecated the file, if any
    pub fn find_deprecating_commit(&self, commits: &[Commit], file_path: &str) -> Option<String> {
        // Look for commits that delete the file
        for commit in commits {
            if commit.message.to_lowercase().contains("deprecate") ||
               commit.message.to_lowercase().contains("remove") {
                if commit.message.contains(file_path) {
                    return Some(commit.id.clone());
                }
            }
        }
        None
    }

    /// Extract topic from description.
    fn extract_topic(&self, description: &str) -> String {
        // Try to extract from conventional commit format
        if let Some(colon_pos) = description.find(':') {
            let before_colon = &description[..colon_pos];
            // Extract scope if present
            if let Some(open_paren) = before_colon.find('(') {
                if let Some(close_paren) = before_colon.find(')') {
                    return before_colon[open_paren + 1..close_paren].to_string();
                }
            }
            // Use type as topic
            before_colon.to_string()
        } else {
            // Use first word as topic
            description
                .split_whitespace()
                .next()
                .unwrap_or("general")
                .to_string()
        }
    }

    /// Check if content matches a topic.
    fn matches_topic(&self, content: &str, topic: &str) -> bool {
        let content_lower = content.to_lowercase();
        let topic_lower = topic.to_lowercase();

        content_lower.contains(&topic_lower) ||
        self.extract_topic(content).to_lowercase() == topic_lower
    }

    /// Format commit description for timeline.
    fn format_commit_description(&self, commit: &Commit) -> String {
        let mut desc = String::new();

        if let Some(typ) = &commit.conv_type {
            desc.push_str(typ);
            if let Some(scope) = &commit.conv_scope {
                desc.push('(');
                desc.push_str(scope);
                desc.push(')');
            }
            desc.push_str(": ");
        }

        desc.push_str(&commit.message_summary);

        if commit.is_breaking {
            desc.push_str(" [BREAKING]");
        }

        desc
    }

    /// Format session description for timeline.
    fn format_session_description(&self, session: &Session) -> String {
        session
            .title
            .as_ref()
            .cloned()
            .unwrap_or_else(|| format!("Session {}", session.id))
    }

    /// Create event summaries grouped by time period.
    ///
    /// # Arguments
    /// * `events` - Timeline events
    /// * `period_days` - Days per period (default 30)
    ///
    /// # Returns
    /// * `Vec<(String, usize)>` - List of (period_label, event_count)
    pub fn summarize_by_period(&self, events: &[TimelineEvent], period_days: i64) -> Vec<(String, usize)> {
        if events.is_empty() {
            return Vec::new();
        }

        let mut summaries: BTreeMap<String, usize> = BTreeMap::new();
        let now = Utc::now();

        for event in events {
            let days_ago = (now - event.timestamp).num_days().max(0);
            let period_index = days_ago / period_days;
            let period_label = self.format_period_label(period_index, period_days);
            *summaries.entry(period_label).or_insert(0) += 1;
        }

        summaries.into_iter().rev().collect()
    }

    /// Format period label.
    fn format_period_label(&self, period_index: i64, period_days: i64) -> String {
        if period_index == 0 {
            format!("Last {} days", period_days)
        } else {
            format!("{}-{} days ago", period_index * period_days, (period_index + 1) * period_days)
        }
    }

    /// Get timeline statistics.
    ///
    /// # Arguments
    /// * `events` - Timeline events
    ///
    /// # Returns
    /// * `TimelineStats` - Statistics about the timeline
    pub fn get_stats(&self, events: &[TimelineEvent]) -> TimelineStats {
        let mut git_changes = 0;
        let mut discussions = 0;
        let mut implementations = 0;

        for event in events {
            match event.event_type {
                TimelineEventType::GitChange => git_changes += 1,
                TimelineEventType::Discussion => discussions += 1,
                TimelineEventType::Implementation => implementations += 1,
            }
        }

        let time_span = if events.len() >= 2 {
            Some(events.last().unwrap().timestamp - events.first().unwrap().timestamp)
        } else {
            None
        };

        TimelineStats {
            total_events: events.len(),
            git_changes,
            discussions,
            implementations,
            time_span_days: time_span.map(|d| d.num_days()),
        }
    }
}

impl Default for FeatureTimeline {
    fn default() -> Self {
        Self::new()
    }
}

/// A timeline event.
#[derive(Debug, Clone)]
pub struct TimelineEvent {
    /// Event timestamp.
    pub timestamp: DateTime<Utc>,
    /// Event type.
    pub event_type: TimelineEventType,
    /// Event description.
    pub description: String,
}

/// Type of timeline event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineEventType {
    /// Git change (commit/diff).
    GitChange,
    /// Discussion (session).
    Discussion,
    /// Implementation (code).
    Implementation,
}

/// Timeline statistics.
#[derive(Debug, Clone)]
pub struct TimelineStats {
    /// Total number of events.
    pub total_events: usize,
    /// Number of Git change events.
    pub git_changes: usize,
    /// Number of discussion events.
    pub discussions: usize,
    /// Number of implementation events.
    pub implementations: usize,
    /// Time span in days.
    pub time_span_days: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn create_test_commit(id: &str, days_ago: i64, message: &str) -> Commit {
        let short_hash = if id.len() >= 7 { &id[..7] } else { id };
        Commit {
            id: id.to_string(),
            short_hash: short_hash.to_string(),
            project_path: "/test".to_string(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
            commit_date: Utc::now() - Duration::days(days_ago),
            message: message.to_string(),
            message_summary: message.to_string(),
            conv_type: Some("feat".to_string()),
            conv_scope: Some("api".to_string()),
            is_breaking: false,
            parent_hashes: vec![],
            files_changed: 1,
            insertions: 10,
            deletions: 0,
        }
    }

    fn create_test_session(id: &str, days_ago: i64, title: &str) -> Session {
        Session {
            id: id.to_string(),
            title: Some(title.to_string()),
            project_path: "/test".to_string(),
            started_at: Utc::now() - Duration::days(days_ago),
            ended_at: None,
            message_count: 5,
            indexed: false,
        }
    }

    #[test]
    fn test_timeline_new() {
        let timeline = FeatureTimeline::new();
        assert_eq!(timeline.events.len(), 0);
    }

    #[test]
    fn test_build_with_commits() {
        let timeline = FeatureTimeline::new();
        let commits = vec![
            create_test_commit("abc123", 5, "feat(api): add endpoint"),
            create_test_commit("def456", 3, "fix(api): resolve bug"),
        ];

        let events = timeline.build(&commits, &[], None);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, TimelineEventType::GitChange);
        // Most recent first
        assert_eq!(events[0].description.contains("fix"), true);
    }

    #[test]
    fn test_build_with_sessions() {
        let timeline = FeatureTimeline::new();
        let sessions = vec![
            create_test_session("sess1", 2, "Discussion about API"),
        ];

        let events = timeline.build(&[], &sessions, None);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, TimelineEventType::Discussion);
    }

    #[test]
    fn test_build_with_topic_filter() {
        let timeline = FeatureTimeline::new();
        let commits = vec![
            create_test_commit("abc123", 5, "feat(api): add endpoint"),
            create_test_commit("def456", 3, "feat(db): add index"),
        ];

        let events = timeline.build(&commits, &[], Some("api"));

        assert_eq!(events.len(), 1);
        assert!(events[0].description.contains("api"));
    }

    #[test]
    fn test_cluster_by_topic() {
        let timeline = FeatureTimeline::new();
        let commits = vec![
            create_test_commit("abc123", 5, "feat: add endpoint"),
            create_test_commit("def456", 3, "fix: resolve bug"),
        ];

        let events = timeline.build(&commits, &[], None);
        let clusters = timeline.cluster_by_topic(&events);

        // Both commits will be clustered under "feat" and "fix" based on type
        assert!(!clusters.is_empty());
    }

    #[test]
    fn test_separate_current_historical() {
        let timeline = FeatureTimeline::new();
        let commits = vec![
            create_test_commit("abc123", 2, "feat: recent change"),
            create_test_commit("def456", 10, "feat: old change"),
        ];

        let events = timeline.build(&commits, &[], None);
        let (current, historical) = timeline.separate_current_historical(&events, 7);

        assert_eq!(current.len(), 1);
        assert_eq!(historical.len(), 1);
    }

    #[test]
    fn test_extract_topic() {
        let timeline = FeatureTimeline::new();

        assert_eq!(timeline.extract_topic("feat(api): add endpoint"), "api");
        assert_eq!(timeline.extract_topic("fix: resolve bug"), "fix");
        assert_eq!(timeline.extract_topic("random text"), "random");
    }

    #[test]
    fn test_summarize_by_period() {
        let timeline = FeatureTimeline::new();
        let commits = vec![
            create_test_commit("abc123", 1, "feat: change 1"),
            create_test_commit("def456", 5, "feat: change 2"),
            create_test_commit("ghi789", 35, "feat: change 3"),
        ];

        let events = timeline.build(&commits, &[], None);
        let summaries = timeline.summarize_by_period(&events, 30);

        // Should have 2 periods: 0-30 days and 30-60 days
        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn test_get_stats() {
        let timeline = FeatureTimeline::new();
        let commits = vec![
            create_test_commit("abc123", 1, "feat: change 1"),
            create_test_commit("def456", 5, "feat: change 2"),
        ];
        let sessions = vec![
            create_test_session("sess1", 2, "Discussion 1"),
        ];

        let events = timeline.build(&commits, &sessions, None);
        let stats = timeline.get_stats(&events);

        assert_eq!(stats.total_events, 3);
        assert_eq!(stats.git_changes, 2);
        assert_eq!(stats.discussions, 1);
        assert!(stats.time_span_days.is_some());
    }

    #[test]
    fn test_find_deprecating_commit() {
        let timeline = FeatureTimeline::new();
        let commits = vec![
            create_test_commit("abc123", 5, "feat: add feature"),
            create_test_commit("def456", 2, "deprecate: remove old_module.rs"),
        ];

        let result = timeline.find_deprecating_commit(&commits, "old_module.rs");
        assert_eq!(result, Some("def456".to_string()));
    }

    #[test]
    fn test_format_commit_description() {
        let timeline = FeatureTimeline::new();
        let commit = create_test_commit("abc123", 1, "feat(api): add endpoint");

        let desc = timeline.format_commit_description(&commit);
        assert!(desc.contains("feat(api):"));
        assert!(desc.contains("add endpoint"));
    }

    #[test]
    fn test_format_session_description() {
        let timeline = FeatureTimeline::new();
        let session = create_test_session("sess1", 1, "Test discussion");

        let desc = timeline.format_session_description(&session);
        assert_eq!(desc, "Test discussion");
    }
}
