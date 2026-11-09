//! SDD 6.3 closing rule and FR-SL-01/SRC-12: every search stage has a work
//! budget; exceeding it keeps the best-scoring prefix and sets a
//! `truncated` flag that caps the verdict at `Ambiguous` (SDD 6.5).
//!
//! Budgets are deliberately generous provisional defaults — they exist to
//! bound worst-case work on pathological inputs, not to trim typical
//! searches. [`WorkBudget::validate`] rejects zero limits; tune the
//! defaults only with simulator evidence (FR-SIM-02).

use crate::error::SearchError;
use crate::posterior::MAX_ASSIGNMENTS;

/// Per-stage work limits for the search pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkBudget {
    /// Maximum hypotheses retained by [`crate::coarse_hypotheses`]; the
    /// lowest-Mahalanobis hypotheses are kept when the gate emits more.
    pub max_hypotheses: usize,
    /// Maximum join candidates retained by [`crate::join_pairs`]; the
    /// smallest `clock_sigma_s` candidates are kept.
    pub max_pairs: usize,
    /// Maximum candidates scored by [`crate::score_candidates`]; the
    /// highest log-posterior candidates are kept.
    pub max_scored_candidates: usize,
    /// Maximum event→track assignments enumerated per candidate; defaults
    /// to [`MAX_ASSIGNMENTS`].
    pub max_assignments: usize,
}

impl Default for WorkBudget {
    fn default() -> Self {
        Self {
            max_hypotheses: 100_000,
            max_pairs: 20_000,
            max_scored_candidates: 10_000,
            max_assignments: MAX_ASSIGNMENTS,
        }
    }
}

impl WorkBudget {
    /// Check that every limit is non-zero.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::InvalidThresholds`] when any limit is zero.
    pub fn validate(&self) -> Result<(), SearchError> {
        if self.max_hypotheses == 0
            || self.max_pairs == 0
            || self.max_scored_candidates == 0
            || self.max_assignments == 0
        {
            return Err(SearchError::InvalidThresholds);
        }
        Ok(())
    }
}
