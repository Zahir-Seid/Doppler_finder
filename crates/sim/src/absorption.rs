//! Air absorption along the propagation path (FR-SIM-01, SDD section 7).
//!
//! SDD section 7 asks the synthesis for "a simple absorption term". This is a
//! one-parameter approximation of the ISO 9613-1 pure-tone curve over the
//! 50–1500 Hz band the front end keeps: the coefficient is stated in decibels
//! per kilometre at 1 kHz and scaled by `(f / 1 kHz)^1.5`. That power law is a
//! shape assumption, not a measurement, so the default is zero attenuation and
//! the coefficient stays a configuration value tagged `PROVISIONAL(G2)` until
//! the error budget fixes it (Q-12).

use crate::error::SimError;

/// Frequency exponent of the shape assumption above.
const FREQUENCY_EXPONENT: f64 = 1.5;
/// Reference frequency of the coefficient, hertz.
const REFERENCE_FREQUENCY_HZ: f64 = 1_000.0;
/// Metres per kilometre.
const METRES_PER_KM: f64 = 1_000.0;

/// Attenuation of a tone travelling through air.
///
/// The value is a linear amplitude factor in `(0, 1]`; absorption never
/// amplifies. Zero means the medium is treated as lossless.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AirAbsorption {
    db_per_km_at_1_khz: f64,
}

impl AirAbsorption {
    /// A lossless medium, the default until G2 fixes a coefficient.
    pub const fn none() -> Self {
        Self {
            db_per_km_at_1_khz: 0.0,
        }
    }

    /// Absorption of `db_per_km_at_1_khz` decibels at 1 kHz over one kilometre.
    ///
    /// Returns [`SimError::InvalidAbsorption`] for a negative or non-finite
    /// coefficient: absorption cannot add energy.
    pub fn new(db_per_km_at_1_khz: f64) -> Result<Self, SimError> {
        if !db_per_km_at_1_khz.is_finite() || db_per_km_at_1_khz < 0.0 {
            return Err(SimError::InvalidAbsorption(db_per_km_at_1_khz));
        }
        Ok(Self { db_per_km_at_1_khz })
    }

    /// The coefficient in decibels per kilometre at 1 kHz.
    pub fn db_per_km_at_1_khz(&self) -> f64 {
        self.db_per_km_at_1_khz
    }

    /// Return `true` when the medium is lossless.
    pub fn is_transparent(&self) -> bool {
        self.db_per_km_at_1_khz == 0.0
    }

    /// Linear amplitude factor for a tone at `frequency_hz` over `range_m`.
    ///
    /// Returns [`SimError::NotFinite`] for a non-finite frequency or range,
    /// [`SimError::InvalidPass`] for a non-positive frequency and
    /// [`SimError::InvalidPass`] for a negative range; the index in that error
    /// is not meaningful here, so callers validate geometry first.
    pub fn attenuation(&self, frequency_hz: f64, range_m: f64) -> Result<f64, SimError> {
        if !frequency_hz.is_finite() || !range_m.is_finite() {
            return Err(SimError::NotFinite);
        }
        if frequency_hz <= 0.0 || range_m < 0.0 {
            return Err(SimError::InvalidPass { index: 0 });
        }
        Ok(self.factor(frequency_hz, range_m))
    }

    /// The same factor without validation, for callers that validated their
    /// geometry once.
    pub(crate) fn factor(self, frequency_hz: f64, range_m: f64) -> f64 {
        if self.is_transparent() {
            return 1.0;
        }
        let shape = (frequency_hz / REFERENCE_FREQUENCY_HZ).powf(FREQUENCY_EXPONENT);
        let decibels = self.db_per_km_at_1_khz * shape * (range_m / METRES_PER_KM);
        10.0_f64.powf(-decibels / 20.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lossless_medium_leaves_the_tone_untouched() {
        let absorption = AirAbsorption::none();
        assert!((absorption.attenuation(100.0, 5_000.0).expect("valid") - 1.0).abs() < 1e-15);
        assert!((absorption.attenuation(1_500.0, 20_000.0).expect("valid") - 1.0).abs() < 1e-15);
        assert!(absorption.is_transparent());
    }

    #[test]
    fn six_decibels_at_one_kilometre_halves_the_amplitude() {
        let absorption = AirAbsorption::new(6.0).expect("valid coefficient");
        // 10^(-6/20) = 0.5012; the tolerance is on the closed form, not on air.
        let factor = absorption.attenuation(1_000.0, 1_000.0).expect("valid");
        assert!((factor - 0.501_187).abs() < 1e-6, "got {factor}");
        assert!(!absorption.is_transparent());
    }

    #[test]
    fn attenuation_falls_with_range_and_frequency() {
        let absorption = AirAbsorption::new(6.0).expect("valid");
        let near = absorption.attenuation(1_000.0, 500.0).expect("valid");
        let far = absorption.attenuation(1_000.0, 2_000.0).expect("valid");
        assert!(far < near, "absorbs more over a longer path");

        let low = absorption.attenuation(100.0, 1_000.0).expect("valid");
        let high = absorption.attenuation(1_000.0, 1_000.0).expect("valid");
        // (100/1000)^1.5 = 0.0316, so the low tone loses about 0.19 dB instead
        // of 6 dB.
        assert!(high < low, "absorbs more at a higher frequency");
        assert!(low > 0.97, "a 100 Hz tone barely attenuates, got {low}");
    }

    #[test]
    fn impossible_absorption_is_rejected() {
        assert!(matches!(
            AirAbsorption::new(-1.0),
            Err(SimError::InvalidAbsorption(_))
        ));
        assert!(matches!(
            AirAbsorption::new(f64::NAN),
            Err(SimError::InvalidAbsorption(_))
        ));
        let absorption = AirAbsorption::new(6.0).expect("valid");
        assert!(matches!(
            absorption.attenuation(0.0, 1_000.0),
            Err(SimError::InvalidPass { index: 0 })
        ));
        assert!(matches!(
            absorption.attenuation(100.0, -1.0),
            Err(SimError::InvalidPass { index: 0 })
        ));
        assert!(matches!(
            absorption.attenuation(f64::INFINITY, 1_000.0),
            Err(SimError::NotFinite)
        ));
    }
}
