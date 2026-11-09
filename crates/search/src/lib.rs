//! Localization estimates, posterior summaries and verdicts.
//!
//! Requirements FR-SL-01 to FR-SL-05 and FR-OUT-01. The grid, join, likelihood
//! fusion and refinement stages are added with their simulator tests; this
//! crate fixes the result types and the refusal rule.

pub mod error;
pub mod estimate;
pub mod likelihood;
pub mod predict;
pub mod verdict;

pub use error::SearchError;
pub use estimate::LocalizationEstimate;
pub use likelihood::{ErrorBudget, event_log_likelihood};
pub use predict::{NOMINAL_EAR_HEIGHT_M, Prediction, ellipsoidal_track, predict};
pub use verdict::{Verdict, VerdictThresholds};
