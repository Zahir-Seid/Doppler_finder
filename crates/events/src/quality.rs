//! Per-event quality score (FR-DM-06).

use crate::error::EventError;

/// A bounded quality score.
///
/// The score combines ridge SNR, harmonic count, residual autocorrelation and
/// the covariance inflation factor. The exact weights are fixed during v0
/// calibration; this type only enforces the range.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct EventQuality {
    /// Score in `[0, 1]`, where larger is better.
    pub score: f64,
}

impl EventQuality {
    /// Build a score, rejecting non-finite input.
    pub fn from_score(score: f64) -> Result<Self, EventError> {
        if !score.is_finite() {
            return Err(EventError::NotFinite);
        }
        Ok(Self {
            score: score.clamp(0.0, 1.0),
        })
    }

    /// Return whether the event survives a configured cut.
    pub fn is_retained(&self, cut: f64) -> bool {
        self.score >= cut
    }
}
