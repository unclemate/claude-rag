//! Progress reporting module for indexing operations.
//!
//! This module provides a unified progress reporting system with:
//! - ProgressReporter trait for flexible backends
//! - ProgressBarReporter using indicatif for rich progress display
//! - CallbackReporter for backward compatibility with simple callbacks
//! - Statistics collection and reporting
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```
//! use claude_rag::{ProgressBarReporter, ProgressReporter, ProgressEvent};
//!
//! let reporter = ProgressBarReporter::default_style();
//!
//! // Report a phase starting
//! reporter.report(ProgressEvent::PhaseStarted {
//!     name: "indexing".to_string(),
//!     total: 100,
//! });
//!
//! // Report items being processed
//! for i in 0..100 {
//!     reporter.report(ProgressEvent::ItemProgress {
//!         current: i + 1,
//!         total: 100,
//!         name: format!("file_{}.rs", i),
//!     });
//!     reporter.report(ProgressEvent::ItemCompleted {
//!         name: format!("file_{}.rs", i),
//!         success: true,
//!     });
//! }
//!
//! reporter.finish();
//!
//! // Get statistics
//! let stats = reporter.stats();
//! println!("Processed {} items in {:.2}s",
//!     stats.succeeded_items,
//!     stats.total_duration_secs().unwrap_or(0.0));
//! ```
//!
//! ## Using with Configuration
//!
//! ```
//! use claude_rag::{ProgressBarReporter, ProgressStyle, ProgressStyleType};
//!
//! // Create a custom style
//! let style = ProgressStyle {
//!     style_type: ProgressStyleType::Compact,
//!     show_cache_stats: true,
//!     show_processing_rate: true,
//!     show_eta: false,
//! };
//!
//! let reporter = ProgressBarReporter::new(style);
//! ```
//!
//! ## Using CallbackReporter for Backward Compatibility
//!
//! ```
//! use claude_rag::{CallbackReporter, ProgressReporter};
//!
//! let messages = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
//! let msgs = messages.clone();
//!
//! let reporter = CallbackReporter::new(move |msg| {
//!     msgs.lock().unwrap().push(msg.to_string());
//!     println!("{}", msg);
//! });
//!
//! // Use the same ProgressReporter interface
//! reporter.report(ProgressEvent::PhaseStarted {
//!     name: "test".to_string(),
//!     total: 10,
//! });
//! ```
//!
//! ## Silent Mode (No Progress Display)
//!
//! ```
//! use claude_rag::{ProgressBarReporter, ProgressStyleType};
//!
//! let reporter = ProgressBarReporter::silent();
//!
//! // All events are silently recorded for statistics
//! reporter.report(ProgressEvent::PhaseStarted {
//!     name: "background".to_string(),
//!     total: 1000,
//! });
//! // ... process items ...
//! reporter.finish();
//!
//! // Still get accurate statistics
//! let stats = reporter.stats();
//! println!("Cache hit rate: {:.1}%", stats.cache_hit_rate().unwrap_or(0.0) * 100.0);
//! ```

mod reporter;
mod stats;
mod style;

pub use reporter::{ProgressReporter, CallbackReporter, ProgressBarReporter};
pub use stats::{ProgressStats, PhaseStats};
pub use style::{ProgressStyle, ProgressStyleType};

/// Progress event types.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// A phase has started.
    PhaseStarted {
        /// Phase name.
        name: String,
        /// Total items in phase.
        total: usize,
    },
    /// Item processing progress update.
    ItemProgress {
        /// Current item count.
        current: usize,
        /// Total items.
        total: usize,
        /// Current item name.
        name: String,
    },
    /// An item has been completed.
    ItemCompleted {
        /// Item name.
        name: String,
        /// Whether processing succeeded.
        success: bool,
    },
    /// Cache hit event.
    CacheHit,
    /// Embedding generated event.
    EmbeddingGenerated,
    /// A phase has completed.
    PhaseCompleted {
        /// Phase name.
        name: String,
        /// Duration in seconds.
        duration_secs: f64,
    },
    /// Error event.
    Error {
        /// Error message.
        message: String,
    },
}

/// Extension trait providing convenience methods for ProgressReporter.
pub trait ProgressReporterExt: ProgressReporter {
    /// Report a simple phase completion with automatic timing.
    ///
    /// # Arguments
    /// * `phase_name` - Name of the completed phase
    /// * `_succeeded` - Number of successfully processed items (unused but kept for API consistency)
    /// * `_failed` - Number of failed items (unused but kept for API consistency)
    /// * `duration_secs` - Duration in seconds
    fn report_phase_completion(
        &self,
        phase_name: &str,
        _succeeded: usize,
        _failed: usize,
        duration_secs: f64,
    ) {
        use crate::progress::ProgressEvent;
        self.report(ProgressEvent::PhaseCompleted {
            name: phase_name.to_string(),
            duration_secs,
        });
    }
}

/// Blanket implementation for all ProgressReporter types.
impl<T: ProgressReporter> ProgressReporterExt for T {}
