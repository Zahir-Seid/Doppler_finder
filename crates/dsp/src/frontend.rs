//! Band-limiting, rational resampling and DC removal for the analysis front end.
//!
//! `docs/SDD.md` section 5 fixes the front end: decode to mono, band-limit to
//! roughly 20-1500 Hz, decimate with a polyphase FIR designed by the Kaiser
//! window method (stopband at least 80 dB), and remove DC. This module performs
//! the band-limit, the rational rate change and the DC removal; decoding lives
//! behind the media seam added with `FR-IN-01`.

use std::f64::consts::TAU;

use crate::error::DspError;
use crate::window::bessel_i0;

/// Default output rate of the analysis front end.
pub const DEFAULT_OUTPUT_RATE_HZ: f64 = 4_000.0;

/// Default lower edge of the analysis band.
pub const DEFAULT_LOW_CUT_HZ: f64 = 20.0;

/// Default passband edge of the analysis front end.
pub const DEFAULT_PASSBAND_HZ: f64 = 1_500.0;

/// Default stopband edge of the analysis front end.
pub const DEFAULT_STOPBAND_HZ: f64 = 1_900.0;

/// Default stopband attenuation of the anti-alias filter.
pub const DEFAULT_ATTENUATION_DB: f64 = 80.0;

/// Ratio between the high-pass passband edge and its stopband edge.
///
/// A wider transition keeps the high-pass short enough for long recordings;
/// the anti-alias filter, not this one, carries the 80 dB requirement.
const HIGHPASS_TRANSITION_RATIO: f64 = 4.0;

/// Largest supported FIR length, protecting against pathological settings.
const MAX_FILTER_TAPS: usize = 1_000_000;

/// Largest supported numerator or denominator of the rational rate change.
const MAX_RATIO_PART: usize = 1_000_000;

/// Scale used to turn decimal sample rates into an exact rational ratio.
const RATE_SCALE: f64 = 1_000_000.0;

/// Kaiser's empirical FIR length factor for a given stopband attenuation.
const KAISER_LENGTH_FACTOR: f64 = 2.285;

/// Configuration for the analysis front end (SDD section 5).
///
/// Rates and frequencies are in hertz. The anti-alias filter is designed at the
/// upsampled rate, so a rational rate change such as 44.1 kHz to 4 kHz is
/// handled without first forcing an integer decimation factor. The lower edge
/// is applied after decimation, where a short linear-phase high-pass is enough.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrontEndConfig {
    /// Input sample rate in hertz.
    pub input_rate_hz: f64,
    /// Output sample rate in hertz.
    pub output_rate_hz: f64,
    /// Lower edge of the analysis band in hertz.
    pub low_cut_hz: f64,
    /// Highest frequency retained in the passband, in hertz.
    pub passband_hz: f64,
    /// Frequency at which the stopband begins, in hertz.
    pub stopband_hz: f64,
    /// Minimum attenuation in the stopband, in decibels.
    pub attenuation_db: f64,
}

impl FrontEndConfig {
    /// Build the default front-end configuration for a known input rate.
    ///
    /// The output rate is [`DEFAULT_OUTPUT_RATE_HZ`], the analysis band runs
    /// from [`DEFAULT_LOW_CUT_HZ`] to [`DEFAULT_PASSBAND_HZ`], the stopband
    /// starts at [`DEFAULT_STOPBAND_HZ`], and the stopband attenuation is
    /// [`DEFAULT_ATTENUATION_DB`].
    pub fn for_input_rate(input_rate_hz: f64) -> Self {
        Self {
            input_rate_hz,
            output_rate_hz: DEFAULT_OUTPUT_RATE_HZ,
            low_cut_hz: DEFAULT_LOW_CUT_HZ,
            passband_hz: DEFAULT_PASSBAND_HZ,
            stopband_hz: DEFAULT_STOPBAND_HZ,
            attenuation_db: DEFAULT_ATTENUATION_DB,
        }
    }

