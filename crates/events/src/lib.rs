//! Doppler-event observations, fitted parameters and quality scores.
//!
//! Requirements FR-DM-01 to FR-DM-06. The detector, track-then-fit estimator
//! and direct parametric estimator are added in later tasks; this crate fixes
//! the shared data model and the validation rules they must obey.

pub mod detect;
pub mod error;
pub mod fit;
pub mod quality;

pub use detect::{Detection, DetectorConfig, EventHypothesis, GridRange, detect};
pub use error::EventError;
pub use fit::{Event, EventFit, EventObservation};
pub use quality::EventQuality;
