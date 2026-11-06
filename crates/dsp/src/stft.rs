//! Short-time Fourier transform, zero-padding and sub-bin peak frequency.
//!
//! `docs/SDD.md` section 5 fixes the analysis transform: a Kaiser-windowed STFT
//! with 75% overlap, zero-padding and parabolic peak interpolation for sub-bin
//! frequency. The transform is synchronous and reuses one FFT plan and its
//! scratch buffers across frames, so the frame loop allocates nothing. Noise
//! whitening and ridge tracking are later stages (DSP-04, DSP-05).

use realfft::RealFftPlanner;

use crate::error::DspError;
use crate::window::{DEFAULT_KAISER_BETA, WindowKind, window};

/// Frame length of the survey transform, in seconds.
///
/// SDD section 5 asks for "a fixed window near 1 s" while surveying, which
/// keeps a propeller curve inside a few frames at typical chirp rates.
pub const SURVEY_WINDOW_S: f64 = 1.0;

/// Overlap between successive frames, as a fraction of the frame length.
pub const DEFAULT_OVERLAP_FRACTION: f64 = 0.75;

/// Frame geometry and windowing for a short-time Fourier transform.
///
/// All times are audio seconds from the start of the clip and all rates are in
/// hertz. The transform length `fft_len` is at least `frame_len`: the frame is
/// zero-padded to it, which carries every sample of the window and refines the
/// bin spacing without narrowing the main lobe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StftConfig {
    /// Sample rate of the (possibly decimated) input in hertz.
    pub sample_rate_hz: f64,
    /// Samples per analysis frame.
    pub frame_len: usize,
    /// Samples between successive frame starts.
    pub hop_len: usize,
    /// Window applied to each frame.
    pub window: WindowKind,
    /// Transform length in samples; `frame_len..` is zero-padded.
    pub fft_len: usize,
}

impl StftConfig {
    /// Build a configuration with the default Kaiser window and 2x zero-padding.
    ///
    /// The transform length is the next power of two at or above twice the
    /// frame length, so a real FFT can run and a peak between two bins can be
    /// interpolated without a large bias.
    ///
    /// # Errors
    ///
    /// Returns [`DspError`] when the resulting geometry is invalid.
    pub fn new(sample_rate_hz: f64, frame_len: usize, hop_len: usize) -> Result<Self, DspError> {
        let config = Self {
            sample_rate_hz,
            frame_len,
            hop_len,
            window: WindowKind::Kaiser {
                beta: DEFAULT_KAISER_BETA,
            },
            fft_len: padded_fft_len(frame_len)?,
        };
        config.validate()?;
        Ok(config)
    }

    /// Build the survey configuration for a sample rate.
    ///
    /// Uses a [`SURVEY_WINDOW_S`] frame and [`DEFAULT_OVERLAP_FRACTION`]
    /// overlap: the hop is the frame length divided by four, rounded down and
    /// never zero.
    ///
    /// # Errors
    ///
    /// Returns [`DspError`] when the sample rate cannot produce a usable frame.
    pub fn survey(sample_rate_hz: f64) -> Result<Self, DspError> {
        let frame_len = (SURVEY_WINDOW_S * sample_rate_hz).round();
        if !frame_len.is_finite() || frame_len < 1.0 {
            return Err(DspError::NotFinite);
        }
        let frame_len = frame_len as usize;
        let hop_len = (frame_len / 4).max(1);
        Self::new(sample_rate_hz, frame_len, hop_len)
    }

    /// Validate the geometry and the window.
    ///
    /// Returns [`DspError`] for a zero frame length, a hop outside
    /// `1..=frame_len`, a non-finite sample rate, an FFT length shorter than the
    /// frame or not even, and an unusable Kaiser parameter.
    pub fn validate(&self) -> Result<(), DspError> {
        if self.frame_len == 0 {
            return Err(DspError::ZeroLength);
        }
        if self.hop_len == 0 || self.hop_len > self.frame_len {
            return Err(DspError::InvalidHop {
                hop: self.hop_len,
                frame: self.frame_len,
            });
        }
        if !self.sample_rate_hz.is_finite() || self.sample_rate_hz <= 0.0 {
            return Err(DspError::NotFinite);
        }
        if self.fft_len < self.frame_len || self.fft_len < 2 || !self.fft_len.is_multiple_of(2) {
            return Err(DspError::InvalidFftLength {
                fft_len: self.fft_len,
                frame: self.frame_len,
            });
        }
        if let WindowKind::Kaiser { beta } = self.window
            && (!beta.is_finite() || beta < 0.0)
        {
            return Err(DspError::InvalidWindow { beta });
        }
        Ok(())
    }

