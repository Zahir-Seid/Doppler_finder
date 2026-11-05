//! Forward simulator: scenario model, synthesis and error injection.
//!
//! Requirements FR-SIM-01 to FR-SIM-04. The simulator is the ground truth for
//! the project and must never depend on `events` or `search` (SDD section 3).
//! This scaffold fixes the scenario format; synthesis, error injection, CRLB
//! and Monte Carlo arrive with their tests.

pub mod crlb;
pub mod error;
pub mod scenario;
pub mod synth;

pub use crlb::CrlbReport;
pub use error::SimError;
pub use scenario::{AircraftPass, Observer, Scenario};
pub use synth::SynthesisConfig;
