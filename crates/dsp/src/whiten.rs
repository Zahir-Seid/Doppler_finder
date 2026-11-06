//! Per-bin noise whitening of the magnitude spectrogram (SDD section 5).
//!
//! Each frequency bin is normalized by its own noise floor over time, so a
//! propeller ridge stands out from coloured, non-stationary background noise.
//! The floor is a running median and the scale a running median absolute
//! deviation (MAD), both taken over a centred window of frames. A second pass
//! can mask detected events before the floor is re-estimated, so a loud pass
//! does not inflate the very scale that should make it visible.

use crate::error::DspError;
use crate::stft::{Spectrogram, StftConfig};

/// Frames in the running noise window.
///
/// PROVISIONAL(G1): no gate has measured this yet. Fifteen frames is roughly
/// 4 s of survey frames at 75% overlap, long enough to include a quiet stretch
/// beside a pass and short enough to follow a slowly changing noise floor.
pub const DEFAULT_WINDOW_FRAMES: usize = 15;

/// Factor that turns a Gaussian MAD into a standard deviation.
///
/// For a normal distribution the MAD is `0.6745 sigma`, so the reciprocal
/// `1.4826` recovers `sigma`.
const MAD_TO_SIGMA: f64 = 1.482_602_218_505_602;

/// Smallest scale used for a bin, as a fraction of its running median.
///
/// A bin that is silent for the whole window has zero MAD, and dividing by it
/// would turn any later sample into an infinite z value. The floor is 60 dB
/// below the bin's median, which still lets a 40 dB onset register.
const SIGMA_FLOOR_FRACTION: f64 = 1.0e-3;

/// Configuration for noise whitening.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WhiteningConfig {
    /// Number of frames in the centred running window, at least one.
    pub window_frames: usize,
}

impl Default for WhiteningConfig {
    fn default() -> Self {
        Self {
            window_frames: DEFAULT_WINDOW_FRAMES,
        }
    }
}

impl WhiteningConfig {
    /// Build a configuration with an explicit window length.
    ///
    /// # Errors
    ///
    /// Returns [`DspError::ZeroLength`] for a zero window.
    pub fn for_window_frames(window_frames: usize) -> Result<Self, DspError> {
        let config = Self { window_frames };
        config.validate()?;
        Ok(config)
    }

    /// Validate the window length.
    ///
    /// # Errors
    ///
    /// Returns [`DspError::ZeroLength`] for a zero window.
    pub fn validate(&self) -> Result<(), DspError> {
        if self.window_frames == 0 {
            return Err(DspError::ZeroLength);
        }
        Ok(())
    }
}

/// A whitened spectrogram: per-bin z values with roughly unit noise scale.
///
/// Values are `(magnitude - median) / sigma`, where the median and sigma come
/// from the surrounding frames of the same bin. Rows are frames, columns bins,
/// matching [`Spectrogram`].
#[derive(Debug, Clone)]
pub struct ZMap {
    /// The geometry of the spectrogram this map came from.
    config: StftConfig,
    /// Window centre time of each frame, in audio seconds.
    frame_times_s: Vec<f64>,
    /// z value per frame and bin, row-major by frame.
    values: Vec<f32>,
}

impl ZMap {
    /// The geometry of the underlying spectrogram.
    pub fn config(&self) -> &StftConfig {
        &self.config
    }

    /// Number of frames.
    pub fn frame_count(&self) -> usize {
        self.frame_times_s.len()
    }

    /// Number of non-negative frequency bins per frame.
    pub fn bin_count(&self) -> usize {
        self.config.bin_count()
    }

    /// Window centre time of each frame, in audio seconds.
    pub fn frame_times_s(&self) -> &[f64] {
        &self.frame_times_s
    }

    /// All z values, row-major by frame.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// z values of one frame, or `None` past the last frame.
    pub fn frame(&self, frame: usize) -> Option<&[f32]> {
        let bins = self.bin_count();
        let start = frame.checked_mul(bins)?;
        let end = start.checked_add(bins)?;
        self.values.get(start..end)
    }

