//! Atmosphere and the emission-time Doppler model.
//!
//! The model is specified in `docs/SDD.md` section 4. It is deliberately
//! dependency-free: the physics crate must not depend on DSP or flight data.

pub mod atmosphere;
pub mod doppler;
pub mod error;

pub use atmosphere::Atmosphere;
pub use doppler::{SPEED_OF_LIGHT_M_PER_S, StraightPass, observed_frequency_hz};
pub use error::AcousticsError;
