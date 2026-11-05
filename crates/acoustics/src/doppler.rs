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
    /// `received_offset_s = u = t_r − t0e` and the result `τ = t_e − t0e` in
    /// seconds. Both offsets are measured from the **emitted** closest
    /// approach, which is why the heard closest approach sits at `u = d / c`
    /// rather than at zero; [`StraightPass::received_offset_from_heard_offset`]
    /// converts an offset from the heard time. The root with `t_e <= t_r` is
    /// returned and `|v| < c` is required.
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

    /// Convert an offset from the heard closest approach to the closed form's.
    ///
    /// Fits work in reception time measured from the heard closest approach,
    /// while [`StraightPass::retarded_offset`] measures `u` from the emitted
    /// one: `u = heard_offset + d / c`. Returns the errors of
    /// [`StraightPass::closest_approach_delay_s`].
    pub fn received_offset_from_heard_offset(
        &self,
        heard_offset_s: f64,
        sound_speed_m_per_s: f64,
    ) -> Result<Seconds, AcousticsError> {
        if !heard_offset_s.is_finite() {
            return Err(AcousticsError::NotFinite);
        }
        self.closest_approach_delay_s(sound_speed_m_per_s)
            .map(|delay| Seconds(heard_offset_s + delay.0))
    }

    /// Emission time of closest approach, given the heard one.
    ///
    /// Inverts `t0r = t0e + d / c` from SDD section 4.
    pub fn emitted_closest_approach_time(
        &self,
        heard_closest_approach_s: Seconds,
        sound_speed_m_per_s: f64,
    ) -> Result<Seconds, AcousticsError> {
        if !heard_closest_approach_s.0.is_finite() {
            return Err(AcousticsError::NotFinite);
        }
        self.closest_approach_delay_s(sound_speed_m_per_s)
            .map(|delay| Seconds(heard_closest_approach_s.0 - delay.0))
    }

    /// Observed frequency of one harmonic at a reception-time offset.
    ///
    /// Evaluates `f_h = h f0 c / (c + v_r(t_e))` from SDD section 4: the radial
    /// speed is taken at the emission time that satisfies the light-time
    /// equation, which [`StraightPass::retarded_offset`] solves. `harmonic` is
    /// the integer `h >= 1`; `received_offset_s` is `u = t_r − t0e`, the
    /// convention of [`StraightPass::retarded_offset`].
    ///
    /// Returns [`AcousticsError::ZeroHarmonic`] for `h = 0`, and the errors of
    /// [`StraightPass::retarded_offset`] and
    /// [`observed_frequency_hz`] for impossible geometry.
    pub fn received_frequency(
        &self,
        fundamental: Hertz,
        harmonic: u32,
        received_offset_s: f64,
        sound_speed_m_per_s: f64,
    ) -> Result<Hertz, AcousticsError> {
        if harmonic == 0 {
            return Err(AcousticsError::ZeroHarmonic);
        }
        if !fundamental.0.is_finite() {
            return Err(AcousticsError::NotFinite);
        }
        let retarded = self.retarded_offset(received_offset_s, sound_speed_m_per_s)?;
        let radial = self.radial_speed(retarded)?;
        let emitted = Hertz(fundamental.0 * f64::from(harmonic));
        observed_frequency_hz(emitted, radial, sound_speed_m_per_s)
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

    fn pass() -> StraightPass {
        StraightPass {
            closest_approach_m: 500.0,
            speed_m_per_s: 250.0,
        }
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
        let pass = pass();
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
    fn emitted_closest_approach_precedes_the_heard_one_by_the_delay() {
        let c = sound_speed();
        let pass = pass();
        let heard = Seconds(12.0);
        let emitted = pass
            .emitted_closest_approach_time(heard, c)
            .expect("valid geometry");
        let delay = pass.closest_approach_delay_s(c).expect("valid");
        assert!(emitted.0 < heard.0, "sound takes time to arrive");
        assert_close(emitted.0 + delay.0, heard.0);
    }

    #[test]
    fn radial_speed_is_zero_at_the_emitted_closest_approach() {
        assert!(
            pass()
                .radial_speed(Seconds(0.0))
                .expect("valid geometry")
                .abs()
                < 1e-12
        );
    }

    #[test]
    fn radial_speed_changes_sign_around_the_pass() {
        let pass = pass();
        let before = pass.radial_speed(Seconds(-5.0)).expect("valid geometry");
        let after = pass.radial_speed(Seconds(5.0)).expect("valid geometry");
        assert!(before < 0.0, "approaching before the pass, got {before}");
        assert!(after > 0.0, "receding after the pass, got {after}");
        assert!((before + after).abs() < 1e-9, "the pass is symmetric");
    }

    #[test]
    fn harmonics_are_multiples_of_the_fundamental() {
        let c = sound_speed();
        let fundamental = Hertz(100.0);
        for received_offset_s in [-3.0, 0.0, 3.0] {
            let first = pass()
                .received_frequency(fundamental, 1, received_offset_s, c)
                .expect("valid geometry")
                .0;
            let second = pass()
                .received_frequency(fundamental, 2, received_offset_s, c)
                .expect("valid geometry")
                .0;
            // Scaling by a power of two is exact in binary floating point.
            assert!(
                (second - 2.0 * first).abs() < 1e-12,
                "harmonic 2 must be twice harmonic 1 at u={received_offset_s}"
            );
        }
    }

    #[test]
    fn received_frequency_stays_between_the_asymptotes() {
        // SDD section 4: the asymptotes are f0 c/(c − v) before the pass and
        // f0 c/(c + v) after it, approached from inside.
        let c = sound_speed();
        let f0 = 100.0;
        let approaching = pass()
            .received_frequency(Hertz(f0), 1, -60.0, c)
            .expect("valid geometry")
            .0;
        let receding = pass()
            .received_frequency(Hertz(f0), 1, 60.0, c)
            .expect("valid geometry")
            .0;
        let high_asymptote = f0 * c / (c - 250.0);
        let low_asymptote = f0 * c / (c + 250.0);
        assert!(approaching < high_asymptote);
        assert!(approaching > f0);
        assert!(receding > low_asymptote);
        assert!(receding < f0);
    }

    #[test]
    fn zero_shift_is_heard_one_sound_travel_time_after_emission() {
        // SDD section 4: `v_r = 0` at `τ = 0`, which is heard at `u = d/c`,
        // not at `u = 0`. At `u = 0` the aircraft is still 737 m away and
        // closing at about 184 m/s, so the tone is far above its emitted pitch.
        let c = sound_speed();
        let pass = pass();
        let zero_shift_offset = pass
            .received_offset_from_heard_offset(0.0, c)
            .expect("valid geometry")
            .0;
        let at_zero_shift = pass
            .received_frequency(Hertz(100.0), 1, zero_shift_offset, c)
            .expect("valid geometry")
            .0;
        assert!(
            (at_zero_shift - 100.0).abs() < 0.05,
            "at the emitted closest approach the shift must vanish, got {at_zero_shift}"
        );
        let at_emission_closest = pass
            .received_frequency(Hertz(100.0), 1, 0.0, c)
            .expect("valid geometry")
            .0;
        assert!(
            at_emission_closest > 200.0,
            "still closing hard at u = 0, got {at_emission_closest}"
        );
    }

    #[test]
    fn heard_offset_conversion_shifts_by_the_sound_travel_time() {
        let c = sound_speed();
        let pass = pass();
        let converted = pass
            .received_offset_from_heard_offset(2.0, c)
            .expect("valid geometry")
            .0;
        let delay = pass.closest_approach_delay_s(c).expect("valid").0;
        assert_close(converted, 2.0 + delay);
        assert!(matches!(
            pass.received_offset_from_heard_offset(f64::NAN, c),
            Err(AcousticsError::NotFinite)
        ));
    }

    #[test]
    fn received_curve_is_not_a_delayed_copy_of_the_reception_time_curve() {
        // FR-DM-03: evaluating the radial speed at the reception time instead
        // of the emission time gives a different curve, not a shifted one. One
        // second after the emitted closest approach the two disagree by tens
        // of hertz, far above any fit residual.
        let c = sound_speed();
        let u = 1.0;
        let modelled = pass()
            .received_frequency(Hertz(100.0), 1, u, c)
            .expect("valid geometry")
            .0;
        let reception_time_copy = observed_frequency_hz(
            Hertz(100.0),
            pass().radial_speed(Seconds(u)).expect("valid geometry"),
            c,
        )
        .expect("valid geometry")
        .0;
        assert!(
            (modelled - reception_time_copy).abs() > 1.0,
            "model {modelled} Hz vs delayed copy {reception_time_copy} Hz"
        );
    }

    #[test]
    fn zero_harmonic_is_rejected() {
        assert!(matches!(
            pass().received_frequency(Hertz(100.0), 0, 0.0, sound_speed()),
            Err(AcousticsError::ZeroHarmonic)
        ));
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
