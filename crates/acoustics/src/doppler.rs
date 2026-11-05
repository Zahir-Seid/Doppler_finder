//! Emission-time (retarded) Doppler model (FR-DM-03).
//!
//! SDD section 4 defines the light-time equation and the observed frequency of
//! harmonic `h`. The model is evaluated at emission time, not at reception
//! time; copying the received curve and delaying it is the common mistake the
//! requirement FR-DM-03 forbids.
//!
//! `c` in every formula below is the **speed of sound**, supplied by the
//! [`Atmosphere`](crate::Atmosphere) model (FR-DA-03). Using the speed of light
//! instead shrinks the predicted shift from tens of percent to parts per
//! million while still looking plausible in a plot.

use core_types::{Hertz, Seconds};

use crate::error::AcousticsError;

/// Reject a propagation speed that cannot be a sound speed.
pub(crate) fn validate_sound_speed(sound_speed_m_per_s: f64) -> Result<(), AcousticsError> {
    if !sound_speed_m_per_s.is_finite() {
        return Err(AcousticsError::NotFinite);
    }
    if sound_speed_m_per_s <= 0.0 {
        return Err(AcousticsError::InvalidSoundSpeed(sound_speed_m_per_s));
    }
    Ok(())
}

/// Observed frequency of a source with radial speed `radial_speed_m_per_s`.
///
/// Implements `f = f_emitted c / (c + v_r)` from SDD section 4, with `c` the
/// speed of sound in metres per second. `v_r` is positive when the
/// source-observer range grows, so a receding source is heard lower.
///
/// Returns [`AcousticsError::InvalidSoundSpeed`] for a non-positive or
/// non-finite `c` and [`AcousticsError::SupersonicSource`] when the source
/// leaves no subsonic propagation (`c + v_r <= 0`).
pub fn observed_frequency_hz(
    emitted: Hertz,
    radial_speed_m_per_s: f64,
    sound_speed_m_per_s: f64,
) -> Result<Hertz, AcousticsError> {
    validate_sound_speed(sound_speed_m_per_s)?;
    if !emitted.0.is_finite() || !radial_speed_m_per_s.is_finite() {
        return Err(AcousticsError::NotFinite);
    }
    let denominator = sound_speed_m_per_s + radial_speed_m_per_s;
    if denominator <= 0.0 {
        return Err(AcousticsError::SupersonicSource(radial_speed_m_per_s));
    }
    Ok(Hertz(emitted.0 * sound_speed_m_per_s / denominator))
}

/// A straight, constant-speed pass with no vertical motion.
///
/// This is the geometry that seeds real fits (SDD section 4): `d` the closest
/// distance and `v` the speed. Offsets are emission-time offsets, denoted
/// `τ = t_e − t0e`; reception-time offsets `u = t_r − t0r` enter only through
/// [`StraightPass::retarded_offset`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StraightPass {
    /// Closest approach distance in metres.
    pub closest_approach_m: f64,
    /// Aircraft speed in metres per second.
    pub speed_m_per_s: f64,
}

impl StraightPass {
    /// Check that the geometry admits a retarded solution.
    fn validate(&self, sound_speed_m_per_s: f64) -> Result<(), AcousticsError> {
        validate_sound_speed(sound_speed_m_per_s)?;
        if !self.closest_approach_m.is_finite() || !self.speed_m_per_s.is_finite() {
            return Err(AcousticsError::NotFinite);
        }
        if self.closest_approach_m < 0.0 {
            return Err(AcousticsError::NegativeDistance(self.closest_approach_m));
        }
        // SDD section 4 derives the closed form for a subsonic source; at or
        // above `c` the quadratic's roots lose their physical meaning.
        if self.speed_m_per_s.abs() >= sound_speed_m_per_s {
            return Err(AcousticsError::SupersonicSource(self.speed_m_per_s));
        }
        Ok(())
    }

