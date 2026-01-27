//! Result formatting for query output with timeline visualization.
//!
//! Provides multiple output formats:
//! - Markdown with confidence badges and color-coded age indicators
//! - JSON for programmatic consumption
//! - Plain text for terminal output

use crate::error::Result;
use crate::models::ContentType;
use crate::retrieval::{ConfidenceLevel, TimelineEvent, TimelineEventType};
use crate::results::EnhancedItem;
use serde_json;

/// Output format options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Markdown format with badges and colors.
    Markdown,
    /// JSON format for APIs.
    Json,
    /// Plain text format.
    Text,
}

/// Result formatter.
pub struct ResultFormatter {
    /// Output format.
    format: OutputFormat,
    /// Whether to show confidence badges.
    show_confidence: bool,
    /// Whether to show age indicators.
    show_age: bool,
    /// Whether to show Git context.
    show_git: bool,
    /// Maximum content length in output.
    max_content_length: usize,
}

impl ResultFormatter {
    /// Create a new formatter with default settings.
    pub fn new() -> Self {
        Self {
            format: OutputFormat::Markdown,
            show_confidence: true,
            show_age: true,
            show_git: true,
            max_content_length: 500,
        }
    }

    /// Set output format.
    pub fn with_format(mut self, format: OutputFormat) -> Self {
        self.format = format;
        self
    }

    /// Set whether to show confidence badges.
    pub fn with_confidence(mut self, show: bool) -> Self {
        self.show_confidence = show;
        self
    }

    /// Set whether to show age indicators.
    pub fn with_age(mut self, show: bool) -> Self {
        self.show_age = show;
        self
    }

    /// Set whether to show Git context.
    pub fn with_git(mut self, show: bool) -> Self {
        self.show_git = show;
        self
    }

    /// Set maximum content length.
    pub fn with_max_content_length(mut self, length: usize) -> Self {
        self.max_content_length = length;
        self
    }

    /// Format enhanced results as markdown.
    pub fn format_markdown(&self, results: &[EnhancedItem]) -> Result<String> {
        let mut output = String::new();

        if results.is_empty() {
            output.push_str("*No results found.*\n");
            return Ok(output);
        }

        output.push_str(&format!("## Found {} results\n\n", results.len()));

        for (idx, item) in results.iter().enumerate() {
            output.push_str(&format!("### {}. ", idx + 1));

            // Add confidence badge
            if self.show_confidence {
                output.push_str(&self.confidence_badge(item.confidence_level));
                output.push(' ');
            }

            // Add content type badge
            output.push_str(&self.content_type_badge(item.content_type));
            output.push('\n');

            // Add scores
            output.push_str(&format!(
                "**Score:** {:.2} (similarity: {:.2}, weight: {:.2})\n\n",
                item.final_score, item.similarity, item.temporal_weight
            ));

            // Add age description
            if self.show_age && !item.age_description.is_empty() {
                output.push_str(&format!("**Age:** {}\n\n", item.age_description));
            }

            // Add Git context
            if self.show_git {
                if let Some(ref git_info) = item.git_info {
                    output.push_str(&format!(
                        "**Git:** [`{}`]({}) - {} @ {}\n\n",
                        git_info.short_hash,
                        git_info.commit_link(None),
                        git_info.commit_message,
                        self.format_timestamp(git_info.commit_date)
                    ));
                }
            }

            // Add status indicators
            if item.is_current {
                output.push_str("**🟢 Current** - Matches Git HEAD\n\n");
            } else if item.is_deprecated {
                output.push_str("**🔴 Deprecated**\n\n");
                if let Some(ref superseded) = item.superseded_by {
                    output.push_str(&format!(
                        "→ Superseded by: {} ({})\n\n",
                        superseded.newer_id, superseded.reason
                    ));
                }
            }

            // Add content snippet
            let content = self.truncate_content(&item.content);
            output.push_str(&format!("```\n{}\n```\n\n", content));

            output.push_str("---\n\n");
        }

        Ok(output)
    }

    /// Format enhanced results as JSON.
    pub fn format_json(&self, results: &[EnhancedItem]) -> Result<String> {
        let json = serde_json::to_string_pretty(results)?;
        Ok(json)
    }

