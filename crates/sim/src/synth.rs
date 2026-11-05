//! Audio synthesis geometry.

use crate::error::SimError;

/// Sample rate and duration for synthesized audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynthesisConfig {
    /// Output sample rate in hertz.
    pub sample_rate_hz: f64,
    /// Output duration in seconds.
    pub duration_s: f64,
}

impl SynthesisConfig {
    /// Number of samples the configuration produces.
    ///
    /// Returns [`SimError`] for a non-finite or non-positive configuration.
    pub fn sample_count(&self) -> Result<usize, SimError> {
        if !self.sample_rate_hz.is_finite()
            || !self.duration_s.is_finite()
            || self.sample_rate_hz <= 0.0
            || self.duration_s <= 0.0
        {
            return Err(SimError::NotFinite);
        }
        let count = (self.sample_rate_hz * self.duration_s).round();
        if count <= 0.0 || count > usize::MAX as f64 {
            return Err(SimError::NotFinite);
        }
        Ok(count as usize)
    }
}