    /// z value at one frame and bin, or `None` outside the map.
    pub fn value(&self, frame: usize, bin: usize) -> Option<f32> {
        let index = frame.checked_mul(self.bin_count())?.checked_add(bin)?;
        self.values.get(index).copied()
    }

    /// Centre frequency of `bin` in hertz, or `None` past the last bin.
    pub fn bin_hz(&self, bin: usize) -> Option<f64> {
        self.config.bin_hz(bin)
    }

    /// Build a map from existing values, checking the shape.
    ///
    /// Used by the whitening itself, by the ridge tracker's tests and by crates
    /// that need a map with exactly known contents, such as the detector's
    /// fixtures.
    ///
    /// # Errors
    ///
    /// Returns [`DspError`] for an invalid geometry, a value count that does
    /// not match the shape, or a non-finite value.
    pub fn from_parts(
        config: StftConfig,
        frame_times_s: Vec<f64>,
        values: Vec<f32>,
    ) -> Result<Self, DspError> {
        config.validate()?;
        let expected = frame_times_s
            .len()
            .checked_mul(config.bin_count())
            .ok_or(DspError::LengthOverflow)?;
        if expected != values.len() {
            return Err(DspError::ShapeMismatch {
                expected,
                actual: values.len(),
            });
        }
        if values.iter().any(|value| !value.is_finite()) {
            return Err(DspError::NotFinite);
        }
        Ok(Self {
            config,
            frame_times_s,
            values,
        })
    }
}

/// Whiten a magnitude spectrogram with no masked cells.
///
/// # Errors
///
/// Returns [`DspError`] when the configuration is invalid or the spectrogram is
/// empty.
pub fn whiten(spectrogram: &Spectrogram, config: &WhiteningConfig) -> Result<ZMap, DspError> {
    whiten_with_mask(spectrogram, config, None)
}

/// Whiten a magnitude spectrogram, excluding masked cells from the noise
/// estimate.
///
/// `masked` is row-major by frame like [`ZMap::values`]; `true` cells are left
/// out of the running median and MAD of their bin, so a detected event does not
/// enlarge the noise scale it is measured against. Masked cells still receive a
/// z value computed from the unmasked neighbours.
///
/// # Errors
///
/// Returns [`DspError`] when the configuration is invalid, the spectrogram is
/// empty, or the mask does not have one entry per frame and bin.
pub fn whiten_with_mask(
    spectrogram: &Spectrogram,
    config: &WhiteningConfig,
    masked: Option<&[bool]>,
) -> Result<ZMap, DspError> {
    config.validate()?;
    let frames = spectrogram.frame_count();
    let bins = spectrogram.bin_count();
    let magnitudes = spectrogram.magnitudes();
    let cells = frames.checked_mul(bins).ok_or(DspError::LengthOverflow)?;
    if frames == 0 || bins == 0 || cells == 0 {
        return Err(DspError::EmptyInput);
    }
    if magnitudes.len() != cells {
        return Err(DspError::ShapeMismatch {
            expected: cells,
            actual: magnitudes.len(),
        });
    }
    if let Some(mask) = masked
        && mask.len() != cells
    {
        return Err(DspError::ShapeMismatch {
            expected: cells,
            actual: mask.len(),
        });
    }

    let rows: Vec<&[f32]> = magnitudes.chunks_exact(bins).collect();
    let mask_rows: Option<Vec<&[bool]>> = masked.map(|mask| mask.chunks_exact(bins).collect());

    let half = config.window_frames / 2;
    let mut column = vec![0.0_f64; config.window_frames];
    let mut deviations = vec![0.0_f64; config.window_frames];
    let mut values = vec![0.0_f32; cells];

    for frame in 0..frames {
        let first = frame.saturating_sub(half);
        let last = frame.saturating_add(half).min(frames - 1);
        let Some(centre_row) = rows.get(frame) else {
            continue;
        };
        for bin in 0..bins {
            let mut count = 0;
            for candidate in first..=last {
                if is_masked(mask_rows.as_deref(), candidate, bin) {
                    continue;
                }
                if let (Some(row), Some(slot)) = (rows.get(candidate), column.get_mut(count))
                    && let Some(&value) = row.get(bin)
                {
                    *slot = f64::from(value);
                    count += 1;
                }
            }
            let Some(&centre) = centre_row.get(bin) else {
                continue;
            };
            let z = whitened_value(&mut column, &mut deviations, count, f64::from(centre));
            if let Some(slot) = values.get_mut(frame * bins + bin) {
                *slot = z;
            }
        }
    }

    ZMap::from_parts(
        *spectrogram.config(),
        spectrogram.frame_times_s().to_vec(),
        values,
    )
}

