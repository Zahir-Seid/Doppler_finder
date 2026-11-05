//! Forward simulator: scenario model, synthesis and error injection.
//!
//! Requirements FR-SIM-01 to FR-SIM-04. The simulator is the ground truth for
//! the project and must never depend on `events` or `search` (SDD section 3),
//! so a test can never share a bug with the code it tests.
//!
//! SDD section 7 fixes the outputs of one scenario and one seed: a WAV file, a
//! truth file and a degraded ADS-B feed, plus the Cramér–Rao bound that says
//! what any estimator could achieve on that audio.

mod absorption;
mod rng;

pub mod crlb;
pub mod error;
pub mod export;
pub mod inject;
pub mod noise;
pub mod scenario;
pub mod synth;

pub use absorption::AirAbsorption;
pub use crlb::{
    CrlbReport, EventBound, EventModel, EventTrack, ObservationWindow, event_bound, position_bound,
    tone_frequency_sigma_hz,
};
pub use error::SimError;
pub use export::{Truth, TruthEvent, truth, write_truth, write_wav};
pub use inject::{
    ErrorProfile, ErrorSource, Feed, Observable, SourceImpact, TrackPoint, degraded_feed, observe,
    sensitivity, truth_observable,
};
pub use noise::{NoiseColour, NoiseConfig};
pub use scenario::{AircraftPass, Observer, Scenario};
pub use synth::{Samples, SynthesisConfig, synthesize};