    /// Convert a reception-time offset to the matching emission-time offset.
    ///
    /// Solves the closed form in SDD section 4 for `τ(u)`, with
    /// `received_offset_s = u = t_r − t0r` and the result `τ = t_e − t0e` in
    /// seconds. The heard and emitted closest-approach times differ by
    /// [`StraightPass::closest_approach_delay_s`]. The root with `t_e <= t_r`
    /// is returned and `|v| < c` is required.
    pub fn retarded_offset(
        &self,
        received_offset_s: f64,
        sound_speed_m_per_s: f64,
    ) -> Result<Seconds, AcousticsError> {
        self.validate(sound_speed_m_per_s)?;
        if !received_offset_s.is_finite() {
            return Err(AcousticsError::NotFinite);
        }

        let c_squared = sound_speed_m_per_s * sound_speed_m_per_s;
        let v_squared = self.speed_m_per_s * self.speed_m_per_s;
        let d_squared = self.closest_approach_m * self.closest_approach_m;
        let discriminant = c_squared - v_squared;
        let radicand = c_squared * v_squared * received_offset_s * received_offset_s
            + d_squared * discriminant;
        if radicand < 0.0 {
            return Err(AcousticsError::NoRealSolution);
        }
        Ok(Seconds(
            (c_squared * received_offset_s - radicand.sqrt()) / discriminant,
        ))
    }

