//! Emission-time (retarded) Doppler model.
//!
//! SDD section 4 defines the light-time equation and the observed frequency of
//! harmonic `h`. The model is evaluated at emission time, not at reception
//! time; copying the received curve and delaying it is the common mistake the
//! requirement FR-DM-03 forbids.

use core_types::{Hertz, Seconds};

use crate::error::AcousticsError;

/// Speed of light in vacuum, metres per second.
pub const SPEED_OF_LIGHT_M_PER_S: f64 = 299_792_458.0;

/// Observed frequency of a source moving with radial speed `v_r`.
///
/// `v_r` is positive when the source-observer range is increasing (receding).
/// The returned frequency is `f_emitted c / (c + v_r)`, the non-relativistic
/// limit appropriate to aircraft speeds.
pub fn observed_frequency_hz(emitted_hz: Hertz, radial_speed_m_per_s: f64) -> Hertz {
    Hertz(emitted_hz.0 * SPEED_OF_LIGHT_M_PER_S / (SPEED_OF_LIGHT_M_PER_S + radial_speed_m_per_s))
}

/// A straight, constant-speed pass with no vertical motion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StraightPass {
    /// Closest approach distance in metres.
    pub closest_approach_m: f64,
    /// Aircraft speed in metres per second.
    pub speed_m_per_s: f64,
}

impl StraightPass {
    /// Convert a received-time offset to the corresponding emission-time offset.
    ///
    /// Solves the closed form in SDD section 4. `received_offset_s` is measured
    /// from the heard closest-approach time; the result is measured from the
    /// emitted closest-approach time. Returns [`AcousticsError`] for geometry
    /// with no real solution.
    pub fn retarded_offset(&self, received_offset_s: f64) -> Result<Seconds, AcousticsError> {
        if !received_offset_s.is_finite()
            || !self.closest_approach_m.is_finite()
            || !self.speed_m_per_s.is_finite()
        {
            return Err(AcousticsError::NotFinite);
        }
        if self.closest_approach_m < 0.0 {
            return Err(AcousticsError::NegativeDistance(self.closest_approach_m));
        }
        if self.speed_m_per_s >= SPEED_OF_LIGHT_M_PER_S {
            return Err(AcousticsError::SpeedTooHigh(self.speed_m_per_s));
        }

        let c = SPEED_OF_LIGHT_M_PER_S;
        let c_squared = c * c;
        let v_squared = self.speed_m_per_s * self.speed_m_per_s;
        let denominator = c_squared - v_squared;
        let d_squared = self.closest_approach_m * self.closest_approach_m;
        let radicand =
            c_squared * v_squared * received_offset_s * received_offset_s + d_squared * denominator;
        if radicand < 0.0 {
            return Err(AcousticsError::NoRealSolution);
        }
        Ok(Seconds(
            (c_squared * received_offset_s - radicand.sqrt()) / denominator,
        ))
    }

    /// Radial speed in metres per second at an emission-time offset.
    ///
    /// Positive means receding. Implements the second line of the closed form
    /// in SDD section 4.
    pub fn radial_speed(&self, retarded_offset_s: Seconds) -> Result<f64, AcousticsError> {
        if !retarded_offset_s.0.is_finite()
            || !self.closest_approach_m.is_finite()
            || !self.speed_m_per_s.is_finite()
        {
            return Err(AcousticsError::NotFinite);
        }
        let d_squared = self.closest_approach_m * self.closest_approach_m;
        let v_squared = self.speed_m_per_s * self.speed_m_per_s;
        let path_squared = d_squared + v_squared * retarded_offset_s.0 * retarded_offset_s.0;
        if path_squared <= 0.0 {
            return Err(AcousticsError::NoRealSolution);
        }
        Ok(v_squared * retarded_offset_s.0 / path_squared.sqrt())
    }
}
