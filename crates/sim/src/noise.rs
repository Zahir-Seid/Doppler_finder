//! Ambient noise at a target signal-to-noise ratio (FR-SIM-01, SDD section 7).
//!
//! The scenario fixes the level and colour of the noise. Both are defined
//! against the clean audio, so a test can state the SNR it asked for and check
//! it on the output: the noise root-mean-square equals the clean peak divided
//! by `10^(snr_db / 20)`, and the peak of the clean mix is the reference. This
//! is a peak-to-RMS definition, not a band-power one, because it stays
//! meaningful for a single flyover whose level changes through the pass.
//!
//! The noise is generated from the scenario seed and added after every pass is
//! mixed, so a fixed scenario gives byte-identical audio (Norms section 5).

use crate::error::SimError;
use crate::rng::NormalSource;

/// One-pole corner frequencies of the coloured-noise approximation, hertz.
const PINK_CORNERS_HZ: [f64; 3] = [60.0, 150.0, 400.0];

/// Spectral shape of the injected noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseColour {
    /// White noise, flat power per hertz.
    White,
    /// Low-frequency-weighted noise, closer to urban ambient sound.
    ///
    /// SDD section 7 asks for a choice of colour but does not fix a slope.
    /// This is three one-pole low-pass sections an octave and a half apart,
    /// summed and then rescaled to the requested RMS: it tilts energy towards
    /// low frequencies without claiming a calibrated 1/f slope. The exact
    /// spectrum is `PROVISIONAL(G2)` (Q-11).
    Pink,
}

/// Level and colour of the ambient noise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseConfig {
    /// Peak-to-RMS signal-to-noise ratio of the clean audio, decibels.
    pub snr_db: f64,
    /// Spectral shape of the noise.
    pub colour: NoiseColour,
}

impl NoiseConfig {
    /// A white-noise configuration at `snr_db`.
    pub const fn white(snr_db: f64) -> Self {
        Self {
            snr_db,
            colour: NoiseColour::White,
        }
    }

    /// A pink-noise configuration at `snr_db`.
    pub const fn pink(snr_db: f64) -> Self {
        Self {
            snr_db,
            colour: NoiseColour::Pink,
        }
    }

    /// Check the noise level.
    ///
    /// Returns [`SimError::InvalidNoise`] for a non-finite level. Any finite
    /// level is accepted: a very high SNR is nearly silent and a very low one
    /// buries the pass, and both are legitimate scenarios.
    pub fn validate(&self) -> Result<(), SimError> {
        if !self.snr_db.is_finite() {
            return Err(SimError::InvalidNoise(self.snr_db));
        }
        Ok(())
    }
}

/// Add seeded noise to a mixed buffer.
///
/// The reference is the largest absolute value already in `signal`; silence
/// stays silent, because there is no level to define an SNR against. Returns
/// [`SimError::InvalidNoise`] for a non-finite level.
pub(crate) fn add_noise(
    signal: &mut [f64],
    config: &NoiseConfig,
    sample_rate_hz: f64,
    seed: u64,
) -> Result<(), SimError> {
    config.validate()?;
    if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
        return Err(SimError::NotFinite);
    }
    let peak = signal
        .iter()
        .fold(0.0_f64, |current, value| current.max(value.abs()));
    if peak <= 0.0 {
        return Ok(());
    }

    let mut source = NormalSource::seeded(seed);
    let mut raw: Vec<f64> = (0..signal.len())
        .map(|_| source.standard_normal())
        .collect();
    if config.colour == NoiseColour::Pink {
        raw = pink(&raw, sample_rate_hz);
    }

    let mean = raw.iter().sum::<f64>() / raw.len() as f64;
    let variance = raw
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / raw.len() as f64;
    if !variance.is_finite() || variance <= 0.0 {
        return Err(SimError::InvalidNoise(config.snr_db));
    }
    let scale = peak / 10.0_f64.powf(config.snr_db / 20.0) / variance.sqrt();
    for (sample, noise) in signal.iter_mut().zip(raw.iter()) {
        *sample += (noise - mean) * scale;
    }
    Ok(())
}