/// Whether a cell is excluded from the noise estimate.
fn is_masked(mask_rows: Option<&[&[bool]]>, frame: usize, bin: usize) -> bool {
    mask_rows
        .and_then(|rows| rows.get(frame))
        .and_then(|row| row.get(bin))
        .copied()
        .unwrap_or(false)
}

/// Whitened value of one cell given the bin's unmasked samples.
///
/// `column` is the scratch sample buffer; it is reordered by the median but not
/// otherwise consumed. Returns zero for a bin with no usable spread, which is
/// the honest answer for a bin that never varies.
fn whitened_value(column: &mut [f64], deviations: &mut [f64], count: usize, centre: f64) -> f32 {
    let Some(sample) = column.get_mut(..count) else {
        return 0.0;
    };
    let Some(median) = median_in_place(sample) else {
        return 0.0;
    };
    let mut deviation_count = 0;
    for &value in sample.iter() {
        if let Some(slot) = deviations.get_mut(deviation_count) {
            *slot = (value - median).abs();
            deviation_count += 1;
        }
    }
    let Some(deviation_sample) = deviations.get_mut(..deviation_count) else {
        return 0.0;
    };
    let Some(mad) = median_in_place(deviation_sample) else {
        return 0.0;
    };
    let floor = median.abs() * SIGMA_FLOOR_FRACTION;
    let sigma = (mad * MAD_TO_SIGMA).max(floor);
    if !sigma.is_finite() || sigma <= 0.0 {
        return 0.0;
    }
    let z = (centre - median) / sigma;
    if z.is_finite() { z as f32 } else { 0.0 }
}

/// Median of a slice, reordering it in place.
///
/// Uses a selection rather than a full sort, so the cost is linear in the
/// window length. An even-length window averages its two middle values.
fn median_in_place(values: &mut [f64]) -> Option<f64> {
    let len = values.len();
    if len == 0 {
        return None;
    }
    let middle = len / 2;
    let (lower, median_ref, _upper) = values.select_nth_unstable_by(middle, f64::total_cmp);
    let upper = *median_ref;
    if len.is_multiple_of(2) {
        let highest_lower = lower.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Some(f64::midpoint(highest_lower, upper))
    } else {
        Some(upper)
    }
}

#[cfg(test)]
mod tests {
    use super::{WhiteningConfig, whiten, whiten_with_mask, whitened_value};
    use crate::error::DspError;
    use crate::stft::{StftConfig, spectrogram};

    const RATE_HZ: f64 = 4_000.0;

    fn survey_config() -> StftConfig {
        StftConfig::survey(RATE_HZ).expect("survey configuration")
    }

