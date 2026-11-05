//! Audio synthesis and sample buffers (FR-SIM-01).
//!
//! SDD section 7 makes the simulator the ground truth for the project. This
//! module propagates each pass with the emission-time model of SDD section 4:
//! the frequency heard at a reception time is the emitted harmonic comb shifted
//! by the radial speed at the emission time that satisfies the light-time
//! equation, so a fit can be compared with a known truth rather than with
//! another copy of the same mistake.
//!
//! Absorption of the propagating tone is configurable (see
//! [`AirAbsorption`](crate::AirAbsorption)) and ambient noise is added after
//! every pass is mixed, at the level and colour the configuration states. The
//! optional ground reflection is deferred to v3 as SDD section 7 allows.
//! Recordings are mono and start at the recording zero, which is the audio
//! clock `t_a`.

use acoustics::{Atmosphere, StraightPass, observed_frequency_hz};
use core_types::Hertz;

use crate::absorption::AirAbsorption;
use crate::error::SimError;
use crate::noise::{NoiseConfig, add_noise};
use crate::scenario::{AircraftPass, Scenario};

/// Sample rate, duration and level for synthesized audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynthesisConfig {
    /// Output sample rate in hertz.
    pub sample_rate_hz: f64,
    /// Output duration in seconds.
    pub duration_s: f64,
    /// Peak amplitude of one pass at its closest approach, in `(0, 1]`.
    ///
    /// Spherical spreading only ever reduces a pass below this value, and
    /// several passes add, so the buffer can exceed it when they overlap.
    pub amplitude: f64,
    /// Air absorption over the propagation path; [`AirAbsorption::none`]
    /// leaves the tone untouched.
    pub absorption: AirAbsorption,
    /// Ambient noise to add after mixing, or `None` for clean audio.
    pub noise: Option<NoiseConfig>,
}

impl SynthesisConfig {
    /// Number of samples the configuration produces.
    ///
    /// Returns [`SimError`] for a non-finite or non-positive configuration.
    pub fn sample_count(&self) -> Result<usize, SimError> {
        if !self.sample_rate_hz.is_finite()
            || !self.duration_s.is_finite()
            || self.sample_rate_hz <= 0.0
            || self.duration_s <= 0.0
        {
            return Err(SimError::NotFinite);
        }
        let count = (self.sample_rate_hz * self.duration_s).round();
        if count <= 0.0 || count > usize::MAX as f64 {
            return Err(SimError::NotFinite);
        }
        Ok(count as usize)
    }

    /// Check the whole configuration, not only the frame count.
    fn validate(&self) -> Result<(), SimError> {
        self.sample_count()?;
        if !self.amplitude.is_finite() || self.amplitude <= 0.0 || self.amplitude > 1.0 {
            return Err(SimError::InvalidAmplitude(self.amplitude));
        }
        if let Some(noise) = &self.noise {
            noise.validate()?;
        }
        Ok(())
    }
}

/// A mono PCM buffer on the audio clock.
#[derive(Debug, Clone, PartialEq)]
pub struct Samples {
    sample_rate_hz: f64,
    values: Vec<f32>,
}

impl Samples {
    /// Sample rate in hertz.
    pub fn sample_rate_hz(&self) -> f64 {
        self.sample_rate_hz
    }

    /// The samples themselves, oldest first.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Return `true` when the buffer holds no samples.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Largest absolute sample value.
    pub fn peak_amplitude(&self) -> f32 {
        self.values
            .iter()
            .fold(0.0_f32, |peak, value| peak.max(value.abs()))
    }
}