    /// Format enhanced results as plain text.
    pub fn format_text(&self, results: &[EnhancedItem]) -> Result<String> {
        let mut output = String::new();

        if results.is_empty() {
            output.push_str("No results found.\n");
            return Ok(output);
        }

        output.push_str(&format!("Found {} results\n\n", results.len()));

        for (idx, item) in results.iter().enumerate() {
            output.push_str(&format!("{}. [", idx + 1));

            // Add confidence level
            output.push_str(&format!("{:?}", item.confidence_level));
            output.push_str("] ");

            // Add content type
            output.push_str(&format!("{:?} - ", item.content_type));

            // Add score
            output.push_str(&format!("score: {:.2}", item.final_score));
            output.push('\n');

            // Add age
            if self.show_age && !item.age_description.is_empty() {
                output.push_str(&format!("   Age: {}\n", item.age_description));
            }

            // Add Git info
            if self.show_git {
                if let Some(ref git_info) = item.git_info {
                    output.push_str(&format!(
                        "   Git: {} - {}\n",
                        git_info.short_hash, git_info.commit_message
                    ));
                }
            }

            // Add status
            if item.is_current {
                output.push_str("   Status: Current (matches HEAD)\n");
            } else if item.is_deprecated {
                output.push_str("   Status: Deprecated\n");
            }

            output.push('\n');
        }

        Ok(output)
    }

    /// Format results with their configured format.
    pub fn format(&self, results: &[EnhancedItem]) -> Result<String> {
        match self.format {
            OutputFormat::Markdown => self.format_markdown(results),
            OutputFormat::Json => self.format_json(results),
            OutputFormat::Text => self.format_text(results),
        }
    }

    /// Format a feature timeline as markdown.
    pub fn format_timeline(&self, events: &[TimelineEvent]) -> Result<String> {
        let mut output = String::new();

        output.push_str("# Feature Timeline\n\n");

        if events.is_empty() {
            output.push_str("*No events found.*\n");
            return Ok(output);
        }

        for event in events {
            let event_str = match event.event_type {
                TimelineEventType::GitChange => {
                    format!(
                        "📜 **Git Change** - {} ({})",
                        event.description,
                        self.format_timestamp(event.timestamp.timestamp())
                    )
                }
                TimelineEventType::Discussion => {
                    format!(
                        "💬 **Discussion** - {} ({})",
                        event.description,
                        self.format_timestamp(event.timestamp.timestamp())
                    )
                }
                TimelineEventType::Implementation => {
                    format!(
                        "⚙️ **Implementation** - {} ({})",
                        event.description,
                        self.format_timestamp(event.timestamp.timestamp())
                    )
                }
            };
            output.push_str(&format!("{}\n\n", event_str));
        }

        Ok(output)
    }

    /// Get confidence badge emoji and label.
    fn confidence_badge(&self, level: ConfidenceLevel) -> String {
        match level {
            ConfidenceLevel::Highest => "🟢 **Highest**".to_string(),
            ConfidenceLevel::High => "🔵 **High**".to_string(),
            ConfidenceLevel::Medium => "🟡 **Medium**".to_string(),
            ConfidenceLevel::Low => "🟠 **Low**".to_string(),
            ConfidenceLevel::Lowest => "🔴 **Lowest**".to_string(),
        }
    }

    /// Get content type badge.
    fn content_type_badge(&self, content_type: ContentType) -> String {
        match content_type {
            ContentType::Session => "`Session`".to_string(),
            ContentType::Message => "`Message`".to_string(),
            ContentType::File => "`File`".to_string(),
            ContentType::Symbol => "`Symbol`".to_string(),
            ContentType::Commit => "`Commit`".to_string(),
            ContentType::GitDiff => "`GitDiff`".to_string(),
        }
    }

    /// Format timestamp as human-readable string.
    fn format_timestamp(&self, timestamp: i64) -> String {
        let now = chrono::Utc::now().timestamp();
        let age = now.saturating_sub(timestamp);

        if age < 60 {
            "just now".to_string()
        } else if age < 3600 {
            format!("{}m ago", age / 60)
        } else if age < 86400 {
            format!("{}h ago", age / 3600)
        } else if age < 604800 {
            format!("{}d ago", age / 86400)
        } else {
            format!("{}w ago", age / 604800)
        }
    }

    /// Truncate content to max length.
    fn truncate_content(&self, content: &str) -> String {
        if content.len() <= self.max_content_length {
            content.to_string()
        } else {
            let mut truncated = content.chars().take(self.max_content_length).collect::<String>();
            truncated.push_str("...");
            truncated
        }
    }
}

impl Default for ResultFormatter {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy query result (simplified version of EnhancedItem).
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Result ID.
    pub id: String,
    /// Content type.
    pub content_type: ContentType,
    /// Similarity score.
    pub similarity: f32,
    /// Temporal weight.
    pub temporal_weight: f32,
    /// Final score.
    pub final_score: f32,
    /// Content snippet.
    pub content: String,
}

