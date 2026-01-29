//! Time decay calculator for temporal scoring.

use tracing::trace;

/// Time decay calculator.
pub struct TimeDecay;

impl TimeDecay {
    /// Calculate exponential decay factor based on age.
    ///
    /// # Arguments
    /// * `age_days` - Age in days
    /// * `decay_rate` - Decay rate (default 0.1 for 10% per day)
    ///
    /// # Returns
    /// Decay factor between 0.0 and 1.0
    pub fn calculate(age_days: i64, decay_rate: f32) -> f32 {
        let days = age_days.max(0) as f32;
        let decay = (-decay_rate * days).exp();
        trace!(
            "Time decay: age_days={} days, decay_rate={} -> factor={:.4}",
            age_days, decay_rate, decay
        );
        decay
    }

    /// Calculate combined score from similarity and temporal factors.
    ///
    /// # Deprecated
    ///
    /// This function is deprecated because it only calculates `similarity * temporal_weight`
    /// and does not include `confidence_weight`. Use direct multiplication instead:
    ///
    /// ```rust
    /// // Instead of:
    /// // let score = TimeDecay::combined_score(similarity, temporal_weight);
    ///
    /// // Use the full formula directly:
    /// let final_score = similarity * temporal_weight * confidence_weight;
    /// ```
    #[deprecated(since = "0.1.0", note = "Use direct multiplication with confidence_weight instead: similarity * temporal_weight * confidence_weight")]
    pub fn combined_score(similarity: f32, temporal_weight: f32) -> f32 {
        similarity * temporal_weight
    }
}

impl Default for TimeDecay {
    fn default() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_decay() {
        // Fresh content (0 days) = 1.0
        assert!((TimeDecay::calculate(0, 0.1) - 1.0).abs() < 0.01);

        // 10 days with 0.1 decay rate
        let decay = TimeDecay::calculate(10, 0.1);
        assert!((decay - (-1.0_f32).exp()).abs() < 0.01);
    }

    #[test]
    fn test_combined_score() {
        // Note: This tests the deprecated combined_score function for backward compatibility.
        // The function should still return correct results even though it's deprecated.
        #[allow(deprecated)]
        let score = TimeDecay::combined_score(0.9, 1.2);
        assert!((score - 1.08).abs() < 0.01);

        // Verify the formula matches direct multiplication
        let direct = 0.9 * 1.2;
        assert!((score - direct).abs() < f32::EPSILON);
    }
}
