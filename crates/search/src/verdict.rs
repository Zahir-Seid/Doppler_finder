//! Verdicts and refusal thresholds (FR-SL-02, FR-OUT-01).

use serde::{Deserialize, Serialize};

use crate::error::SearchError;

/// The result of a localization attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Verdict {
    /// The best cluster passes the locatable threshold.
    Locatable,
    /// Evidence supports more than one cluster, or only one weak cluster.
    Ambiguous,
    /// The evidence is insufficient or inconsistent; the honest answer.
    NotLocatable,
}

/// Posterior-mass thresholds for the verdict.
///
/// Intervals are half-open at the top: `Locatable` is
/// `[locatable_min, 1]`, `Ambiguous` is `[ambiguous_min, locatable_min)`, and
/// `NotLocatable` is `[0, ambiguous_min)`. Defaults are provisional until v0
/// calibration (SDD section 6.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerdictThresholds {
    /// Minimum best-cluster mass for `Locatable`.
    pub locatable_min: f64,
    /// Minimum best-cluster mass for `Ambiguous`.
    pub ambiguous_min: f64,
}

impl Default for VerdictThresholds {
    fn default() -> Self {
        Self {
            locatable_min: 0.9,
            ambiguous_min: 0.5,
        }
    }
}

impl VerdictThresholds {
    /// Check that thresholds are finite and ordered.
    pub fn validate(&self) -> Result<(), SearchError> {
        if !self.locatable_min.is_finite() || !self.ambiguous_min.is_finite() {
            return Err(SearchError::NotFinite);
        }
        if !(0.0..=1.0).contains(&self.ambiguous_min)
            || !(0.0..=1.0).contains(&self.locatable_min)
            || self.ambiguous_min > self.locatable_min
        {
            return Err(SearchError::InvalidThresholds);
        }
        Ok(())
    }

    /// Classify a posterior mass using the half-open intervals above.
    ///
    /// The caller must call [`Self::validate`] first.
    pub fn classify(&self, posterior_mass: f64) -> Verdict {
        if posterior_mass >= self.locatable_min {
            Verdict::Locatable
        } else if posterior_mass >= self.ambiguous_min {
            Verdict::Ambiguous
        } else {
            Verdict::NotLocatable
        }
    }
}