    /// Number of complete frames in an input of `sample_count` samples.
    ///
    /// Returns [`DspError::InputTooShort`] when the input cannot hold one
    /// complete frame.
    pub fn frame_count(&self, sample_count: usize) -> Result<usize, DspError> {
        self.validate()?;
        if sample_count < self.frame_len {
            return Err(DspError::InputTooShort {
                samples: sample_count,
                frame: self.frame_len,
            });
        }
        Ok((sample_count - self.frame_len) / self.hop_len + 1)
    }

    /// Number of non-negative frequency bins, `fft_len / 2 + 1`.
    pub fn bin_count(&self) -> usize {
        self.fft_len / 2 + 1
    }

    /// Bin spacing in hertz, `sample_rate / fft_len`.
    pub fn bin_width_hz(&self) -> f64 {
        self.sample_rate_hz / self.fft_len as f64
    }

    /// Centre frequency of `bin` in hertz, or `None` past the last bin.
    pub fn bin_hz(&self, bin: usize) -> Option<f64> {
        if bin >= self.bin_count() {
            return None;
        }
        Some(bin as f64 * self.bin_width_hz())
    }

    /// Audio time of the centre of the frame that starts at `start`.
    pub fn frame_time_s(&self, start: usize) -> f64 {
        (start as f64 + (self.frame_len as f64 - 1.0) / 2.0) / self.sample_rate_hz
    }
}

/// Largest transform length derived from a frame length by [`padded_fft_len`].
///
/// Next power of two of twice the frame length; a frame longer than half this
/// would overflow the allocation and is rejected instead.
fn padded_fft_len(frame_len: usize) -> Result<usize, DspError> {
    if frame_len == 0 {
        return Err(DspError::ZeroLength);
    }
    let doubled = frame_len.checked_mul(2).ok_or(DspError::LengthOverflow)?;
    Ok(doubled.next_power_of_two())
}

/// A magnitude spectrogram in audio time and frequency.
///
/// Magnitudes are calibrated as an amplitude spectrum: a sinusoid of amplitude
/// `A` at a bin centre reads `A` in that bin, whatever the window and transform
/// length. Rows are frames in increasing time, columns are bins from zero to
/// Nyquist inclusive.
#[derive(Debug, Clone)]
pub struct Spectrogram {
    /// The geometry used to build this spectrogram.
    config: StftConfig,
    /// Magnitude per frame and bin, row-major by frame.
    magnitudes: Vec<f32>,
    /// Window centre time of each frame, in audio seconds.
    frame_times_s: Vec<f64>,
}

impl Spectrogram {
    /// The geometry used to build this spectrogram.
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

    /// All magnitudes, row-major by frame.
    pub fn magnitudes(&self) -> &[f32] {
        &self.magnitudes
    }

    /// Magnitudes of one frame, or `None` past the last frame.
    pub fn frame(&self, frame: usize) -> Option<&[f32]> {
        let bins = self.bin_count();
        let start = frame.checked_mul(bins)?;
        let end = start.checked_add(bins)?;
        self.magnitudes.get(start..end)
    }

    /// Magnitude at one frame and bin, or `None` outside the map.
    pub fn magnitude(&self, frame: usize, bin: usize) -> Option<f32> {
        let index = frame.checked_mul(self.bin_count())?.checked_add(bin)?;
        self.magnitudes.get(index).copied()
    }

    /// Centre frequency of `bin` in hertz, or `None` past the last bin.
    pub fn bin_hz(&self, bin: usize) -> Option<f64> {
        self.config.bin_hz(bin)
    }

