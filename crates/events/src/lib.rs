//! Doppler-event observations, fitted parameters and quality scores.
//!
//! Requirements FR-DM-01 to FR-DM-06. The crate holds the detector, the
//! track-then-fit estimator and the direct parametric estimator, plus the
//! shared event data model and the validation rules they obey.

pub mod detect;
pub mod direct;
pub mod error;
pub mod fit;
pub mod quality;

pub use detect::{Detection, DetectorConfig, EventHypothesis, GridRange, detect};
pub use direct::{DirectConfig, fit_event_direct};
pub use error::EventError;
pub use fit::{
    Event, EventFit, EventObservation, FitConfig, FitMethod, FitReport, HarmonicTrack,
    ProfileInterval, TrackPoint, fit_event, fit_event_from_seeds, multi_start_seeds,
};
pub use quality::EventQuality;
