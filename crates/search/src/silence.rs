//! Silence term of the SDD 6.2 log posterior.
//!
//! Every propeller track in the region that should have been audible inside
//! the recording window but matches no event makes the hypothesis less
//! likely: each unexplained pass contributes `log(1 − P_detect)`. The joint
//! probability of *no* unexplained detections is Poisson-binomial, so the
//! term is the sum of `log(1 − p_i)` over those passes.
//!
//! `P_detect` is a function of slant range at closest approach and engine
//! class. SDD 6.2 says it is measured on the simulator's detection ROC; that
//! measurement has not landed (FR-SIM-02 is still open), so
//! [`DetectionModel::default`] carries provisional logistic parameters
//! tagged `PROVISIONAL(T-4/SRC-04)` until the ROC replaces them.

use adsb::EngineClass;
use serde::{Deserialize, Serialize};

use crate::error::SearchError;

/// One candidate audible pass for the silence term.
///
/// The caller (the search orchestrator) reduces each leg to its closest
/// approach against the hypothesis observer and keeps the passes that could
/// have been heard inside the recording window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SilencePass {
    /// Leg identity, used to decide whether the pass is explained by an
    /// event assignment.
    pub leg_id: String,
    /// Propulsion class from the ICAO type code.
    pub engine_class: EngineClass,
    /// Slant range at closest approach in metres.
    pub slant_range_m: f64,
    /// Heard closest-approach time as Unix seconds.
    pub heard_unix_s: f64,
}

/// Provisional `P_detect(range, engine class)` model.
///
/// `P_detect` for a propeller is logistic in slant range with `half_range_m`
/// the 50% point and `width_m` the transition scale. Jets never produce the
/// propeller fundamental the detector listens for, so their `P_detect` is 0
/// and they are excluded outright. [`EngineClass::Unknown`] is treated
/// probabilistically: the pass is a propeller with prior
/// `unknown_propeller_prior` and silent otherwise, so its effective
/// `P_detect` is the prior times the propeller value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DetectionModel {
    /// Slant range in metres at which `P_detect` is one half for a
    /// propeller.
    pub half_range_m: f64,
    /// Logistic transition width in metres.
    pub width_m: f64,
    /// Prior probability that an [`EngineClass::Unknown`] aircraft is a
    /// propeller.
    pub unknown_propeller_prior: f64,
}

/// PROVISIONAL(T-4/SRC-04): a propeller pass is half-detectable at 10 km
/// slant range with a 3 km transition. These constants are placeholders
/// until the simulator's detection ROC is measured (FR-SIM-02).
pub const PROVISIONAL_HALF_RANGE_M: f64 = 10_000.0;
/// See [`PROVISIONAL_HALF_RANGE_M`].
pub const PROVISIONAL_WIDTH_M: f64 = 3_000.0;
/// See [`PROVISIONAL_HALF_RANGE_M`]: without a type code the aircraft is a
/// propeller half the time.
pub const PROVISIONAL_UNKNOWN_PROP_PRIOR: f64 = 0.5;

impl Default for DetectionModel {
    fn default() -> Self {
        Self {
            half_range_m: PROVISIONAL_HALF_RANGE_M,
            width_m: PROVISIONAL_WIDTH_M,
            unknown_propeller_prior: PROVISIONAL_UNKNOWN_PROP_PRIOR,
        }
    }
}

impl DetectionModel {
    /// `P_detect` for one pass: 0 for jets, logistic in range for
    /// propellers, prior-scaled for unknown classes.
    ///
    /// # Errors
    /// Returns [`SearchError::NotFinite`] when `slant_range_m` is not finite
    /// or the model parameters are invalid.
    pub fn p_detect(&self, pass: &SilencePass) -> Result<f64, SearchError> {
        if !pass.slant_range_m.is_finite() || pass.slant_range_m < 0.0 {
            return Err(SearchError::NotFinite);
        }
        if !self.half_range_m.is_finite()
            || !self.width_m.is_finite()
            || self.width_m <= 0.0
            || !self.unknown_propeller_prior.is_finite()
            || !(0.0..=1.0).contains(&self.unknown_propeller_prior)
        {
            return Err(SearchError::NotFinite);
        }
        let propeller_p =
            1.0 / (1.0 + ((pass.slant_range_m - self.half_range_m) / self.width_m).exp());
        let p = match pass.engine_class {
            EngineClass::Jet => 0.0,
            EngineClass::Propeller => propeller_p,
            EngineClass::Unknown => self.unknown_propeller_prior * propeller_p,
        };
        Ok(p.clamp(0.0, 1.0))
    }
}

/// Log Poisson-binomial probability that none of the unexplained audible
/// passes produced a detected event.
///
/// `passes` are the candidate passes for the hypothesis; `explained`
/// contains the `leg_id`s already matched to events under the current
/// assignment; `[window_start_unix_s, window_end_unix_s)` is the recording
/// window — passes heard outside it cannot have been missed. The result is
/// `sum_i log(1 − p_i)` over the remaining passes, which is `log` of the
/// product form of the Poisson-binomial zero-count probability.
///
/// # Errors
/// Returns [`SearchError::NotFinite`] when any pass or window bound is not
/// finite, or when a [`DetectionModel`] is invalid.
///
/// # Panics
/// Never; non-finite results propagate as `-inf`, which is the honest score
/// for a pass with `P_detect = 1`.
pub fn silence_log_likelihood(
    passes: &[SilencePass],
    explained: &std::collections::BTreeSet<String>,
    window_start_unix_s: f64,
    window_end_unix_s: f64,
    model: &DetectionModel,
) -> Result<f64, SearchError> {
    if !window_start_unix_s.is_finite()
        || !window_end_unix_s.is_finite()
        || window_end_unix_s <= window_start_unix_s
    {
        return Err(SearchError::NotFinite);
    }
    let mut log_l = 0.0;
    for pass in passes {
        if explained.contains(&pass.leg_id) {
            continue;
        }
        if !pass.heard_unix_s.is_finite() {
            return Err(SearchError::NotFinite);
        }
        if !(window_start_unix_s..window_end_unix_s).contains(&pass.heard_unix_s) {
            continue;
        }
        let p = model.p_detect(pass)?;
        // p < 1 strictly: the logistic never reaches 1, and the Unknown
        // prior is clamped to <= 1, so log(1 - p) stays finite unless a
        // degenerate model makes p == 1 exactly.
        log_l += (1.0 - p).ln();
    }
    Ok(log_l)
}
