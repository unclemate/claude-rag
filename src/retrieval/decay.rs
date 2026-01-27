//! Time decay calculator for temporal scoring.

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
        (-decay_rate * days).exp()
    }

    /// Calculate combined score from similarity and temporal factors.
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
        let score = TimeDecay::combined_score(0.9, 1.2);
        assert!((score - 1.08).abs() < 0.01);
    }
}
