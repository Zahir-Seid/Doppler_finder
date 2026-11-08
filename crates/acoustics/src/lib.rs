//! Atmosphere and the emission-time Doppler model.
//!
//! The model is specified in `docs/SDD.md` section 4. It depends only on
//! `core-types` for units and atmosphere provenance and on `geodesy` for ECEF
//! positions; it must not depend on DSP or flight data.

pub mod atmosphere;
pub mod doppler;
pub mod error;
pub mod retarded;
pub mod weather;

pub use atmosphere::Atmosphere;
pub use doppler::{StraightPass, observed_frequency_hz};
pub use error::AcousticsError;
pub use retarded::solve_emission_time;
pub use weather::{
    Conditions, WeatherSample, Wind, atmosphere_for_window, conditions_in_window, parse_weather,
};