    /// Check the configuration before any filter is designed.
    ///
    /// The stopband must sit below the Nyquist frequency of both the input and
    /// the output, otherwise the decimation would alias energy into the
    /// passband.
    pub fn validate(&self) -> Result<(), DspError> {
        let values = [
            self.input_rate_hz,
            self.output_rate_hz,
            self.low_cut_hz,
            self.passband_hz,
            self.stopband_hz,
            self.attenuation_db,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err(DspError::NotFinite);
        }
        if self.input_rate_hz <= 0.0 || self.output_rate_hz <= 0.0 {
            return Err(DspError::InvalidRate {
                input_hz: self.input_rate_hz,
                output_hz: self.output_rate_hz,
            });
        }
        let nyquist_hz = 0.5 * self.input_rate_hz.min(self.output_rate_hz);
        if self.low_cut_hz <= 0.0
            || self.passband_hz <= self.low_cut_hz
            || self.stopband_hz <= self.passband_hz
            || self.stopband_hz >= nyquist_hz
        {
            return Err(DspError::InvalidBand {
                low_cut_hz: self.low_cut_hz,
                passband_hz: self.passband_hz,
                stopband_hz: self.stopband_hz,
                nyquist_hz,
            });
        }
        if self.attenuation_db <= 0.0 {
            return Err(DspError::InvalidAttenuation(self.attenuation_db));
        }
        Ok(())
    }
}

/// Band-limit, resample to the configured analysis rate and remove DC.
///
/// The input is mono PCM. The signal is upsampled by `L`, low-pass filtered by
/// a Kaiser-windowed linear-phase FIR, downsampled by `M`, high-pass filtered
/// at the lower band edge, and then centred by subtracting its mean. `L / M` is
/// the reduced ratio of the requested rates.
///
/// # Errors
///
/// Returns [`DspError`] when the input is empty or non-finite, the rates or band
/// edges are invalid, or the filter design exceeds the implementation limits.
pub fn resample_to_analysis_rate(
    input: &[f32],
    config: &FrontEndConfig,
) -> Result<Vec<f32>, DspError> {
    let mut output = resample(input, config)?;
    remove_dc(&mut output)?;
    Ok(output)
}

/// Remove the arithmetic mean from a sample buffer in place.
///
/// The mean is accumulated in `f64` and subtracted in `f64` before the result is
/// stored back as `f32`. An empty buffer is left unchanged.
///
/// # Errors
///
/// Returns [`DspError::NotFinite`] if any sample is NaN or infinite.
pub fn remove_dc(samples: &mut [f32]) -> Result<(), DspError> {
    if samples.is_empty() {
        return Ok(());
    }
    let mut sum = 0.0_f64;
    for &sample in samples.iter() {
        if !sample.is_finite() {
            return Err(DspError::NotFinite);
        }
        sum += f64::from(sample);
    }
    let mean = sum / samples.len() as f64;
    for sample in samples.iter_mut() {
        *sample = (f64::from(*sample) - mean) as f32;
    }
    Ok(())
}

/// Resample and band-limit without removing DC.
fn resample(input: &[f32], config: &FrontEndConfig) -> Result<Vec<f32>, DspError> {
    config.validate()?;
    if input.is_empty() {
        return Err(DspError::EmptyInput);
    }
    if input.iter().any(|sample| !sample.is_finite()) {
        return Err(DspError::NotFinite);
    }
    let (up, down) = rational_ratio(config.input_rate_hz, config.output_rate_hz)?;
    let taps = design_lowpass(config, up)?;
    let resampled = ResamplingPlan::new(taps, up, down)?.apply(input)?;
    let highpass = design_highpass(config)?;
    apply_highpass(&resampled, &highpass)
}