/// Tilt white noise towards low frequencies with three one-pole sections.
fn pink(white: &[f64], sample_rate_hz: f64) -> Vec<f64> {
    let mut coloured = vec![0.0_f64; white.len()];
    for corner in PINK_CORNERS_HZ {
        let coefficient = 1.0 - (-core::f64::consts::TAU * corner / sample_rate_hz).exp();
        let mut state = 0.0;
        for (out, sample) in coloured.iter_mut().zip(white.iter()) {
            state += coefficient * (*sample - state);
            *out += state;
        }
    }
    coloured
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 100 Hz tone at 0.5 peak, the level the synthesis tests use.
    fn tone(sample_rate_hz: f64, seconds: f64) -> Vec<f64> {
        let count = (sample_rate_hz * seconds).round() as usize;
        (0..count)
            .map(|index| {
                0.5 * (core::f64::consts::TAU * 100.0 * index as f64 / sample_rate_hz).sin()
            })
            .collect()
    }

    fn rms(values: &[f64]) -> f64 {
        (values.iter().map(|value| value * value).sum::<f64>() / values.len() as f64).sqrt()
    }

    #[test]
    fn white_noise_hits_the_requested_snr() {
        let clean = tone(8_000.0, 10.0);
        let mut noisy = clean.clone();
        add_noise(&mut noisy, &NoiseConfig::white(20.0), 8_000.0, 5).expect("valid");
        let residuals: Vec<f64> = noisy
            .iter()
            .zip(clean.iter())
            .map(|(loud, quiet)| loud - quiet)
            .collect();
        // The reference is the clean peak of 0.5, so 20 dB means 0.05 RMS.
        // A ten-second buffer holds 80 000 residuals, putting the standard
        // error of their RMS below 0.5 %, so 5 % is a loose sanity bound.
        let measured = rms(&residuals);
        assert!(
            (measured - 0.05).abs() < 0.05 * 0.05,
            "expected 0.05 RMS, got {measured}"
        );
    }

    #[test]
    fn seeded_noise_is_reproducible() {
        let clean = tone(8_000.0, 2.0);
        let mut first = clean.clone();
        let mut second = clean.clone();
        let mut third = clean.clone();
        add_noise(&mut first, &NoiseConfig::pink(10.0), 8_000.0, 9).expect("valid");
        add_noise(&mut second, &NoiseConfig::pink(10.0), 8_000.0, 9).expect("valid");
        add_noise(&mut third, &NoiseConfig::pink(10.0), 8_000.0, 10).expect("valid");
        assert_eq!(first, second, "a fixed seed must repeat the noise");
        assert_ne!(first, third, "a different seed must change the noise");
    }

    #[test]
    fn pink_noise_is_smoother_than_white() {
        let clean = tone(8_000.0, 10.0);
        let mut white = clean.clone();
        let mut pink = clean.clone();
        add_noise(&mut white, &NoiseConfig::white(10.0), 8_000.0, 21).expect("valid");
        add_noise(&mut pink, &NoiseConfig::pink(10.0), 8_000.0, 21).expect("valid");

        // Both buffers have the same RMS by construction, so the ratio of the
        // first-difference variance to the variance isolates spectral tilt.
        let roughness = |values: &[f64]| {
            let differences: Vec<f64> = values.windows(2).map(|pair| pair[1] - pair[0]).collect();
            rms(&differences) / rms(values)
        };
        let white_roughness = roughness(&white);
        let pink_roughness = roughness(&pink);
        assert!(
            pink_roughness < 0.5 * white_roughness,
            "pink noise should be smoother: {pink_roughness} vs {white_roughness}"
        );
    }

    #[test]
    fn silence_stays_silent() {
        let mut silence = vec![0.0_f64; 1_000];
        add_noise(&mut silence, &NoiseConfig::white(0.0), 8_000.0, 1).expect("valid");
        assert!(silence.iter().all(|value| *value == 0.0));
    }

    #[test]
    fn non_finite_noise_level_is_rejected() {
        let mut buffer = tone(8_000.0, 0.1);
        assert!(matches!(
            add_noise(&mut buffer, &NoiseConfig::white(f64::NAN), 8_000.0, 1),
            Err(SimError::InvalidNoise(_))
        ));
        assert!(matches!(
            add_noise(&mut buffer, &NoiseConfig::white(10.0), 0.0, 1),
            Err(SimError::NotFinite)
        ));
    }
}
