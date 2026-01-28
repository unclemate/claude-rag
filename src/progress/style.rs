//! Progress bar style configuration.

use serde::{Deserialize, Serialize};

/// Progress display style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgressStyleType {
    /// Default rich progress bar with colors and animations.
    Default,
    /// Compact single-line progress bar.
    Compact,
    /// Silent mode - no progress display, only final stats.
    Silent,
}

impl Default for ProgressStyleType {
    fn default() -> Self {
        Self::Default
    }
}

impl std::str::FromStr for ProgressStyleType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "default" => Ok(Self::Default),
            "compact" => Ok(Self::Compact),
            "silent" => Ok(Self::Silent),
            _ => Err(format!("Invalid progress style: {}", s)),
        }
    }
}

/// Progress style configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressStyle {
    /// Style type.
    #[serde(default)]
    pub style_type: ProgressStyleType,
    /// Show cache statistics.
    #[serde(default = "default_show_cache_stats")]
    pub show_cache_stats: bool,
    /// Show processing rate.
    #[serde(default = "default_show_processing_rate")]
    pub show_processing_rate: bool,
    /// Show ETA.
    #[serde(default = "default_show_eta")]
    pub show_eta: bool,
}

fn default_show_cache_stats() -> bool {
    true
}

fn default_show_processing_rate() -> bool {
    true
}

fn default_show_eta() -> bool {
    true
}

impl Default for ProgressStyle {
    fn default() -> Self {
        Self {
            style_type: ProgressStyleType::default(),
            show_cache_stats: default_show_cache_stats(),
            show_processing_rate: default_show_processing_rate(),
            show_eta: default_show_eta(),
        }
    }
}

impl From<ProgressStyleType> for ProgressStyle {
    fn from(style_type: ProgressStyleType) -> Self {
        Self {
            style_type,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_progress_style_type_default() {
        assert_eq!(ProgressStyleType::default(), ProgressStyleType::Default);
    }

    #[test]
    fn test_progress_style_type_from_str() {
        assert_eq!(ProgressStyleType::from_str("default").unwrap(), ProgressStyleType::Default);
        assert_eq!(ProgressStyleType::from_str("compact").unwrap(), ProgressStyleType::Compact);
        assert_eq!(ProgressStyleType::from_str("silent").unwrap(), ProgressStyleType::Silent);
        assert_eq!(ProgressStyleType::from_str("DEFAULT").unwrap(), ProgressStyleType::Default);
        assert_eq!(ProgressStyleType::from_str("COMPACT").unwrap(), ProgressStyleType::Compact);
    }

    #[test]
    fn test_progress_style_type_from_str_invalid() {
        assert!(ProgressStyleType::from_str("invalid").is_err());
    }

    #[test]
    fn test_progress_style_default() {
        let style = ProgressStyle::default();
        assert_eq!(style.style_type, ProgressStyleType::Default);
        assert!(style.show_cache_stats);
        assert!(style.show_processing_rate);
        assert!(style.show_eta);
    }

    #[test]
    fn test_progress_style_from_type() {
        let style: ProgressStyle = ProgressStyleType::Compact.into();
        assert_eq!(style.style_type, ProgressStyleType::Compact);
        assert!(style.show_cache_stats);
        assert!(style.show_processing_rate);
        assert!(style.show_eta);
    }
}