/// Reduce the rate ratio to the smallest integer numerator and denominator.
fn rational_ratio(input_rate_hz: f64, output_rate_hz: f64) -> Result<(usize, usize), DspError> {
    let input_scaled = (input_rate_hz * RATE_SCALE).round();
    let output_scaled = (output_rate_hz * RATE_SCALE).round();
    let limit = usize::MAX as f64;
    if input_scaled < 1.0 || output_scaled < 1.0 || input_scaled > limit || output_scaled > limit {
        return Err(DspError::UnsupportedRate {
            input_hz: input_rate_hz,
            output_hz: output_rate_hz,
        });
    }
    let input_units = input_scaled as usize;
    let output_units = output_scaled as usize;
    let divisor = greatest_common_divisor(input_units, output_units);
    if divisor == 0 {
        return Err(DspError::UnsupportedRate {
            input_hz: input_rate_hz,
            output_hz: output_rate_hz,
        });
    }
    let up = output_units / divisor;
    let down = input_units / divisor;
    if up == 0 || down == 0 || up > MAX_RATIO_PART || down > MAX_RATIO_PART {
        return Err(DspError::UnsupportedRate {
            input_hz: input_rate_hz,
            output_hz: output_rate_hz,
        });
    }
    Ok((up, down))
}

/// Euclidean greatest common divisor.
fn greatest_common_divisor(mut left: usize, mut right: usize) -> usize {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

/// Design a Kaiser-windowed linear-phase low-pass FIR.
///
/// The filter runs at `input_rate * up` because the signal is upsampled before
/// it is filtered and decimated. The cutoff is the midpoint of the passband and
/// stopband edges, and the length follows Kaiser's empirical length estimate.
///
/// The prototype is normalized to sum to `up`, the interpolation gain. Without
/// that gain, zero insertion would divide a constant input by the upsampling
/// factor.
fn design_lowpass(config: &FrontEndConfig, up: usize) -> Result<Vec<f64>, DspError> {
    let upsampled_rate_hz = config.input_rate_hz * up as f64;
    let transition_hz = config.stopband_hz - config.passband_hz;
    let cutoff_hz = f64::midpoint(config.passband_hz, config.stopband_hz);
    let tap_count = kaiser_tap_count(transition_hz, upsampled_rate_hz, config.attenuation_db)?;
    let mut taps = kaiser_lowpass_taps(
        tap_count,
        cutoff_hz,
        upsampled_rate_hz,
        config.attenuation_db,
    )?;
    normalize_taps(&mut taps, up as f64)?;
    Ok(taps)
}

/// Design the lower band edge as a linear-phase high-pass FIR.
///
/// The prototype is a low-pass whose stopband edge is the high-pass passband
/// divided by [`HIGHPASS_TRANSITION_RATIO`]. Spectral inversion turns it into a
/// high-pass with zero DC gain, so the transition stays short enough for long
/// recordings. The filter runs at the analysis rate, after decimation.
fn design_highpass(config: &FrontEndConfig) -> Result<Vec<f64>, DspError> {
    let passband_hz = config.low_cut_hz;
    let stopband_hz = passband_hz / HIGHPASS_TRANSITION_RATIO;
    let transition_hz = passband_hz - stopband_hz;
    let cutoff_hz = f64::midpoint(stopband_hz, passband_hz);
    let tap_count = kaiser_tap_count(transition_hz, config.output_rate_hz, config.attenuation_db)?;
    let mut taps = kaiser_lowpass_taps(
        tap_count,
        cutoff_hz,
        config.output_rate_hz,
        config.attenuation_db,
    )?;
    normalize_taps(&mut taps, 1.0)?;
    let center = taps.len() / 2;
    for (index, tap) in taps.iter_mut().enumerate() {
        *tap = if index == center { 1.0 - *tap } else { -*tap };
    }
    Ok(taps)
}

/// Kaiser's empirical tap count for a transition width and rate.
///
/// The result is always odd so the linear-phase filter has an integer group
/// delay and one central tap.
fn kaiser_tap_count(
    transition_hz: f64,
    rate_hz: f64,
    attenuation_db: f64,
) -> Result<usize, DspError> {
    let delta_omega = TAU * transition_hz / rate_hz;
    let estimate = ((attenuation_db - 8.0) / (KAISER_LENGTH_FACTOR * delta_omega)).ceil();
    if !estimate.is_finite() || estimate <= 0.0 {
        return Err(DspError::InvalidFilter);
    }
    if estimate > MAX_FILTER_TAPS as f64 {
        return Err(DspError::FilterTooLong {
            taps: estimate as usize,
            max_taps: MAX_FILTER_TAPS,
        });
    }
    let mut tap_count = estimate as usize;
    if tap_count.is_multiple_of(2) {
        tap_count += 1;
    }
    if tap_count > MAX_FILTER_TAPS {
        return Err(DspError::FilterTooLong {
            taps: tap_count,
            max_taps: MAX_FILTER_TAPS,
        });
    }
    Ok(tap_count)
}

/// Build an unnormalized Kaiser-windowed sinc low-pass prototype.
fn kaiser_lowpass_taps(
    tap_count: usize,
    cutoff_hz: f64,
    rate_hz: f64,
    attenuation_db: f64,
) -> Result<Vec<f64>, DspError> {
    let beta = kaiser_beta(attenuation_db);
    let denominator = bessel_i0(beta);
    if !denominator.is_finite() || denominator <= 0.0 {
        return Err(DspError::InvalidFilter);
    }
    let center = (tap_count - 1) as f64 / 2.0;
    let normalized_cutoff = cutoff_hz / rate_hz;
    let mut taps = Vec::with_capacity(tap_count);
    for index in 0..tap_count {
        let offset = index as f64 - center;
        let argument = TAU * normalized_cutoff * offset;
        let sinc = if argument.abs() < f64::EPSILON {
            1.0
        } else {
            argument.sin() / argument
        };
        let ideal = 2.0 * normalized_cutoff * sinc;
        let ratio = offset / center;
        let window = if ratio.abs() >= 1.0 {
            0.0
        } else {
            bessel_i0(beta * (1.0 - ratio * ratio).sqrt()) / denominator
        };
        taps.push(ideal * window);
    }
    Ok(taps)
}

/// Scale a filter so its taps sum to the requested gain.
fn normalize_taps(taps: &mut [f64], target_sum: f64) -> Result<(), DspError> {
    let sum: f64 = taps.iter().sum();
    if !sum.is_finite() || sum.abs() < f64::EPSILON {
        return Err(DspError::InvalidFilter);
    }
    let scale = target_sum / sum;
    for tap in taps.iter_mut() {
        *tap *= scale;
    }
    Ok(())
}

/// Apply a linear-phase FIR and discard its group delay.
///
/// Output sample `n` is aligned with input sample `n`: the filter reads forward
/// by the group delay and backward through the taps, so the band-limit stage
/// does not shift event times. Samples beyond either edge are treated as zero.
fn apply_highpass(samples: &[f32], taps: &[f64]) -> Result<Vec<f32>, DspError> {
    if taps.is_empty() || taps.len().is_multiple_of(2) {
        return Err(DspError::InvalidFilter);
    }
    let delay = taps.len() / 2;
    let last_tap = taps.len() - 1;
    let mut output = Vec::with_capacity(samples.len());
    for index in 0..samples.len() {
        let high = (index + delay).min(last_tap);
        let low = (index + delay + 1).saturating_sub(samples.len());
        if low > high {
            output.push(0.0);
            continue;
        }
        let x_start = index + delay - high;
        let x_end = index + delay - low + 1;
        let Some(tap_slice) = taps.get(low..=high) else {
            output.push(0.0);
            continue;
        };
        let Some(sample_slice) = samples.get(x_start..x_end) else {
            output.push(0.0);
            continue;
        };
        let mut sum = 0.0_f64;
        for (&tap, &sample) in tap_slice.iter().rev().zip(sample_slice.iter()) {
            sum += tap * f64::from(sample);
        }
        output.push(sum as f32);
    }
    Ok(output)
}

/// Kaiser's window-shape parameter for a stopband attenuation in decibels.
fn kaiser_beta(attenuation_db: f64) -> f64 {
    if attenuation_db > 50.0 {
        0.1102 * (attenuation_db - 8.7)
    } else if attenuation_db >= 21.0 {
        0.5842 * (attenuation_db - 21.0).powf(0.4) + 0.07886 * (attenuation_db - 21.0)
    } else {
        0.0
    }
}

/// A polyphase decomposition of the anti-alias filter.
#[derive(Debug, Clone)]
struct ResamplingPlan {
    /// One tap phase per upsampling index, in increasing phase order.
    phases: Vec<Vec<f64>>,
    /// Upsampling factor.
    up: usize,
    /// Downsampling factor.
    down: usize,
    /// Linear-phase group delay in upsampled samples.
    delay: usize,
}

impl ResamplingPlan {
    /// Build the phase bank from a prototype filter.
    fn new(taps: Vec<f64>, up: usize, down: usize) -> Result<Self, DspError> {
        if up == 0 || down == 0 || taps.is_empty() {
            return Err(DspError::InvalidFilter);
        }
        let delay = (taps.len() - 1) / 2;
        let mut phases = vec![Vec::new(); up];
        for (index, &tap) in taps.iter().enumerate() {
            let phase_index = index % up;
            let Some(phase) = phases.get_mut(phase_index) else {
                return Err(DspError::InvalidFilter);
            };
            phase.push(tap);
        }
        Ok(Self {
            phases,
            up,
            down,
            delay,
        })
    }

    /// Apply the phase bank to a mono input.
    fn apply(&self, input: &[f32]) -> Result<Vec<f32>, DspError> {
        let upsampled_len = input
            .len()
            .checked_mul(self.up)
            .ok_or(DspError::LengthOverflow)?;
        let output_len = upsampled_len.div_ceil(self.down);
        let mut output = Vec::with_capacity(output_len);
        for output_index in 0..output_len {
            let raw = output_index
                .checked_mul(self.down)
                .ok_or(DspError::LengthOverflow)?;
            let centered = raw
                .checked_add(self.delay)
                .ok_or(DspError::LengthOverflow)?;
            let input_index = centered / self.up;
            let phase_index = centered % self.up;
            let Some(phase) = self.phases.get(phase_index) else {
                return Err(DspError::InvalidFilter);
            };
            let mut sum = 0.0_f64;
            for (offset, &tap) in phase.iter().enumerate() {
                let Some(index) = input_index.checked_sub(offset) else {
                    continue;
                };
                let Some(&sample) = input.get(index) else {
                    continue;
                };
                sum += tap * f64::from(sample);
            }
            output.push(sum as f32);
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_OUTPUT_RATE_HZ, FrontEndConfig, design_highpass, design_lowpass, rational_ratio,
        remove_dc, resample_to_analysis_rate,
    };
    use crate::error::DspError;
    use crate::window::bessel_i0;

    fn tone(rate_hz: f64, duration_s: f64, frequency_hz: f64, amplitude: f64) -> Vec<f32> {
        let count = (rate_hz * duration_s).round() as usize;
        (0..count)
            .map(|index| {
                let time_s = index as f64 / rate_hz;
                (amplitude * (std::f64::consts::TAU * frequency_hz * time_s).sin()) as f32
            })
            .collect()
    }

    fn middle(samples: &[f32]) -> &[f32] {
        let margin = samples.len() / 10;
        samples
            .get(margin..samples.len().saturating_sub(margin))
            .unwrap_or(samples)
    }

    fn rms(samples: &[f32]) -> f64 {
        let sum: f64 = samples
            .iter()
            .map(|&sample| f64::from(sample).powi(2))
            .sum();
        (sum / samples.len() as f64).sqrt()
    }

    fn peak(samples: &[f32]) -> f64 {
        samples
            .iter()
            .map(|&sample| f64::from(sample).abs())
            .fold(0.0, f64::max)
    }

    #[test]
    fn rational_ratio_reduces_common_audio_rates() {
        assert_eq!(rational_ratio(48_000.0, 4_000.0).expect("48k"), (1, 12));
        assert_eq!(rational_ratio(44_100.0, 4_000.0).expect("44.1k"), (40, 441));
    }

    #[test]
    fn bessel_i0_at_zero_is_one() {
        assert!((bessel_i0(0.0) - 1.0).abs() < 1.0e-15);
    }

    #[test]
    fn designed_filter_has_interpolation_gain() {
        let config = FrontEndConfig::for_input_rate(44_100.0);
        let (up, _down) =
            rational_ratio(config.input_rate_hz, config.output_rate_hz).expect("ratio");
        let taps = design_lowpass(&config, up).expect("filter");
        let sum: f64 = taps.iter().sum();
        assert!((sum - up as f64).abs() < 1.0e-9, "sum {sum}, up {up}");
    }

    #[test]
    fn designed_highpass_has_zero_dc_gain() {
        let config = FrontEndConfig::for_input_rate(48_000.0);
        let taps = design_highpass(&config).expect("highpass");
        let sum: f64 = taps.iter().sum();
        assert!(sum.abs() < 1.0e-9, "sum {sum}");
    }

    #[test]
    fn sub_audio_tone_is_removed_by_the_highpass() {
        let config = FrontEndConfig::for_input_rate(48_000.0);
        let input = tone(48_000.0, 5.0, 2.0, 0.5);
        let output = resample_to_analysis_rate(&input, &config).expect("resampling");
        let residual = peak(middle(&output));
        assert!(residual < 1.0e-3, "residual peak {residual}");
    }

    #[test]
    fn low_cut_above_the_passband_is_rejected() {
        let mut config = FrontEndConfig::for_input_rate(48_000.0);
        config.low_cut_hz = config.passband_hz;
        assert!(matches!(
            config.validate(),
            Err(DspError::InvalidBand { .. })
        ));
    }

    #[test]
    fn passband_tone_survives_48k_resampling() {
        let config = FrontEndConfig::for_input_rate(48_000.0);
        let input = tone(48_000.0, 1.0, 1_000.0, 0.5);
        let output = resample_to_analysis_rate(&input, &config).expect("resampling");
        assert_eq!(output.len(), 4_000);
        let expected_rms = 0.5 / 2.0_f64.sqrt();
        let recovered = rms(middle(&output));
        assert!(
            (recovered - expected_rms).abs() / expected_rms < 0.02,
            "rms {recovered}, expected {expected_rms}"
        );
    }

    #[test]
    fn passband_tone_survives_44k_resampling() {
        let config = FrontEndConfig::for_input_rate(44_100.0);
        let input = tone(44_100.0, 1.0, 1_000.0, 0.5);
        let output = resample_to_analysis_rate(&input, &config).expect("resampling");
        assert_eq!(output.len(), 4_000);
        let expected_rms = 0.5 / 2.0_f64.sqrt();
        let recovered = rms(middle(&output));
        assert!(
            (recovered - expected_rms).abs() / expected_rms < 0.02,
            "rms {recovered}, expected {expected_rms}"
        );
    }

    #[test]
    fn stopband_tone_is_attenuated_by_the_anti_alias_filter() {
        let config = FrontEndConfig::for_input_rate(48_000.0);
        let input = tone(48_000.0, 1.0, 3_000.0, 0.5);
        let output = resample_to_analysis_rate(&input, &config).expect("resampling");
        let residual = peak(middle(&output));
        assert!(residual < 1.0e-4, "residual peak {residual}");
    }

    #[test]
    fn out_of_band_tone_does_not_alias_into_the_passband() {
        let config = FrontEndConfig::for_input_rate(48_000.0);
        let input = tone(48_000.0, 1.0, 7_000.0, 0.5);
        let output = resample_to_analysis_rate(&input, &config).expect("resampling");
        let residual = peak(middle(&output));
        assert!(residual < 1.0e-4, "residual peak {residual}");
    }

    #[test]
    fn dc_removal_zeroes_the_mean() {
        let mut samples = vec![0.25_f32; 100];
        remove_dc(&mut samples).expect("dc removal");
        let mean =
            samples.iter().map(|&sample| f64::from(sample)).sum::<f64>() / samples.len() as f64;
        assert!(mean.abs() < 1.0e-6, "mean {mean}");
    }

    #[test]
    fn resampling_a_constant_leaves_no_dc() {
        let config = FrontEndConfig::for_input_rate(48_000.0);
        let input = vec![0.25_f32; 48_000];
        let output = resample_to_analysis_rate(&input, &config).expect("resampling");
        let mean =
            output.iter().map(|&sample| f64::from(sample)).sum::<f64>() / output.len() as f64;
        assert!(mean.abs() < 1.0e-6, "mean {mean}");
    }

    #[test]
    fn short_input_produces_a_short_output() {
        let config = FrontEndConfig::for_input_rate(48_000.0);
        let input = vec![0.5_f32; 10];
        let output = resample_to_analysis_rate(&input, &config).expect("resampling");
        assert_eq!(output.len(), 1);
    }

    #[test]
    fn resampling_is_deterministic() {
        let config = FrontEndConfig::for_input_rate(48_000.0);
        let input = tone(48_000.0, 0.25, 440.0, 0.5);
        let first = resample_to_analysis_rate(&input, &config).expect("first");
        let second = resample_to_analysis_rate(&input, &config).expect("second");
        assert_eq!(first, second);
    }

    #[test]
    fn empty_input_is_rejected() {
        let config = FrontEndConfig::for_input_rate(48_000.0);
        let result = resample_to_analysis_rate(&[], &config);
        assert!(matches!(result, Err(DspError::EmptyInput)));
    }

    #[test]
    fn non_finite_input_is_rejected() {
        let config = FrontEndConfig::for_input_rate(48_000.0);
        let result = resample_to_analysis_rate(&[0.0, f32::NAN], &config);
        assert!(matches!(result, Err(DspError::NotFinite)));
    }

    #[test]
    fn invalid_band_is_rejected() {
        let mut config = FrontEndConfig::for_input_rate(48_000.0);
        config.stopband_hz = config.passband_hz;
        assert!(matches!(
            config.validate(),
            Err(DspError::InvalidBand { .. })
        ));
    }

    #[test]
    fn band_above_output_nyquist_is_rejected() {
        let mut config = FrontEndConfig::for_input_rate(48_000.0);
        config.stopband_hz = 2_100.0;
        assert!(matches!(
            config.validate(),
            Err(DspError::InvalidBand { .. })
        ));
    }

    #[test]
    fn invalid_rate_is_rejected() {
        let mut config = FrontEndConfig::for_input_rate(48_000.0);
        config.input_rate_hz = 0.0;
        assert!(matches!(
            config.validate(),
            Err(DspError::InvalidRate { .. })
        ));
    }

    #[test]
    fn excessive_filter_length_is_rejected() {
        let mut config = FrontEndConfig::for_input_rate(48_000.0);
        config.attenuation_db = 10_000.0;
        config.stopband_hz = config.passband_hz + 0.000_001;
        let (up, _down) =
            rational_ratio(config.input_rate_hz, config.output_rate_hz).expect("ratio");
        let result = design_lowpass(&config, up);
        assert!(matches!(result, Err(DspError::FilterTooLong { .. })));
    }

    #[test]
    fn default_output_rate_is_expected() {
        assert!((DEFAULT_OUTPUT_RATE_HZ - 4_000.0).abs() < f64::EPSILON);
    }
}
