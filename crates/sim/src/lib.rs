//! Forward simulator: scenario model, synthesis and error injection.
//!
//! Requirements FR-SIM-01 to FR-SIM-04. The simulator is the ground truth for
//! the project and must never depend on `events` or `search` (SDD section 3),
//! so a test can never share a bug with the code it tests. The scenario format
//! and the emission-time tone synthesis exist; error injection, the Cramér–Rao
//! bound and the Monte Carlo study arrive with their tests.

pub mod crlb;
pub mod error;
pub mod scenario;
pub mod synth;

pub use crlb::CrlbReport;
pub use error::SimError;
pub use scenario::{AircraftPass, Observer, Scenario};
pub use synth::{Samples, SynthesisConfig, synthesize};