    /// Radial speed in metres per second at an emission-time offset.
    ///
    /// Positive means receding. Implements the second line of the closed form
    /// in SDD section 4, `v_r = v² τ / sqrt(d² + v² τ²)`; no sound speed is
    /// needed because the range derivative is pure geometry.
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
            // Only reachable with d = 0 and τ = 0: the source is exactly at
            // the observer, where the radial direction is undefined.
            return Err(AcousticsError::NoRealSolution);
        }
        Ok(v_squared * retarded_offset_s.0 / path_squared.sqrt())
    }

    /// Delay in seconds between the emitted and the heard closest approach.
    ///
    /// `t0r − t0e = d / c` from SDD section 4. Returns
    /// [`AcousticsError::NegativeDistance`] for a negative `d`.
    pub fn closest_approach_delay_s(
        &self,
        sound_speed_m_per_s: f64,
    ) -> Result<Seconds, AcousticsError> {
        validate_sound_speed(sound_speed_m_per_s)?;
        if !self.closest_approach_m.is_finite() {
            return Err(AcousticsError::NotFinite);
        }
        if self.closest_approach_m < 0.0 {
            return Err(AcousticsError::NegativeDistance(self.closest_approach_m));
        }
        Ok(Seconds(self.closest_approach_m / sound_speed_m_per_s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atmosphere::Atmosphere;

    /// Relative tolerance for frequency ratios: both sides of every frequency
    /// comparison use the same closed form, so only rounding separates them.
    const REL_TOL: f64 = 1e-12;

    /// Absolute tolerance on the light-time residual in seconds. The equation
    /// is exact in exact arithmetic, so 1 µs covers the rounding of a range of
    /// a few kilometres divided by a sound speed of a few hundred m/s.
    const LIGHT_TIME_TOL_S: f64 = 1e-6;

    fn sound_speed() -> f64 {
        Atmosphere::standard().speed_of_sound_m_per_s()
    }

    fn assert_close(actual: f64, expected: f64) {
        let scale = expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= REL_TOL * scale,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn stationary_source_is_heard_at_its_emitted_frequency() {
        let heard = observed_frequency_hz(Hertz(120.0), 0.0, sound_speed()).expect("valid");
        assert_close(heard.0, 120.0);
    }

    #[test]
    fn receding_at_250_m_per_s_shifts_the_tone_by_tens_of_percent() {
        // Regression for the speed-of-light constant: with c = 3e8 the same
        // formula shifts 100 Hz by 8e-5 Hz, which is not the drop a propeller
        // pass produces. A subsonic receding source must lose tens of percent.
        let c = sound_speed();
        let heard = observed_frequency_hz(Hertz(100.0), 250.0, c).expect("valid");
        assert_close(heard.0, 100.0 * c / (c + 250.0));
        assert!(
            (heard.0 - 57.9).abs() < 0.5,
            "250 m/s receding should be heard near 58 Hz, got {heard}"
        );
    }

    #[test]
    fn approaching_at_250_m_per_s_is_heard_higher() {
        let c = sound_speed();
        let heard = observed_frequency_hz(Hertz(100.0), -250.0, c).expect("valid");
        assert_close(heard.0, 100.0 * c / (c - 250.0));
        assert!(heard.0 > 300.0, "approaching tone should rise, got {heard}");
    }

    #[test]
    fn observed_frequency_rejects_invalid_sound_speed() {
        assert!(matches!(
            observed_frequency_hz(Hertz(100.0), 0.0, 0.0),
            Err(AcousticsError::InvalidSoundSpeed(_))
        ));
        assert!(matches!(
            observed_frequency_hz(Hertz(100.0), 0.0, f64::NAN),
            Err(AcousticsError::NotFinite)
        ));
        assert!(matches!(
            observed_frequency_hz(Hertz(100.0), -400.0, 340.0),
            Err(AcousticsError::SupersonicSource(_))
        ));
    }

    #[test]
    fn retarded_offset_satisfies_the_light_time_equation() {
        // SDD section 4: ‖p − x‖ = c (t_r − t_e). This is what "emission time"
        // means and what a delayed copy of the received curve cannot satisfy.
        let c = sound_speed();
        let pass = StraightPass {
            closest_approach_m: 500.0,
            speed_m_per_s: 250.0,
        };
        for received_offset_s in [-10.0, -2.0, -0.1, 0.0, 0.1, 2.0, 10.0] {
            let tau = pass
                .retarded_offset(received_offset_s, c)
                .expect("valid geometry")
                .0;
            let d_squared = pass.closest_approach_m * pass.closest_approach_m;
            let v_squared = pass.speed_m_per_s * pass.speed_m_per_s;
            let range = (d_squared + v_squared * tau * tau).sqrt();
            assert!(
                (range - c * (received_offset_s - tau)).abs() < LIGHT_TIME_TOL_S,
                "u={received_offset_s} tau={tau} residual={}",
                range - c * (received_offset_s - tau)
            );
            assert!(tau <= received_offset_s, "emission must precede reception");
        }
    }

    #[test]
    fn retarded_offset_is_monotone_in_reception_time() {
        let c = sound_speed();
        let pass = StraightPass {
            closest_approach_m: 800.0,
            speed_m_per_s: 200.0,
        };
        let mut previous = f64::NEG_INFINITY;
        for step in -20..=20 {
            let tau = pass
                .retarded_offset(f64::from(step) * 0.5, c)
                .expect("valid geometry")
                .0;
            assert!(tau > previous, "τ must grow with u, broke at step {step}");
            previous = tau;
        }
    }

    #[test]
    fn closest_approach_delay_is_distance_over_sound_speed() {
        let c = sound_speed();
        let pass = StraightPass {
            closest_approach_m: 1_000.0,
            speed_m_per_s: 100.0,
        };
        let delay = pass.closest_approach_delay_s(c).expect("valid").0;
        assert_close(delay, 1_000.0 / c);
    }

    #[test]
    fn radial_speed_is_zero_at_the_emitted_closest_approach() {
        let pass = StraightPass {
            closest_approach_m: 500.0,
            speed_m_per_s: 250.0,
        };
        assert!(
            pass.radial_speed(Seconds(0.0))
                .expect("valid geometry")
                .abs()
                < 1e-12
        );
    }

    #[test]
    fn radial_speed_changes_sign_around_the_pass() {
        let pass = StraightPass {
            closest_approach_m: 500.0,
            speed_m_per_s: 250.0,
        };
        let before = pass.radial_speed(Seconds(-5.0)).expect("valid geometry");
        let after = pass.radial_speed(Seconds(5.0)).expect("valid geometry");
        assert!(before < 0.0, "approaching before the pass, got {before}");
        assert!(after > 0.0, "receding after the pass, got {after}");
        assert!((before + after).abs() < 1e-9, "the pass is symmetric");
    }

    #[test]
    fn straight_pass_rejects_impossible_geometry() {
        let supersonic = StraightPass {
            closest_approach_m: 500.0,
            speed_m_per_s: 400.0,
        };
        assert!(matches!(
            supersonic.retarded_offset(0.0, 340.0),
            Err(AcousticsError::SupersonicSource(_))
        ));
        let negative = StraightPass {
            closest_approach_m: -1.0,
            speed_m_per_s: 100.0,
        };
        assert!(matches!(
            negative.closest_approach_delay_s(340.0),
            Err(AcousticsError::NegativeDistance(_))
        ));
        let finite = StraightPass {
            closest_approach_m: f64::NAN,
            speed_m_per_s: 100.0,
        };
        assert!(matches!(
            finite.retarded_offset(0.0, 340.0),
            Err(AcousticsError::NotFinite)
        ));
    }
}