/// Synthesize a scenario into mono audio (FR-SIM-01).
///
/// Every pass is a straight, level flight summarised by the closed-form
/// geometry of SDD section 4, so `synthesize` is exact for the model the
/// estimator fits. The emitted harmonics of pass `j` are scaled by the relative
/// weights in [`AircraftPass::harmonics`] and by `1 / range` relative to the
/// slant range at closest approach, and their phases are accumulated in
/// reception time, which keeps the tone continuous through the chirp.
///
/// Noise is added last, from [`Scenario::seed`](crate::Scenario), so the
/// pass-to-noise ratio does not depend on how many passes the scenario holds.
///
/// Returns [`SimError`] for an invalid scenario or configuration, for a pass
/// that does not clear the observer's ear height, and for geometry the
/// acoustics model rejects (a source at or above the speed of sound).
pub fn synthesize(
    scenario: &Scenario,
    config: &SynthesisConfig,
    atmosphere: &Atmosphere,
) -> Result<Samples, SimError> {
    scenario.validate()?;
    config.validate()?;

    let sound_speed_m_per_s = atmosphere.speed_of_sound_m_per_s();
    let mut accumulator = vec![0.0_f64; config.sample_count()?];
    for (index, pass) in scenario.passes.iter().enumerate() {
        let voice = Voice::new(
            index,
            pass,
            scenario.observer.ear_height_m,
            sound_speed_m_per_s,
        )?;
        voice.mix_into(
            &mut accumulator,
            config.sample_rate_hz,
            config.amplitude,
            config.absorption,
        )?;
    }
    if let Some(noise) = &config.noise {
        add_noise(
            &mut accumulator,
            noise,
            config.sample_rate_hz,
            scenario.seed,
        )?;
    }

    Ok(Samples {
        sample_rate_hz: config.sample_rate_hz,
        values: accumulator.into_iter().map(|value| value as f32).collect(),
    })
}

/// The tone of one pass: geometry, comb and normalised weights.
struct Voice {
    pass: StraightPass,
    sound_speed_m_per_s: f64,
    closest_range_m: f64,
    emitted_closest_approach_s: f64,
    fundamental_hz: f64,
    weights: Vec<f64>,
}

impl Voice {
    fn new(
        index: usize,
        pass: &AircraftPass,
        ear_height_m: f64,
        sound_speed_m_per_s: f64,
    ) -> Result<Self, SimError> {
        let height_m = pass.altitude_m - ear_height_m;
        if !height_m.is_finite() || height_m <= 0.0 {
            return Err(SimError::PassBelowObserver { index });
        }
        let closest_range_m = pass.closest_approach_m.hypot(height_m);
        let total: f64 = pass.harmonics.iter().sum();
        if !total.is_finite() || total <= 0.0 {
            return Err(SimError::InvalidHarmonics { index });
        }

        Ok(Self {
            pass: StraightPass {
                closest_approach_m: closest_range_m,
                speed_m_per_s: pass.velocity_m_per_s,
            },
            sound_speed_m_per_s,
            closest_range_m,
            emitted_closest_approach_s: pass.offset_s,
            fundamental_hz: pass.fundamental_hz,
            weights: pass.harmonics.iter().map(|weight| weight / total).collect(),
        })
    }