    /// Sub-bin peak frequency at `bin`, in hertz.
    ///
    /// Fits a parabola in log magnitude through `bin - 1`, `bin` and `bin + 1`
    /// and returns the vertex, which is the usual estimator for a windowed tone
    /// that falls between bins (SDD section 5). `None` when the bin is at an
    /// edge, the three magnitudes are not positive, or the vertex is not a peak.
    ///
    /// The vertex is limited to half a bin either side of `bin`, so the caller
    /// still has to choose the right bin.
    pub fn interpolated_peak_hz(&self, frame: usize, bin: usize) -> Option<f64> {
        let bins = self.bin_count();
        if bin == 0 || bin.checked_add(1)? >= bins {
            return None;
        }
        let previous = bin.checked_sub(1)?;
        let left = self.magnitude(frame, previous)?;
        let centre = self.magnitude(frame, bin)?;
        let right = self.magnitude(frame, bin.checked_add(1)?)?;
        if left <= 0.0 || centre <= 0.0 || right <= 0.0 {
            return None;
        }
        let left = f64::from(left).ln();
        let centre = f64::from(centre).ln();
        let right = f64::from(right).ln();
        let curvature = left - 2.0 * centre + right;
        // A peak has downward curvature; a flat or upward bin is not one.
        if !curvature.is_finite() || curvature >= 0.0 {
            return None;
        }
        let offset = 0.5 * (left - right) / curvature;
        if !offset.is_finite() {
            return None;
        }
        let offset = offset.clamp(-0.5, 0.5);
        Some(self.config.bin_hz(bin)? + offset * self.config.bin_width_hz())
    }
}

