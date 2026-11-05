//! Speed of sound from a measured or standard atmosphere (FR-DA-03).
//!
//! The SDD (section 4) asks for measured temperature and wind, with a standard
//! atmosphere as a flagged fallback. This module owns the first-order
//! temperature relation and records which of the two produced the value, so the
//! run manifest can say which one the result rests on.

use core_types::AtmosphereSource;

use crate::error::AcousticsError;

/// Sea-level temperature of the International Standard Atmosphere, kelvin.
const STANDARD_TEMPERATURE_K: f64 = 288.15;
/// Sea-level pressure of the International Standard Atmosphere, pascals.
const STANDARD_PRESSURE_PA: f64 = 101_325.0;
/// Speed of sound in dry air at 0 °C, metres per second. The ideal-gas
/// relation is `a = a0 sqrt(T / T0)` because `a = sqrt(gamma R T)`.
const DRY_AIR_SPEED_OF_SOUND_AT_ZERO_CELSIUS_M_PER_S: f64 = 331.3;
/// 0 °C in kelvin, the reference temperature of the relation above.
const ZERO_CELSIUS_K: f64 = 273.15;

/// Atmospheric conditions used to propagate sound from aircraft to observer.
///
/// The fields stay private so a value always records where it came from:
/// [`Atmosphere::measured`] validates an observation and tags it
/// [`AtmosphereSource::Measured`], while [`Atmosphere::standard`] tags the
/// fallback [`AtmosphereSource::StandardFallback`] (SDD T-2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Atmosphere {
    temperature_k: f64,
    pressure_pa: f64,
    relative_humidity: f64,
    source: AtmosphereSource,
}

impl Atmosphere {
    /// The International Standard Atmosphere at sea level, 15 °C and 1013.25 hPa.
    pub fn standard() -> Self {
        Self {
            temperature_k: STANDARD_TEMPERATURE_K,
            pressure_pa: STANDARD_PRESSURE_PA,
            relative_humidity: 0.0,
            source: AtmosphereSource::StandardFallback,
        }
    }

    /// Atmospheric conditions measured at the observer.
    ///
    /// Returns [`AcousticsError::NotFinite`] for a NaN or infinite value,
    /// [`AcousticsError::InvalidTemperature`] for a temperature at or below
    /// absolute zero, [`AcousticsError::InvalidPressure`] for a non-positive
    /// pressure and [`AcousticsError::InvalidHumidity`] for a fraction outside
    /// `[0, 1]`.
    pub fn measured(
        temperature_k: f64,
        pressure_pa: f64,
        relative_humidity: f64,
    ) -> Result<Self, AcousticsError> {
        if !temperature_k.is_finite() || !pressure_pa.is_finite() || !relative_humidity.is_finite()
        {
            return Err(AcousticsError::NotFinite);
        }
        if temperature_k <= 0.0 {
            return Err(AcousticsError::InvalidTemperature(temperature_k));
        }
        if pressure_pa <= 0.0 {
            return Err(AcousticsError::InvalidPressure(pressure_pa));
        }
        if !(0.0..=1.0).contains(&relative_humidity) {
            return Err(AcousticsError::InvalidHumidity(relative_humidity));
        }
        Ok(Self {
            temperature_k,
            pressure_pa,
            relative_humidity,
            source: AtmosphereSource::Measured,
        })
    }

    /// Air temperature, kelvin.
    pub fn temperature_k(&self) -> f64 {
        self.temperature_k
    }

    /// Static pressure, pascals.
    pub fn pressure_pa(&self) -> f64 {
        self.pressure_pa
    }

    /// Relative humidity as a fraction in `[0, 1]`.
    pub fn relative_humidity(&self) -> f64 {
        self.relative_humidity
    }

    /// Where these conditions came from, for the run manifest.
    pub fn source(&self) -> AtmosphereSource {
        self.source
    }

    /// Return `true` when the standard atmosphere stood in for a measurement.
    ///
    /// Callers flag this in the run manifest, because the fallback carries the
    /// wide model error SDD T-2 warns about.
    pub fn is_standard_fallback(&self) -> bool {
        self.source == AtmosphereSource::StandardFallback
    }

    /// Speed of sound in metres per second.
    ///
    /// `a = 331.3 sqrt(T / 273.15)` from SDD section 4. Humidity and pressure
    /// do not enter this first-order form; their contribution is under 0.5 %
    /// over the temperature range of interest and is deferred to the validation
    /// study, so a measured atmosphere and the fallback with the same
    /// temperature give the same speed.
    pub fn speed_of_sound_m_per_s(&self) -> f64 {
        DRY_AIR_SPEED_OF_SOUND_AT_ZERO_CELSIUS_M_PER_S
            * (self.temperature_k / ZERO_CELSIUS_K).sqrt()
    }