impl From<EnhancedItem> for QueryResult {
    fn from(item: EnhancedItem) -> Self {
        Self {
            id: item.id,
            content_type: item.content_type,
            similarity: item.similarity,
            temporal_weight: item.temporal_weight,
            final_score: item.final_score,
            content: item.content,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrieval::ConfidenceLevel;
    use crate::models::ContentType;

    #[test]
    fn test_formatter_new() {
        let formatter = ResultFormatter::new();
        assert_eq!(formatter.format, OutputFormat::Markdown);
        assert!(formatter.show_confidence);
        assert!(formatter.show_age);
        assert!(formatter.show_git);
    }

    #[test]
    fn test_formatter_with_options() {
        let formatter = ResultFormatter::new()
            .with_format(OutputFormat::Json)
            .with_confidence(false)
            .with_age(false)
            .with_git(false);

        assert_eq!(formatter.format, OutputFormat::Json);
        assert!(!formatter.show_confidence);
        assert!(!formatter.show_age);
        assert!(!formatter.show_git);
    }

    #[test]
    fn test_format_empty_results() {
        let formatter = ResultFormatter::new();
        let output = formatter.format_markdown(&[]).unwrap();
        assert_eq!(output, "*No results found.*\n");
    }

    #[test]
    fn test_format_markdown() {
        let formatter = ResultFormatter::new().with_max_content_length(50);

        let item = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::Message,
            0.85,
            1640000000,
            "This is a test message that is somewhat long and should be truncated at fifty characters maximum.".to_string(),
        )
        .with_confidence(ConfidenceLevel::High)
        .mark_current();

        let output = formatter.format_markdown(&[item]).unwrap();
        assert!(output.contains("## Found 1 results"));
        assert!(output.contains("🔵 **High**"));
        assert!(output.contains("🟢 Current"));
        assert!(output.contains("Score:"));
    }

    #[test]
    fn test_format_json() {
        let formatter = ResultFormatter::new();

        let item = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::File,
            0.9,
            1640000000,
            "test content".to_string(),
        );

        let output = formatter.format_json(&[item]).unwrap();
        // Verify it's valid JSON and contains our data
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed[0]["id"], "test-id");
        assert_eq!(parsed[0]["similarity"], 0.9);
    }

    #[test]
    fn test_format_text() {
        let formatter = ResultFormatter::new();

        let item = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::Symbol,
            0.75,
            1640000000,
            "test content".to_string(),
        );

        let output = formatter.format_text(&[item]).unwrap();
        assert!(output.contains("Found 1 results"));
        assert!(output.contains("score: 0.75"));
    }

    #[test]
    fn test_confidence_badge() {
        let formatter = ResultFormatter::new();
        assert_eq!(formatter.confidence_badge(ConfidenceLevel::Highest), "🟢 **Highest**");
        assert_eq!(formatter.confidence_badge(ConfidenceLevel::High), "🔵 **High**");
        assert_eq!(formatter.confidence_badge(ConfidenceLevel::Medium), "🟡 **Medium**");
        assert_eq!(formatter.confidence_badge(ConfidenceLevel::Low), "🟠 **Low**");
        assert_eq!(formatter.confidence_badge(ConfidenceLevel::Lowest), "🔴 **Lowest**");
    }

    #[test]
    fn test_content_type_badge() {
        let formatter = ResultFormatter::new();
        assert_eq!(formatter.content_type_badge(ContentType::File), "`File`");
        assert_eq!(formatter.content_type_badge(ContentType::Commit), "`Commit`");
        assert_eq!(formatter.content_type_badge(ContentType::Message), "`Message`");
    }

    #[test]
    fn test_truncate_content() {
        let formatter = ResultFormatter::new().with_max_content_length(20);

        let long_content = "This is a very long content that should be truncated to twenty characters or less.";
        let truncated = formatter.truncate_content(long_content);

        assert!(truncated.len() <= 23); // 20 chars + "..."
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn test_query_result_from_enhanced() {
        let enhanced = EnhancedItem::new(
            "id".to_string(),
            ContentType::Session,
            0.8,
            1640000000,
            "content".to_string(),
        )
        .with_temporal_weight(1.2);

        let query: QueryResult = enhanced.into();
        assert_eq!(query.id, "id");
        assert_eq!(query.similarity, 0.8);
        assert_eq!(query.temporal_weight, 1.2);
        // Account for floating point precision
        assert!((query.final_score - 0.96).abs() < 0.001);
    }

    #[test]
    fn test_format_timestamp() {
        let formatter = ResultFormatter::new();
        let now = chrono::Utc::now().timestamp();

        // Test various ages
        assert_eq!(formatter.format_timestamp(now), "just now");
        assert!(formatter.format_timestamp(now - 120).contains("m ago"));
        assert!(formatter.format_timestamp(now - 7200).contains("h ago"));
        assert!(formatter.format_timestamp(now - 172800).contains("d ago"));
    }
}