    /// Add this pass to the accumulator, in place, without allocating per
    /// sample.
    fn mix_into(
        &self,
        accumulator: &mut [f64],
        sample_rate_hz: f64,
        amplitude: f64,
        absorption: AirAbsorption,
    ) -> Result<(), SimError> {
        let step_s = 1.0 / sample_rate_hz;
        let mut phases = vec![0.0_f64; self.weights.len()];

        for (index, sample) in accumulator.iter_mut().enumerate() {
            let reception_s = index as f64 * step_s;
            let received_offset_s = reception_s - self.emitted_closest_approach_s;
            let retarded = self
                .pass
                .retarded_offset(received_offset_s, self.sound_speed_m_per_s)?;
            let radial = self.pass.radial_speed(retarded)?;
            let fundamental = observed_frequency_hz(
                Hertz(self.fundamental_hz),
                radial,
                self.sound_speed_m_per_s,
            )?;

            // SDD section 4: ‖p − x‖ = sqrt(d² + v² τ²), so the range shrinks
            // to its minimum at the emitted closest approach.
            let range_m = self
                .closest_range_m
                .hypot(self.pass.speed_m_per_s * retarded.0);
            let gain = (self.closest_range_m / range_m).clamp(0.0, 1.0);

            let mut value = 0.0;
            for (harmonic, (phase, weight)) in
                phases.iter_mut().zip(self.weights.iter()).enumerate()
            {
                let harmonic_number = harmonic as f64 + 1.0;
                let frequency_hz = fundamental.0 * harmonic_number;
                *phase += core::f64::consts::TAU * frequency_hz * step_s;
                value += weight * absorption.factor(frequency_hz, range_m) * phase.sin();
            }
            *sample += amplitude * gain * value;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::Observer;
    use core_types::LatLon;

    /// Relative tolerance on a frequency estimated by counting zero crossings.
    /// A 0.5 s window holds about 50 cycles of a 100 Hz tone, so one miscounted
    /// boundary crossing is 2 %, and the chirp moves the instantaneous
    /// frequency by roughly 1 Hz across the window.
    const CROSSING_REL_TOL: f64 = 0.04;

    /// Absolute tolerance when comparing a summed buffer with the sum of its
    /// parts. Both paths are identical in `f64`, so only the final `f32` cast
    /// differs, which is below 6e-8 for these amplitudes.
    const MIX_TOL: f32 = 1e-6;

    fn pass() -> AircraftPass {
        AircraftPass {
            callsign: "TEST123".to_owned(),
            velocity_m_per_s: 90.0,
            closest_approach_m: 800.0,
            offset_s: 30.0,
            altitude_m: 600.0,
            heading_deg: 90.0,
            fundamental_hz: 100.0,
            harmonics: vec![1.0],
        }
    }

    fn scenario(passes: Vec<AircraftPass>) -> Scenario {
        Scenario {
            name: "test".to_owned(),
            seed: 7,
            duration_s: 60.0,
            observer: Observer {
                position: LatLon::new(52.0, 13.0),
                ear_height_m: 1.7,
            },
            passes,
        }
    }

    fn config() -> SynthesisConfig {
        SynthesisConfig {
            sample_rate_hz: 8_000.0,
            duration_s: 60.0,
            amplitude: 0.5,
            absorption: AirAbsorption::none(),
            noise: None,
        }
    }

    /// Slant range at closest approach for the standard test pass.
    fn closest_range_m() -> f64 {
        800.0_f64.hypot(600.0 - 1.7)
    }

    /// Reception time of the heard closest approach for the standard pass.
    fn heard_at_s() -> f64 {
        let c = Atmosphere::standard().speed_of_sound_m_per_s();
        30.0 + closest_range_m() / c
    }

    fn window(samples: &Samples, start_s: f64, length_s: f64) -> &[f32] {
        let rate = samples.sample_rate_hz();
        let start = ((start_s * rate).round().max(0.0)) as usize;
        let length = (length_s * rate).round() as usize;
        let end = (start + length).min(samples.len());
        &samples.values()[start..end]
    }

    /// Frequency estimated from the number of sign changes in a window.
    ///
    /// Valid only for a single sinusoid that completes many cycles, which is
    /// why the tests pass `harmonics: vec![1.0]`.
    fn crossing_frequency(window: &[f32], sample_rate_hz: f64) -> f64 {
        let crossings = window
            .windows(2)
            .filter(|pair| (pair[0] > 0.0) != (pair[1] > 0.0))
            .count();
        crossings as f64 * sample_rate_hz / (2.0 * window.len() as f64)
    }

    /// Power at one frequency, by the Goertzel recurrence.
    fn goertzel_power(window: &[f32], sample_rate_hz: f64, frequency_hz: f64) -> f64 {
        let omega = core::f64::consts::TAU * frequency_hz / sample_rate_hz;
        let coefficient = 2.0 * omega.cos();
        let mut previous = 0.0;
        let mut before_previous = 0.0;
        for sample in window {
            let current = f64::from(*sample) + coefficient * previous - before_previous;
            before_previous = previous;
            previous = current;
        }
        previous * previous + before_previous * before_previous
            - coefficient * previous * before_previous
    }

    #[test]
    fn tone_crosses_the_emitted_pitch_at_the_heard_closest_approach() {
        // SDD section 4: the radial speed is zero at the emitted closest
        // approach, which is heard d/c later, so the tone passes through the
        // emitted fundamental there and nowhere else.
        let atmosphere = Atmosphere::standard();
        let samples = synthesize(&scenario(vec![pass()]), &config(), &atmosphere).expect("valid");
        let estimate = crossing_frequency(
            window(&samples, heard_at_s() - 0.25, 0.5),
            samples.sample_rate_hz(),
        );
        assert!(
            (estimate - 100.0).abs() <= CROSSING_REL_TOL * 100.0,
            "expected the emitted pitch at the heard closest approach, got {estimate} Hz"
        );
    }

    #[test]
    fn synthesized_tone_is_higher_before_the_pass_than_after() {
        let atmosphere = Atmosphere::standard();
        let c = atmosphere.speed_of_sound_m_per_s();
        let samples = synthesize(&scenario(vec![pass()]), &config(), &atmosphere).expect("valid");
        let rate = samples.sample_rate_hz();
        let approaching = crossing_frequency(window(&samples, 5.0, 0.5), rate);
        let receding = crossing_frequency(window(&samples, 55.0, 0.5), rate);
        // Asymptotes of a 90 m/s pass: 136 Hz closing, 79 Hz receding.
        let high_asymptote = 100.0 * c / (c - 90.0);
        let low_asymptote = 100.0 * c / (c + 90.0);
        assert!(
            approaching > 100.0 && approaching < high_asymptote,
            "closing half should sit between f0 and the high asymptote, got {approaching}"
        );
        assert!(
            receding < 100.0 && receding > low_asymptote,
            "receding half should sit between f0 and the low asymptote, got {receding}"
        );
        assert!(
            approaching / receding > 1.3,
            "a 90 m/s pass must shift the tone by tens of percent, got {approaching} / {receding}"
        );
    }

    #[test]
    fn loudest_window_is_centred_on_the_heard_closest_approach() {
        let atmosphere = Atmosphere::standard();
        let samples = synthesize(&scenario(vec![pass()]), &config(), &atmosphere).expect("valid");
        let mut best = (0.0_f64, 0.0_f64);
        for step in 0..120 {
            let start_s = f64::from(step) * 0.5;
            let squared: f64 = window(&samples, start_s, 0.5)
                .iter()
                .map(|value| f64::from(*value).powi(2))
                .sum();
            if squared > best.0 {
                best = (squared, start_s);
            }
        }
        // Spherical spreading makes the range shortest at the heard closest
        // approach, which sits at the centre of the loudest window.
        let expected_start_s = heard_at_s() - 0.25;
        assert!(
            (best.1 - expected_start_s).abs() < 1.0,
            "loudest window starts at {} s, expected about {expected_start_s} s",
            best.1
        );
    }

    #[test]
    fn harmonic_weights_set_the_spectral_balance() {
        let atmosphere = Atmosphere::standard();
        let mut voice = pass();
        voice.harmonics = vec![1.0, 0.5];
        let samples = synthesize(&scenario(vec![voice]), &config(), &atmosphere).expect("valid");
        let rate = samples.sample_rate_hz();
        let segment = window(&samples, heard_at_s() - 0.25, 0.5);
        let first = goertzel_power(segment, rate, 100.0);
        let second = goertzel_power(segment, rate, 200.0);
        let ratio = first / second;
        // Weights 1 and 0.5 give a power ratio of 4 at the closest approach;
        // 25 % covers the chirp and the leakage of a half-second window.
        assert!(
            (ratio - 4.0).abs() < 0.25 * 4.0,
            "expected a power ratio near 4, got {ratio}"
        );
    }

    #[test]
    fn synthesis_is_deterministic_and_respects_the_bounds() {
        let atmosphere = Atmosphere::standard();
        let scenario = scenario(vec![pass()]);
        let first = synthesize(&scenario, &config(), &atmosphere).expect("valid");
        let second = synthesize(&scenario, &config(), &atmosphere).expect("valid");
        assert_eq!(first.len(), 480_000);
        assert_eq!(
            first, second,
            "a fixed scenario must give byte-identical audio"
        );
        assert!(
            first.peak_amplitude() <= 0.5 + 1e-6,
            "one pass cannot exceed the requested peak, got {}",
            first.peak_amplitude()
        );
        assert!(
            first.peak_amplitude() > 0.4,
            "the pass should be near the requested peak, got {}",
            first.peak_amplitude()
        );
    }

    #[test]
    fn overlapping_passes_add_linearly() {
        let atmosphere = Atmosphere::standard();
        let mut second = pass();
        second.callsign = "TEST456".to_owned();
        second.offset_s = 45.0;
        second.fundamental_hz = 140.0;
        second.closest_approach_m = 1_500.0;

        let first_only =
            synthesize(&scenario(vec![pass()]), &config(), &atmosphere).expect("valid");
        let second_only =
            synthesize(&scenario(vec![second.clone()]), &config(), &atmosphere).expect("valid");
        let both =
            synthesize(&scenario(vec![pass(), second]), &config(), &atmosphere).expect("valid");

        assert_eq!(both.len(), first_only.len());
        for (index, mixed) in both.values().iter().enumerate() {
            let expected = first_only.values()[index] + second_only.values()[index];
            assert!(
                (mixed - expected).abs() < MIX_TOL,
                "sample {index}: {mixed} != {expected}"
            );
        }
    }

    #[test]
    fn synthesis_rejects_impossible_inputs() {
        let atmosphere = Atmosphere::standard();

        let mut low = pass();
        low.altitude_m = 1.0;
        assert!(matches!(
            synthesize(&scenario(vec![low]), &config(), &atmosphere),
            Err(SimError::PassBelowObserver { index: 0 })
        ));

        let mut fast = pass();
        fast.velocity_m_per_s = 400.0;
        assert!(matches!(
            synthesize(&scenario(vec![fast]), &config(), &atmosphere),
            Err(SimError::Acoustics(_))
        ));

        let mut loud = config();
        loud.amplitude = 0.0;
        assert!(matches!(
            synthesize(&scenario(vec![pass()]), &loud, &atmosphere),
            Err(SimError::InvalidAmplitude(_))
        ));

        let mut hiss = config();
        hiss.noise = Some(NoiseConfig::white(f64::INFINITY));
        assert!(matches!(
            synthesize(&scenario(vec![pass()]), &hiss, &atmosphere),
            Err(SimError::InvalidNoise(_))
        ));
    }

    #[test]
    fn absorption_quietens_the_far_part_of_the_pass() {
        // A deliberately large coefficient makes the wiring visible: at 60
        // dB/km and 100 Hz the closest approach loses about 1.9 dB, while the
        // 3 km edges of the pass lose several times that.
        let atmosphere = Atmosphere::standard();
        let clean = synthesize(&scenario(vec![pass()]), &config(), &atmosphere).expect("valid");
        let mut absorbed_config = config();
        absorbed_config.absorption = AirAbsorption::new(60.0).expect("valid coefficient");
        let absorbed =
            synthesize(&scenario(vec![pass()]), &absorbed_config, &atmosphere).expect("valid");

        assert!(
            absorbed.peak_amplitude() < 0.97 * clean.peak_amplitude(),
            "absorption must lower the peak: {} vs {}",
            absorbed.peak_amplitude(),
            clean.peak_amplitude()
        );
        let padding = |samples: &Samples| -> f64 {
            let window = window(samples, 55.0, 0.5);
            (window
                .iter()
                .map(|value| f64::from(*value).powi(2))
                .sum::<f64>()
                / window.len() as f64)
                .sqrt()
        };
        // The receding tail is the longest path, so absorption changes it most.
        assert!(padding(&absorbed) < 0.98 * padding(&clean));
    }

    #[test]
    fn noise_is_added_at_the_requested_level() {
        let atmosphere = Atmosphere::standard();
        let clean = synthesize(&scenario(vec![pass()]), &config(), &atmosphere).expect("valid");
        let mut noisy_config = config();
        noisy_config.noise = Some(NoiseConfig::white(20.0));
        let noisy = synthesize(&scenario(vec![pass()]), &noisy_config, &atmosphere).expect("valid");
        let again = synthesize(&scenario(vec![pass()]), &noisy_config, &atmosphere).expect("valid");
        assert_eq!(noisy, again, "the scenario seed must fix the noise");

        let residuals: Vec<f64> = noisy
            .values()
            .iter()
            .zip(clean.values().iter())
            .map(|(loud, quiet)| f64::from(*loud) - f64::from(*quiet))
            .collect();
        let rms = (residuals.iter().map(|value| value * value).sum::<f64>()
            / residuals.len() as f64)
            .sqrt();
        // The peak of the clean mix is near the requested 0.5 and 20 dB is a
        // factor of ten, so the noise sits near 0.05 RMS.
        assert!(
            (rms - 0.05).abs() < 0.01,
            "expected about 0.05 RMS of noise, got {rms}"
        );
    }
}
