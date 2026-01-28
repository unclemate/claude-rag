//! Progress reporter implementations.

use crate::progress::{ProgressEvent, ProgressStats, ProgressStyle, ProgressStyleType};
use std::sync::{Arc, Mutex};

/// Default template for rich progress bars.
const DEFAULT_PROGRESS_TEMPLATE: &str =
    "{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}";

/// Compact template for progress bars.
const COMPACT_PROGRESS_TEMPLATE: &str =
    "[{elapsed_precise}] {pos}/{len} {msg}";

/// Default progress bar characters.
const DEFAULT_PROGRESS_CHARS: &str = "##+-";

/// Compact progress bar characters.
const COMPACT_PROGRESS_CHARS: &str = "=>-";

/// Trait for progress reporting backends.
pub trait ProgressReporter: Send + Sync {
    /// Report a progress event.
    fn report(&self, event: ProgressEvent);

    /// Finish progress reporting.
    fn finish(&self);

    /// Get collected statistics.
    fn stats(&self) -> ProgressStats;
}

/// Callback-based progress reporter for backward compatibility.
pub struct CallbackReporter {
    callback: Arc<dyn Fn(&str) + Send + Sync>,
    stats: Arc<Mutex<ProgressStats>>,
}

impl CallbackReporter {
    /// Create a new callback reporter.
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        Self {
            callback: Arc::new(callback),
            stats: Arc::new(Mutex::new(ProgressStats::default())),
        }
    }
}

impl ProgressReporter for CallbackReporter {
    fn report(&self, event: ProgressEvent) {
        // Update stats first (before any early returns)
        let mut stats = self.stats.lock().unwrap();
        match &event {
            ProgressEvent::PhaseStarted { total, .. } => {
                stats.set_total(*total);
                stats.mark_started();
            }
            ProgressEvent::ItemCompleted { success: true, .. } => {
                stats.record_success();
            }
            ProgressEvent::ItemCompleted { success: false, .. } => {
                stats.record_failure();
            }
            ProgressEvent::CacheHit => {
                stats.record_cache_hit();
            }
            ProgressEvent::EmbeddingGenerated => {
                stats.record_embedding();
            }
            ProgressEvent::PhaseCompleted { .. } => {
                stats.mark_finished();
            }
            _ => {}
        }
        drop(stats);

        // Generate message for callback (if needed)
        let msg: Option<String> = match &event {
            ProgressEvent::PhaseStarted { .. } => Some("Starting...".to_string()),
            ProgressEvent::ItemProgress { .. } => None, // Don't spam for progress updates
            ProgressEvent::PhaseCompleted { .. } => Some("Phase complete".to_string()),
            ProgressEvent::ItemCompleted { success: true, .. } => Some("Item complete".to_string()),
            ProgressEvent::ItemCompleted { success: false, .. } => Some("Item failed".to_string()),
            ProgressEvent::CacheHit => None, // Don't spam for cache hits
            ProgressEvent::EmbeddingGenerated => None, // Don't spam for embeddings
            ProgressEvent::Error { message } => {
                (self.callback)(format!("Error: {}", message).as_str());
                None
            }
        };

        // Call callback if we have a message
        if let Some(msg) = msg {
            (self.callback)(&msg);
        }
    }

    fn finish(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.mark_finished();
    }

    fn stats(&self) -> ProgressStats {
        self.stats.lock().unwrap().clone()
    }
}

/// Rich progress bar reporter using indicatif.
pub struct ProgressBarReporter {
    inner: Arc<Mutex<ProgressBarReporterInner>>,
}

struct ProgressBarReporterInner {
    style: ProgressStyle,
    stats: ProgressStats,
    // Option<Box<...>> for dynamic multi-progress based on style
    multi: Option<indicatif::MultiProgress>,
    // Current progress bars for different phases
    current_phase_bar: Option<indicatif::ProgressBar>,
}

impl ProgressBarReporter {
    /// Create a new progress bar reporter.
    pub fn new(style: ProgressStyle) -> Self {
        let multi = match style.style_type {
            ProgressStyleType::Silent => None,
            _ => Some(indicatif::MultiProgress::new()),
        };

        Self {
            inner: Arc::new(Mutex::new(ProgressBarReporterInner {
                style,
                stats: ProgressStats::default(),
                multi,
                current_phase_bar: None,
            })),
        }
    }

    /// Create reporter with default style.
    pub fn default_style() -> Self {
        Self::new(ProgressStyle::default())
    }

    /// Create a silent reporter (no progress display).
    pub fn silent() -> Self {
        Self::new(ProgressStyleType::Silent.into())
    }

    /// Create a compact reporter.
    pub fn compact() -> Self {
        Self::new(ProgressStyleType::Compact.into())
    }
}

