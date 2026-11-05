//! Speed of sound from measured or standard atmosphere (FR-DA-03).

/// Atmospheric conditions at the observer.
///
/// `pressure_pa` and `relative_humidity` are recorded in the run manifest even
/// though the first-order speed-of-sound formula uses only temperature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Atmosphere {
    /// Air temperature in kelvin.
    pub temperature_k: f64,
    /// Static pressure in pascals.
    pub pressure_pa: f64,
    /// Relative humidity in the range `[0, 1]`.
    pub relative_humidity: f64,
}

impl Atmosphere {
    /// The International Standard Atmosphere at sea level.
    pub fn standard() -> Self {
        Self {
            temperature_k: 288.15,
            pressure_pa: 101_325.0,
            relative_humidity: 0.0,
        }
    }

    /// Speed of sound in metres per second.
    ///
    /// Uses the ideal-gas approximation `a = 331.3 sqrt(T / 273.15)`. The
    /// result is valid for the temperature range in which dry air behaves as
    /// an ideal gas; humidity corrections are deferred to the validation study.
    pub fn speed_of_sound_m_per_s(&self) -> f64 {
        331.3 * (self.temperature_k / 273.15).sqrt()
    }
}