    /// Deterministic white noise in `[-1, 1]`, so tests need no dependency.
    fn white_noise(count: usize, seed: u32) -> Vec<f32> {
        let mut state = seed | 1;
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (f64::from(state) / f64::from(u32::MAX)) * 2.0 - 1.0
            })
            .map(|value| value as f32)
            .collect()
    }

    fn tone(count: usize, frequency_hz: f64, amplitude: f64) -> Vec<f32> {
        (0..count)
            .map(|index| {
                let time_s = index as f64 / RATE_HZ;
                (amplitude * (std::f64::consts::TAU * frequency_hz * time_s).sin()) as f32
            })
            .collect()
    }

    /// Standard deviation of the z values of one bin, skipping the frames whose
    /// centred window is truncated.
    fn bin_scale(map: &super::ZMap, bin: usize, margin: usize) -> f64 {
        let frames = map.frame_count();
        let values: Vec<f64> = (margin..frames.saturating_sub(margin))
            .filter_map(|frame| map.value(frame, bin).map(f64::from))
            .collect();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / values.len() as f64;
        variance.sqrt()
    }

    #[test]
    fn fr_dm_01_white_noise_whitens_to_unit_scale() {
        let config = survey_config();
        let samples = white_noise(48_000, 12_345);
        let map = spectrogram(&samples, &config).expect("spectrogram");
        let zmap = whiten(&map, &WhiteningConfig::default()).expect("whitening");
        assert!(zmap.frame_count() > 20);
        // Pool interior bins and full-window frames: one z value per cell.
        let margin = 8;
        let frames = zmap.frame_count();
        let mut pooled = Vec::new();
        for frame in margin..frames.saturating_sub(margin) {
            for bin in (100..4_000).step_by(10) {
                if let Some(value) = zmap.value(frame, bin) {
                    pooled.push(f64::from(value));
                }
            }
        }
        let mean = pooled.iter().sum::<f64>() / pooled.len() as f64;
        let scale = (pooled
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / pooled.len() as f64)
            .sqrt();
        // Unit scale is the point of whitening; a MAD estimate from 15 frames
        // is itself noisy, so the pooled tolerance is 20%.
        assert!(
            (0.80..1.25).contains(&scale),
            "pooled noise scale {scale} with mean {mean}"
        );
        // No single bin may be wildly mis-scaled.
        for bin in (200..3_800).step_by(200) {
            let bin_scale = bin_scale(&zmap, bin, margin);
            assert!(
                (0.3..3.0).contains(&bin_scale),
                "bin {bin} scale {bin_scale} is far from unit"
            );
        }
    }

    #[test]
    fn fr_dm_01_a_transient_tone_rises_well_above_the_noise() {
        let config = survey_config();
        let sample_count = 48_000;
        let mut samples = white_noise(sample_count, 777);
        let burst_start = 20 * config.hop_len;
        let burst_end = (burst_start + 4 * config.hop_len).min(sample_count);
        let burst = tone(sample_count, 400.0, 0.4);
        for (index, slot) in samples.iter_mut().enumerate() {
            if (burst_start..burst_end).contains(&index)
                && let Some(&value) = burst.get(index)
            {
                *slot += value;
            }
        }
        let map = spectrogram(&samples, &config).expect("spectrogram");
        let zmap = whiten(&map, &WhiteningConfig::default()).expect("whitening");
        let frame = 22;
        let peak = (0..zmap.bin_count())
            .filter_map(|bin| zmap.value(frame, bin))
            .fold(f64::NEG_INFINITY, |best, value| best.max(f64::from(value)));
        // A propeller ridge sits far above the noise, whose z values stay
        // within a few units at this frame.
        assert!(peak > 10.0, "peak z {peak}");
        let peak_bin = (0..zmap.bin_count())
            .max_by(|&left, &right| {
                zmap.value(frame, left)
                    .unwrap_or(0.0)
                    .total_cmp(&zmap.value(frame, right).unwrap_or(0.0))
            })
            .expect("peak bin");
        // 400 Hz is 819.2 bins at 4 kHz with an 8192-point transform. The tone
        // sits 0.2 bin off centre and each bin has its own noise scale, so the
        // whitened argmax may land on either neighbour.
        assert!(
            (818..=820).contains(&peak_bin),
            "peak bin {peak_bin}, expected 819"
        );
    }

    #[test]
    fn fr_dm_01_masking_a_loud_event_keeps_its_z_value_high() {
        let config = survey_config();
        let frames = 24;
        let count = config.frame_len + (frames - 1) * config.hop_len;
        let mut samples = white_noise(count, 4_242);
        let burst_start = 10 * config.hop_len;
        let burst_end = (burst_start + 4 * config.hop_len).min(samples.len());
        let burst = tone(samples.len(), 400.0, 0.5);
        for (index, slot) in samples.iter_mut().enumerate() {
            if (burst_start..burst_end).contains(&index)
                && let Some(&value) = burst.get(index)
            {
                *slot += value;
            }
        }
        let map = spectrogram(&samples, &config).expect("spectrogram");
        let mask_len = map.frame_count() * map.bin_count();
        let mut mask = vec![false; mask_len];
        let bins = map.bin_count();
        for frame in 12..14 {
            for bin in 815..823 {
                if let Some(slot) = mask.get_mut(frame * bins + bin) {
                    *slot = true;
                }
            }
        }
        let plain = whiten(&map, &WhiteningConfig::default()).expect("plain");
        let masked =
            whiten_with_mask(&map, &WhiteningConfig::default(), Some(&mask)).expect("masked");
        let plain_peak = f64::from(plain.value(12, 819).expect("plain peak"));
        let masked_peak = f64::from(masked.value(12, 819).expect("masked peak"));
        assert!(
            masked_peak > plain_peak,
            "masked peak {masked_peak} did not beat plain peak {plain_peak}"
        );
    }

    #[test]
    fn a_flat_bin_whitens_to_zero_instead_of_dividing_by_zero() {
        let config = survey_config();
        let samples = vec![0.0_f32; 16_000];
        let map = spectrogram(&samples, &config).expect("spectrogram");
        let zmap = whiten(&map, &WhiteningConfig::default()).expect("whitening");
        assert!(
            zmap.values().iter().all(|&value| value == 0.0),
            "silence should whiten to zero"
        );
    }

    #[test]
    fn a_bin_with_no_spread_caps_a_constant_offset() {
        let mut column = vec![1.0_f64; 5];
        let mut deviations = vec![0.0_f64; 5];
        // One sample 60 dB above a constant window: the sigma floor bounds it.
        let z = whitened_value(&mut column, &mut deviations, 5, 1_000.0);
        assert!(z.is_finite() && z > 0.0, "z {z}");
        assert!(z <= 1_000.0 / (1.0e-3 * 1.0) + 1.0, "z {z}");
    }

    #[test]
    fn z_map_is_deterministic() {
        let config = survey_config();
        let samples = white_noise(16_000, 99);
        let map = spectrogram(&samples, &config).expect("spectrogram");
        let first = whiten(&map, &WhiteningConfig::default()).expect("first");
        let second = whiten(&map, &WhiteningConfig::default()).expect("second");
        assert_eq!(first.values(), second.values());
        assert_eq!(first.frame_times_s(), second.frame_times_s());
    }

    #[test]
    fn mask_of_the_wrong_length_is_rejected() {
        let config = survey_config();
        let samples = white_noise(16_000, 3);
        let map = spectrogram(&samples, &config).expect("spectrogram");
        let mask = vec![false; 3];
        assert!(matches!(
            whiten_with_mask(&map, &WhiteningConfig::default(), Some(&mask)),
            Err(DspError::ShapeMismatch { .. })
        ));
    }

    #[test]
    fn a_zero_window_is_rejected() {
        assert!(matches!(
            WhiteningConfig::for_window_frames(0),
            Err(DspError::ZeroLength)
        ));
        let config = survey_config();
        let map = spectrogram(&white_noise(8_000, 5), &config).expect("spectrogram");
        let zero = WhiteningConfig { window_frames: 0 };
        assert!(matches!(whiten(&map, &zero), Err(DspError::ZeroLength)));
    }

    #[test]
    fn accessors_stop_at_the_map_edge() {
        let config = survey_config();
        let map = spectrogram(&white_noise(8_000, 11), &config).expect("spectrogram");
        let zmap = whiten(&map, &WhiteningConfig::default()).expect("whitening");
        assert!(zmap.frame(zmap.frame_count()).is_none());
        assert!(
            zmap.value(zmap.frame_count() - 1, zmap.bin_count())
                .is_none()
        );
        assert!(zmap.bin_hz(zmap.bin_count()).is_none());
        assert_eq!(
            zmap.frame(0).map(<[f32]>::len),
            Some(zmap.bin_count()),
            "frame length"
        );
    }
}