impl ProgressReporter for ProgressBarReporter {
    fn report(&self, event: ProgressEvent) {
        let mut inner = self.inner.lock().unwrap();

        // Update stats first
        match &event {
            ProgressEvent::PhaseStarted { total, .. } => {
                inner.stats.set_total(*total);
                inner.stats.mark_started();
            }
            ProgressEvent::ItemCompleted { success: true, .. } => {
                inner.stats.record_success();
            }
            ProgressEvent::ItemCompleted { success: false, .. } => {
                inner.stats.record_failure();
            }
            ProgressEvent::CacheHit => {
                inner.stats.record_cache_hit();
            }
            ProgressEvent::EmbeddingGenerated => {
                inner.stats.record_embedding();
            }
            _ => {}
        }

        // Early exit for silent mode
        if matches!(inner.style.style_type, ProgressStyleType::Silent) {
            return;
        }

        // Handle event for progress bar
        match event {
            ProgressEvent::PhaseStarted { name, total } => {
                // Clear previous phase bar if exists
                if let Some(bar) = inner.current_phase_bar.take() {
                    bar.finish();
                }

                // Create new progress bar
                let multi = inner.multi.as_ref().unwrap();
                let bar = match inner.style.style_type {
                    ProgressStyleType::Default => {
                        let style = indicatif::ProgressStyle::default_bar()
                            .template(DEFAULT_PROGRESS_TEMPLATE)
                            .expect("invalid progress template")
                            .progress_chars(DEFAULT_PROGRESS_CHARS);
                        multi.add(indicatif::ProgressBar::new(total as u64).with_style(style))
                    }
                    ProgressStyleType::Compact => {
                        let style = indicatif::ProgressStyle::default_bar()
                            .template(COMPACT_PROGRESS_TEMPLATE)
                            .expect("invalid progress template")
                            .progress_chars(COMPACT_PROGRESS_CHARS);
                        multi.add(indicatif::ProgressBar::new(total as u64).with_style(style))
                    }
                    ProgressStyleType::Silent => unreachable!(),
                };

                bar.set_message(name);
                inner.current_phase_bar = Some(bar);
            }

            ProgressEvent::ItemProgress {
                current,
                total,
                name,
            } => {
                if let Some(bar) = &inner.current_phase_bar {
                    bar.set_length(total as u64);
                    bar.set_position(current as u64);
                    bar.set_message(name);
                }
            }

            ProgressEvent::ItemCompleted { name, success } => {
                if let Some(bar) = &inner.current_phase_bar {
                    if success {
                        bar.inc(1);
                    }
                    bar.set_message(name);
                }
            }

            ProgressEvent::PhaseCompleted { name, duration_secs } => {
                inner.stats.complete_phase(name.clone(), duration_secs);
                if let Some(bar) = &inner.current_phase_bar {
                    bar.finish_with_message(format!("{} complete", name));
                }
                inner.current_phase_bar = None;
            }

            ProgressEvent::Error { message } => {
                if let Some(bar) = &inner.current_phase_bar {
                    bar.suspend(|| {
                        eprintln!("Error: {}", message);
                    });
                } else {
                    eprintln!("Error: {}", message);
                }
            }

            _ => {}
        }
    }

    fn finish(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.stats.mark_finished();

        if let Some(bar) = inner.current_phase_bar.take() {
            bar.finish();
        }
    }

    fn stats(&self) -> ProgressStats {
        self.inner.lock().unwrap().stats.clone()
    }
}