    /// Speed of propagation along the source-observer path, metres per second.
    ///
    /// `wind_towards_observer_m_per_s` is the wind component along the path,
    /// positive when it blows from the source towards the observer and so
    /// shortens the propagation time (SDD section 4). Returns
    /// [`AcousticsError::NotFinite`] for a non-finite component and
    /// [`AcousticsError::InvalidSoundSpeed`] when the wind would leave no
    /// forward propagation.
    pub fn effective_speed_m_per_s(
        &self,
        wind_towards_observer_m_per_s: f64,
    ) -> Result<f64, AcousticsError> {
        if !wind_towards_observer_m_per_s.is_finite() {
            return Err(AcousticsError::NotFinite);
        }
        let speed = self.speed_of_sound_m_per_s() + wind_towards_observer_m_per_s;
        if speed <= 0.0 {
            return Err(AcousticsError::InvalidSoundSpeed(speed));
        }
        Ok(speed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tolerance on a speed of sound in metres per second. The relation is
    /// compared against published values rounded to 0.1 m/s, and the ideal-gas
    /// approximation itself is good to about 0.1 %.
    const SPEED_TOL_M_PER_S: f64 = 0.3;

    #[test]
    fn standard_atmosphere_is_the_sea_level_isa() {
        let standard = Atmosphere::standard();
        assert!((standard.temperature_k() - 288.15).abs() < 1e-9);
        assert!((standard.pressure_pa() - 101_325.0).abs() < 1e-9);
        assert!((standard.relative_humidity()).abs() < 1e-12);
        assert!(standard.is_standard_fallback());
        assert_eq!(standard.source(), AtmosphereSource::StandardFallback);
        // 15 °C is 340.3 m/s, not the 343.2 m/s of 20 °C: a standard
        // atmosphere is not a warm afternoon.
        assert!(
            (standard.speed_of_sound_m_per_s() - 340.3).abs() < SPEED_TOL_M_PER_S,
            "got {}",
            standard.speed_of_sound_m_per_s()
        );
    }

    #[test]
    fn speed_of_sound_at_twenty_celsius_is_about_343() {
        let measured = Atmosphere::measured(293.15, 101_325.0, 0.4).expect("valid conditions");
        assert!(
            (measured.speed_of_sound_m_per_s() - 343.2).abs() < SPEED_TOL_M_PER_S,
            "got {}",
            measured.speed_of_sound_m_per_s()
        );
    }

    #[test]
    fn speed_of_sound_rises_about_0_6_m_per_s_per_kelvin() {
        let cold = Atmosphere::measured(288.15, 101_325.0, 0.0).expect("valid");
        let warm = Atmosphere::measured(289.15, 101_325.0, 0.0).expect("valid");
        let per_kelvin = warm.speed_of_sound_m_per_s() - cold.speed_of_sound_m_per_s();
        assert!(
            (per_kelvin - 0.59).abs() < 0.02,
            "expected about 0.59 m/s per K, got {per_kelvin}"
        );
    }

    #[test]
    fn measured_atmosphere_keeps_its_provenance() {
        let measured = Atmosphere::measured(300.0, 95_000.0, 0.8).expect("valid");
        assert_eq!(measured.source(), AtmosphereSource::Measured);
        assert!(!measured.is_standard_fallback());
    }

    #[test]
    fn measured_atmosphere_rejects_impossible_values() {
        assert!(matches!(
            Atmosphere::measured(0.0, 101_325.0, 0.0),
            Err(AcousticsError::InvalidTemperature(_))
        ));
        assert!(matches!(
            Atmosphere::measured(288.15, 0.0, 0.0),
            Err(AcousticsError::InvalidPressure(_))
        ));
        assert!(matches!(
            Atmosphere::measured(288.15, 101_325.0, 1.5),
            Err(AcousticsError::InvalidHumidity(_))
        ));
        assert!(matches!(
            Atmosphere::measured(288.15, 101_325.0, -0.1),
            Err(AcousticsError::InvalidHumidity(_))
        ));
        assert!(matches!(
            Atmosphere::measured(f64::NAN, 101_325.0, 0.0),
            Err(AcousticsError::NotFinite)
        ));
    }

    #[test]
    fn wind_shifts_the_effective_speed_along_the_path() {
        let standard = Atmosphere::standard();
        let still = standard.speed_of_sound_m_per_s();
        let tailwind = standard.effective_speed_m_per_s(10.0).expect("valid wind");
        let headwind = standard.effective_speed_m_per_s(-10.0).expect("valid wind");
        // A following wind shortens the propagation time, a head wind lengthens
        // it, and neither changes the sound speed itself.
        assert!((tailwind - (still + 10.0)).abs() < 1e-12);
        assert!((headwind - (still - 10.0)).abs() < 1e-12);
    }

    #[test]
    fn wind_rejects_values_that_leave_no_propagation() {
        let standard = Atmosphere::standard();
        assert!(matches!(
            standard.effective_speed_m_per_s(-1_000.0),
            Err(AcousticsError::InvalidSoundSpeed(_))
        ));
        assert!(matches!(
            standard.effective_speed_m_per_s(f64::INFINITY),
            Err(AcousticsError::NotFinite)
        ));
    }

    #[test]
    fn pressure_and_humidity_do_not_change_the_first_order_speed() {
        // Documents the deferral: the relation is temperature-only until the
        // validation study fixes a humid-air correction.
        let dry = Atmosphere::measured(290.0, 101_325.0, 0.0).expect("valid");
        let humid = Atmosphere::measured(290.0, 90_000.0, 1.0).expect("valid");
        assert!((dry.speed_of_sound_m_per_s() - humid.speed_of_sound_m_per_s()).abs() < 1e-12);
    }
}
