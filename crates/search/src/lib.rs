//! Localization estimates, posterior summaries and verdicts.
//!
//! Requirements FR-SL-01 to FR-SL-05 and FR-OUT-01. The grid, join, likelihood
//! fusion and refinement stages are added with their simulator tests; this
//! crate fixes the result types and the refusal rule.

pub mod clock;
pub mod coarse;
pub mod error;
pub mod estimate;
pub mod likelihood;
pub mod predict;
pub mod silence;
pub mod verdict;

pub use clock::{
    PROVISIONAL_ADSB_TIMING_SIGMA_S, clock_log_likelihood, clock_terms_log_likelihood,
    implied_recording_start, known_time_log_likelihood, recording_start_sigma,
};
pub use coarse::{CoarseEnv, DEFAULT_GATE_SIGMA, GridSpec, Hypothesis, coarse_hypotheses};
pub use error::SearchError;
pub use estimate::LocalizationEstimate;
pub use likelihood::{ErrorBudget, event_log_likelihood};
pub use predict::{NOMINAL_EAR_HEIGHT_M, Prediction, ellipsoidal_track, predict};
pub use silence::{
    DetectionModel, PROVISIONAL_HALF_RANGE_M, PROVISIONAL_UNKNOWN_PROP_PRIOR, PROVISIONAL_WIDTH_M,
    SilencePass, silence_log_likelihood,
};
pub use verdict::{Verdict, VerdictThresholds};