/// Backward compatibility: Implement ProgressReporter for function pointers.
impl<F> ProgressReporter for F
where
    F: Fn(&str) + Send + Sync,
{
    fn report(&self, event: ProgressEvent) {
        let msg: &'static str = match event {
            ProgressEvent::PhaseStarted { .. } => "Starting...",
            ProgressEvent::ItemProgress { .. } => "Processing...",
            ProgressEvent::PhaseCompleted { .. } => "Complete",
            _ => return,
        };
        self(msg);
    }

    fn finish(&self) {
        // Nothing to finish for simple callback
    }

    fn stats(&self) -> ProgressStats {
        ProgressStats::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_callback_reporter() {
        let messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let msgs = messages.clone();

        let reporter = CallbackReporter::new(move |msg| {
            msgs.lock().unwrap().push(msg.to_string());
        });

        reporter.report(ProgressEvent::PhaseStarted {
            name: "test".to_string(),
            total: 10,
        });

        reporter.report(ProgressEvent::ItemProgress {
            current: 5,
            total: 10,
            name: "item.rs".to_string(),
        });

        reporter.report(ProgressEvent::ItemCompleted {
            name: "item.rs".to_string(),
            success: true,
        });

        reporter.finish();

        let stats = reporter.stats();
        assert_eq!(stats.succeeded_items, 1);
        assert!(stats.started_at.is_some());
        assert!(stats.finished_at.is_some());

        let msgs = messages.lock().unwrap();
        assert_eq!(msgs.len(), 2); // PhaseStarted and ItemCompleted (ItemProgress is silent)
    }

    #[test]
    fn test_callback_reporter_error() {
        let messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let msgs = messages.clone();

        let reporter = CallbackReporter::new(move |msg| {
            msgs.lock().unwrap().push(msg.to_string());
        });

        reporter.report(ProgressEvent::Error {
            message: "test error".to_string(),
        });

        let msgs = messages.lock().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], "Error: test error");
    }

    #[test]
    fn test_callback_reporter_cache_hits() {
        let reporter = CallbackReporter::new(|_| {});

        reporter.report(ProgressEvent::CacheHit);
        reporter.report(ProgressEvent::CacheHit);
        reporter.report(ProgressEvent::EmbeddingGenerated);

        let stats = reporter.stats();
        assert_eq!(stats.cache_hits, 2);
        assert_eq!(stats.embeddings_generated, 1);
        assert_eq!(stats.cache_hit_rate(), Some(2.0 / 3.0));
    }

    #[test]
    fn test_progress_bar_reporter_default() {
        let reporter = ProgressBarReporter::default_style();
        assert!(matches!(
            reporter.inner.lock().unwrap().style.style_type,
            ProgressStyleType::Default
        ));
    }

    #[test]
    fn test_progress_bar_reporter_silent() {
        let reporter = ProgressBarReporter::silent();
        assert!(matches!(
            reporter.inner.lock().unwrap().style.style_type,
            ProgressStyleType::Silent
        ));
        // Silent mode should not have multi-progress
        assert!(reporter.inner.lock().unwrap().multi.is_none());
    }

    #[test]
    fn test_progress_bar_reporter_compact() {
        let reporter = ProgressBarReporter::compact();
        assert!(matches!(
            reporter.inner.lock().unwrap().style.style_type,
            ProgressStyleType::Compact
        ));
    }

    #[test]
    fn test_fn_progress_reporter() {
        use std::sync::{Arc, Mutex};

        let messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let msgs = messages.clone();

        let callback = move |msg: &str| {
            msgs.lock().unwrap().push(msg.to_string());
        };

        callback.report(ProgressEvent::PhaseStarted {
            name: "test".to_string(),
            total: 10,
        });

        callback.report(ProgressEvent::ItemProgress {
            current: 5,
            total: 10,
            name: "item.rs".to_string(),
        });

        let msgs = messages.lock().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], "Starting...");
        assert_eq!(msgs[1], "Processing...");
    }

    #[test]
    fn test_concurrent_callback_reporter() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        let messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let reporter = Arc::new(CallbackReporter::new({
            let msgs = messages.clone();
            move |msg| {
                msgs.lock().unwrap().push(msg.to_string());
            }
        }));

        // Spawn multiple threads reporting concurrently
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let reporter = reporter.clone();
                thread::spawn(move || {
                    reporter.report(ProgressEvent::PhaseStarted {
                        name: format!("thread-{}", i),
                        total: 10,
                    });
                    for j in 0..10 {
                        reporter.report(ProgressEvent::ItemCompleted {
                            name: format!("item-{}-{}", i, j),
                            success: true,
                        });
                    }
                    reporter.report(ProgressEvent::PhaseCompleted {
                        name: format!("thread-{}", i),
                        duration_secs: 1.0,
                    });
                })
            })
            .collect();

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify stats are consistent
        let stats = reporter.stats();
        assert_eq!(stats.succeeded_items, 50); // 5 threads * 10 items
        assert!(stats.started_at.is_some());
        assert!(stats.finished_at.is_some());

        // Verify messages were recorded (order may vary)
        let msgs = messages.lock().unwrap();
        // Each thread: 1 PhaseStarted + 10 ItemCompleted + 1 PhaseCompleted = 12 messages
        // 5 threads * 12 = 60 messages
        assert_eq!(msgs.len(), 60);
    }

    #[test]
    fn test_concurrent_progress_bar_reporter_stats() {
        use std::thread;

        let reporter = Arc::new(ProgressBarReporter::silent());

        // Spawn multiple threads reporting concurrently
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let reporter = reporter.clone();
                thread::spawn(move || {
                    reporter.report(ProgressEvent::PhaseStarted {
                        name: format!("thread-{}", i),
                        total: 10,
                    });
                    for j in 0..10 {
                        reporter.report(ProgressEvent::ItemCompleted {
                            name: format!("item-{}-{}", i, j),
                            success: true,
                        });
                    }
                    reporter.report(ProgressEvent::CacheHit);
                    reporter.report(ProgressEvent::EmbeddingGenerated);
                    reporter.report(ProgressEvent::PhaseCompleted {
                        name: format!("thread-{}", i),
                        duration_secs: 1.0,
                    });
                })
            })
            .collect();

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify stats are consistent
        let stats = reporter.stats();
        assert_eq!(stats.succeeded_items, 50); // 5 threads * 10 items
        assert_eq!(stats.cache_hits, 5); // 5 threads * 1 cache hit
        assert_eq!(stats.embeddings_generated, 5); // 5 threads * 1 embedding
    }
}