/// Compute a windowed magnitude spectrogram of mono samples.
///
/// Each frame is multiplied by the configured window, zero-padded to
/// `fft_len`, transformed with a real FFT and normalized to an amplitude
/// spectrum. Frames start every `hop_len` samples; a partial trailing frame is
/// not emitted.
///
/// # Errors
///
/// Returns [`DspError`] when the input is empty or non-finite, the geometry is
/// invalid, the input cannot hold one frame, or the transform rejects its
/// buffers.
pub fn spectrogram(input: &[f32], config: &StftConfig) -> Result<Spectrogram, DspError> {
    config.validate()?;
    if input.is_empty() {
        return Err(DspError::EmptyInput);
    }
    if input.iter().any(|sample| !sample.is_finite()) {
        return Err(DspError::NotFinite);
    }
    let frame_count = config.frame_count(input.len())?;
    let bins = config.bin_count();
    let weights = window(config.window, config.frame_len)?;
    let sum_of_weights: f64 = weights.iter().sum();
    if !sum_of_weights.is_finite() || sum_of_weights <= 0.0 {
        return Err(DspError::InvalidWindow { beta: f64::NAN });
    }
    // An amplitude spectrum needs the one-sided spectrum doubled and the
    // window's coherent gain divided out, so a tone of amplitude A reads A.
    let scale = 2.0 / sum_of_weights;

    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(config.fft_len);
    let mut frame_buffer = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();

    let capacity = frame_count
        .checked_mul(bins)
        .ok_or(DspError::LengthOverflow)?;
    let mut magnitudes = Vec::with_capacity(capacity);
    let mut frame_times_s = Vec::with_capacity(frame_count);

    for frame_index in 0..frame_count {
        let start = frame_index
            .checked_mul(config.hop_len)
            .ok_or(DspError::LengthOverflow)?;
        let end = start
            .checked_add(config.frame_len)
            .ok_or(DspError::LengthOverflow)?;
        let Some(samples) = input.get(start..end) else {
            return Err(DspError::InputTooShort {
                samples: input.len(),
                frame: config.frame_len,
            });
        };
        // The FFT may use its input as scratch, so every slot is rewritten.
        frame_buffer.fill(0.0);
        let Some(head) = frame_buffer.get_mut(..config.frame_len) else {
            return Err(DspError::InvalidFftLength {
                fft_len: config.fft_len,
                frame: config.frame_len,
            });
        };
        for (slot, (&sample, &weight)) in head.iter_mut().zip(samples.iter().zip(weights.iter())) {
            *slot = f64::from(sample) * weight;
        }
        fft.process_with_scratch(&mut frame_buffer, &mut spectrum, &mut scratch)
            .map_err(|_error| DspError::TransformFailed)?;
        magnitudes.extend(spectrum.iter().map(|value| (value.norm() * scale) as f32));
        frame_times_s.push(config.frame_time_s(start));
    }

    Ok(Spectrogram {
        config: *config,
        magnitudes,
        frame_times_s,
    })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::{StftConfig, spectrogram};
    use crate::error::DspError;
    use crate::window::WindowKind;

    const RATE_HZ: f64 = 4_000.0;

    fn tone(rate_hz: f64, count: usize, frequency_hz: f64, amplitude: f64) -> Vec<f32> {
        (0..count)
            .map(|index| {
                let time_s = index as f64 / rate_hz;
                (amplitude * (TAU * frequency_hz * time_s).sin()) as f32
            })
            .collect()
    }

    fn survey_config() -> StftConfig {
        StftConfig::survey(RATE_HZ).expect("survey configuration")
    }

    /// Peak bin of one frame of the spectrogram.
    fn peak_bin(samples: &[f32], config: &StftConfig, frame: usize) -> usize {
        let map = spectrogram(samples, config).expect("spectrogram");
        let row = map.frame(frame).expect("frame");
        let mut best = 0;
        let mut best_value = f64::NEG_INFINITY;
        for (bin, &value) in row.iter().enumerate() {
            if f64::from(value) > best_value {
                best_value = f64::from(value);
                best = bin;
            }
        }
        best
    }

    #[test]
    fn survey_configuration_uses_one_second_and_75_percent_overlap() {
        let config = survey_config();
        assert_eq!(config.frame_len, 4_000);
        assert_eq!(config.hop_len, 1_000);
        assert_eq!(config.fft_len, 8_192);
        assert!(matches!(config.window, WindowKind::Kaiser { .. }));
        // 8 frames for 7 s of audio: (28_000 - 4_000) / 1_000 + 1.
        assert_eq!(config.frame_count(28_000).expect("frames"), 25);
    }

    #[test]
    fn zero_padding_refines_the_bin_spacing_below_one_over_frame_len() {
        let config = survey_config();
        let unpadded_width = config.sample_rate_hz / config.frame_len as f64;
        assert!(config.fft_len > config.frame_len);
        assert!(
            config.bin_width_hz() < unpadded_width,
            "width {} unpadded {unpadded_width}",
            config.bin_width_hz()
        );
        assert_eq!(config.bin_count(), 4_097);
        assert_eq!(config.bin_hz(config.bin_count() - 1), Some(2_000.0));
        assert_eq!(config.bin_hz(config.bin_count()), None);
    }

    #[test]
    fn frame_times_are_window_centres() {
        let config = survey_config();
        let map = spectrogram(&vec![0.0_f32; 8_000], &config).expect("spectrogram");
        let times = map.frame_times_s();
        assert_eq!(times.len(), 5);
        let expected_first = (config.frame_len as f64 - 1.0) / 2.0 / RATE_HZ;
        assert!((times[0] - expected_first).abs() < 1.0e-12);
        let hop_s = config.hop_len as f64 / RATE_HZ;
        assert!((times[1] - times[0] - hop_s).abs() < 1.0e-12);
    }

    #[test]
    fn fr_dm_01_amplitude_spectrum_is_calibrated_to_a_sine() {
        let config = survey_config();
        let frequency_hz = config.bin_hz(819).expect("bin");
        let samples = tone(RATE_HZ, 8_000, frequency_hz, 0.5);
        let map = spectrogram(&samples, &config).expect("spectrogram");
        let peak = map.magnitude(2, 819).expect("peak magnitude").max(0.0);
        assert!(
            (peak - 0.5).abs() < 0.005,
            "peak magnitude {peak} for amplitude 0.5"
        );
    }

    #[test]
    fn fr_dm_01_parabolic_interpolation_recovers_an_off_bin_frequency() {
        let config = survey_config();
        // Bin 819 is 399.902 Hz; 400 Hz sits 0.2 bin above it.
        let frequency_hz = 400.0;
        let samples = tone(RATE_HZ, 8_000, frequency_hz, 0.5);
        let map = spectrogram(&samples, &config).expect("spectrogram");
        let peak = map.interpolated_peak_hz(2, 819).expect("interpolated peak");
        // A tenth of the 0.488 Hz bin: well above the interpolation bias of a
        // Kaiser-windowed peak at 0.2 bin and below the nearest real bin.
        assert!(
            (peak - frequency_hz).abs() < 0.05,
            "recovered {peak} Hz, expected {frequency_hz} Hz"
        );
    }

    #[test]
    fn fr_dm_01_interpolation_refines_a_frequency_off_the_peak_bin_only() {
        let config = survey_config();
        let samples = tone(RATE_HZ, 8_000, 400.0, 0.5);
        let map = spectrogram(&samples, &config).expect("spectrogram");
        let bin = peak_bin(&samples, &config, 2);
        let nearest = map.bin_hz(bin).expect("bin");
        let interpolated = map.interpolated_peak_hz(2, bin).expect("peak");
        assert!((interpolated - 400.0).abs() < (nearest - 400.0).abs());
    }

    #[test]
    fn interpolation_is_unavailable_at_the_edge_bins_and_on_silence() {
        let config = survey_config();
        let samples = tone(RATE_HZ, 8_000, 400.0, 0.5);
        let map = spectrogram(&samples, &config).expect("spectrogram");
        assert_eq!(map.interpolated_peak_hz(2, 0), None);
        assert_eq!(map.interpolated_peak_hz(2, map.bin_count() - 1), None);
        assert_eq!(map.interpolated_peak_hz(map.frame_count(), 10), None);

        let silence = spectrogram(&vec![0.0_f32; 8_000], &config).expect("spectrogram");
        let bin = silence.bin_count() / 2;
        assert_eq!(silence.interpolated_peak_hz(0, bin), None);
    }

    #[test]
    fn hann_windowed_transform_recovers_the_same_tone() {
        let mut config = survey_config();
        config.window = WindowKind::Hann;
        let samples = tone(RATE_HZ, 8_000, 400.0, 0.5);
        let map = spectrogram(&samples, &config).expect("spectrogram");
        let bin = map.bin_count() / 3;
        assert!(map.frame(0).is_some());
        assert!(map.magnitude(0, bin).is_some());
        let interpolated = map
            .interpolated_peak_hz(2, 819)
            .expect("hann interpolated peak");
        assert!(
            (interpolated - 400.0).abs() < 0.05,
            "recovered {interpolated}"
        );
    }

    #[test]
    fn spectrogram_is_deterministic() {
        let config = survey_config();
        let samples = tone(RATE_HZ, 8_000, 440.0, 0.25);
        let first = spectrogram(&samples, &config).expect("first");
        let second = spectrogram(&samples, &config).expect("second");
        assert_eq!(first.magnitudes(), second.magnitudes());
        assert_eq!(first.frame_times_s(), second.frame_times_s());
    }

    #[test]
    fn short_input_is_rejected() {
        let config = survey_config();
        let result = spectrogram(&vec![0.0_f32; 100], &config);
        assert!(matches!(result, Err(DspError::InputTooShort { .. })));
    }

    #[test]
    fn empty_input_is_rejected() {
        let config = survey_config();
        let result = spectrogram(&[], &config);
        assert!(matches!(result, Err(DspError::EmptyInput)));
    }

    #[test]
    fn non_finite_input_is_rejected() {
        let config = survey_config();
        let samples = tone(RATE_HZ, 8_000, 400.0, 0.5);
        let mut samples = samples;
        if let Some(first) = samples.first_mut() {
            *first = f32::NAN;
        }
        let result = spectrogram(&samples, &config);
        assert!(matches!(result, Err(DspError::NotFinite)));
    }

    #[test]
    fn transform_shorter_than_the_frame_is_rejected() {
        let mut config = survey_config();
        config.fft_len = config.frame_len - 1;
        assert!(matches!(
            config.validate(),
            Err(DspError::InvalidFftLength { .. })
        ));
    }

    #[test]
    fn odd_transform_length_is_rejected() {
        let mut config = survey_config();
        config.fft_len += 1;
        assert!(matches!(
            config.validate(),
            Err(DspError::InvalidFftLength { .. })
        ));
    }

    #[test]
    fn hop_beyond_the_frame_is_rejected() {
        let mut config = survey_config();
        config.hop_len = config.frame_len + 1;
        assert!(matches!(
            config.validate(),
            Err(DspError::InvalidHop { .. })
        ));
    }

    #[test]
    fn invalid_kaiser_parameter_is_rejected_by_the_config() {
        let mut config = survey_config();
        config.window = WindowKind::Kaiser { beta: f64::NAN };
        assert!(matches!(
            config.validate(),
            Err(DspError::InvalidWindow { .. })
        ));
    }

    #[test]
    fn survey_rejects_a_non_positive_rate() {
        assert!(matches!(StftConfig::survey(0.0), Err(DspError::NotFinite)));
        assert!(matches!(
            StftConfig::survey(f64::NAN),
            Err(DspError::NotFinite)
        ));
    }
}
