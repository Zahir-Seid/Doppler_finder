//! M2 exit criterion (FR-IN-01, FR-DM-01): one simulated pass travels through
//! the whole front end.
//!
//! A 16-bit 48 kHz WAV carries a known straight-line Doppler curve plus seeded
//! white noise. The test decodes it with the FFmpeg adapter, resamples to the
//! 4 kHz analysis rate, forms a survey spectrogram, whitens it into a z-map and
//! tracks a ridge through a corridor built from the same analytic curve. The
//! recovered frequency must follow the truth to within a stated tolerance and
//! the result must be reproducible.
//!
//! The geometry is chosen so the pass is visible to both halves of the front
//! end at once. Whitening needs the sweep to cross a bin faster than the
//! 15-frame noise window, and ridge tracking needs the per-frame step to stay
//! inside the configured limit; a 45 m/s pass 450 m away sweeps about 5 Hz/s,
//! which is roughly 2.7 survey bins per frame with a dwell of about 3.5 frames
//! per bin.
//!
//! The decoder test is skipped with a printed reason when no FFmpeg is
//! installed, so the suite stays offline and portable.

use std::f64::consts::TAU;
use std::path::PathBuf;

use dsp::{
    Corridor, DecoderConfig, FfmpegDecoder, FrontEndConfig, MediaDecoder, RidgeConfig, RidgeMethod,
    StftConfig, WhiteningConfig, resample_to_analysis_rate, spectrogram, track_ridge, whiten,
};

/// Input sample rate of the simulated recording, in hertz.
const INPUT_RATE_HZ: f64 = 48_000.0;

/// Analysis sample rate after the front end, in hertz.
const ANALYSIS_RATE_HZ: f64 = 4_000.0;

/// Length of the simulated recording, in seconds.
const DURATION_S: f64 = 6.0;

/// Speed of sound used to build the curve, in metres per second.
const SOUND_SPEED_M_PER_S: f64 = 343.0;

/// Aircraft speed over the ground, in metres per second (about 160 km/h).
const SPEED_M_PER_S: f64 = 45.0;

/// Closest approach distance, in metres.
const CLOSEST_M: f64 = 450.0;

/// Fundamental frequency at closest approach, in hertz.
const CLOSEST_HZ: f64 = 400.0;

/// Tone amplitude of the pass, as a fraction of full scale.
const TONE_AMPLITUDE: f64 = 0.3;

/// Noise amplitude of the recording, as a fraction of full scale.
const NOISE_AMPLITUDE: f64 = 0.01;

/// Tolerated slack outside the swept band of a frame, in bins.
///
/// The assertion is measured against the band a frame's window spans, because
/// the transform cannot resolve finer than that: a one-second frame of this
/// pass covers about 5.3 survey bins. Measured runs put the greedy peak up to
/// about 1.2 bins outside that band, because whitening normalises each bin
/// against the neighbouring frames, so the band is widened by two and a half
/// bins; that stays well inside the corridor the detector supplies.
const TOLERANCE_BINS: f64 = 2.5;

/// Corridor half-width around the truth, in bins.
///
/// A real detector does not know the curve this well; the width is set wide
/// enough that the corridor is not the thing doing the tracking.
const CORRIDOR_HALF_WIDTH_BINS: usize = 8;

/// Instantaneous frequency of the pass at `t_rel` seconds from closest
/// approach, in hertz.
///
/// A straight-line constant-speed pass has `f = f0 c / (c - v_r)` with the
/// radial velocity `v_r = -v^2 t / sqrt(d^2 + (v t)^2)` positive while the
/// aircraft approaches (SDD section 4). The frame times of the transform are
/// arrival times; the emission-time correction is below the stated tolerance at
/// this speed and distance, so it is not solved here.
fn predicted_hz(t_rel: f64) -> f64 {
    let range_m = (CLOSEST_M * CLOSEST_M + (SPEED_M_PER_S * t_rel).powi(2)).sqrt();
    let radial_m_per_s = -SPEED_M_PER_S * SPEED_M_PER_S * t_rel / range_m;
    CLOSEST_HZ * SOUND_SPEED_M_PER_S / (SOUND_SPEED_M_PER_S - radial_m_per_s)
}

/// Deterministic white noise in `[-1, 1]`, so the test needs no dependency.
fn white_noise(count: usize, seed: u32) -> Vec<f64> {
    let mut state = seed | 1;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (f64::from(state) / f64::from(u32::MAX)) * 2.0 - 1.0
        })
        .collect()
}

/// The simulated pass as 16-bit PCM samples: a phase-continuous chirp of the
/// analytic curve over seeded noise.
fn pass_samples() -> Vec<i16> {
    let count = (DURATION_S * INPUT_RATE_HZ) as usize;
    let noise = white_noise(count, 20_261);
    let mut phase = 0.0_f64;
    let mut samples = Vec::with_capacity(count);
    for (index, &noise_value) in noise.iter().enumerate() {
        let time_s = index as f64 / INPUT_RATE_HZ;
        let t_rel = time_s - 0.5 * DURATION_S;
        let frequency_hz = predicted_hz(t_rel);
        let value = TONE_AMPLITUDE * phase.sin() + NOISE_AMPLITUDE * noise_value;
        samples.push((value.clamp(-1.0, 1.0) * f64::from(i16::MAX)).round() as i16);
        phase = (phase + TAU * frequency_hz / INPUT_RATE_HZ).rem_euclid(TAU);
    }
    samples
}

/// A mono 16-bit PCM WAV file around `samples` in memory.
fn wav_bytes(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let block_align = 2_u32;
    let data_len = (samples.len() as u32).saturating_mul(block_align);
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate.saturating_mul(block_align)).to_le_bytes());
    bytes.extend_from_slice(&(block_align as u16).to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// Write the simulated recording to a temporary file.
fn write_recording() -> std::io::Result<(tempfile::TempDir, PathBuf)> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("pass.wav");
    let samples = pass_samples();
    std::fs::write(&path, wav_bytes(&samples, INPUT_RATE_HZ as u32))?;
    Ok((directory, path))
}

#[test]
fn m2_exit_a_simulated_pass_is_decoded_and_tracked_to_the_known_curve() {
    let decoder = FfmpegDecoder::new(DecoderConfig::for_sample_rate(INPUT_RATE_HZ))
        .expect("decoder configuration");
    if decoder.version().is_err() {
        eprintln!("skipping the M2 exit test: no ffmpeg on the search path");
        return;
    }
    let (_directory, path) = write_recording().expect("write the simulated recording");

    // Decode: the adapter is asked for mono 32-bit float at the input rate.
    let decoded = decoder.decode(&path).expect("decode the simulated pass");
    assert_eq!(decoded.sample_rate_hz(), INPUT_RATE_HZ);
    assert!(
        (decoded.duration_s() - DURATION_S).abs() < 0.01,
        "decoded duration {}",
        decoded.duration_s()
    );

    // Band-limit and resample to the analysis rate, then remove DC.
    let front_end = FrontEndConfig::for_input_rate(INPUT_RATE_HZ);
    let analysis = resample_to_analysis_rate(decoded.samples(), &front_end)
        .expect("resample to the analysis rate");
    let expected_samples = DURATION_S * ANALYSIS_RATE_HZ;
    assert!(
        (analysis.len() as f64 - expected_samples).abs() < 2.0 * f64::from(1_000_u32),
        "analysis length {} for {} expected",
        analysis.len(),
        expected_samples
    );

    // Transform with the survey geometry and whiten. One frame is dropped at
    // each end: the polyphase FIR has no history before the first sample and no
    // tail after the last, so its edge frames are not a fair test of tracking.
    let stft = StftConfig::survey(ANALYSIS_RATE_HZ).expect("survey geometry");
    let edge = stft.frame_len;
    let trimmed = analysis
        .get(edge..analysis.len().saturating_sub(edge))
        .expect("analysis long enough to trim one frame at each end");
    // Buffer time plus this offset gives seconds from closest approach.
    let origin_s = edge as f64 / ANALYSIS_RATE_HZ - 0.5 * DURATION_S;

    let map = spectrogram(trimmed, &stft).expect("spectrogram");
    let zmap = whiten(&map, &WhiteningConfig::default()).expect("whitening");
    assert!(zmap.frame_count() >= 10, "frames {}", zmap.frame_count());

    // The corridor a detector would supply, built from the analytic curve.
    let bin_width_hz = zmap.config().bin_width_hz();
    let bins = zmap.bin_count();
    let mut bounds = Vec::with_capacity(zmap.frame_count());
    for &frame_time_s in zmap.frame_times_s() {
        let truth_bin = predicted_hz(frame_time_s + origin_s) / bin_width_hz;
        let centre = truth_bin.round().max(0.0) as usize;
        let low = centre.saturating_sub(CORRIDOR_HALF_WIDTH_BINS);
        let high = centre
            .saturating_add(CORRIDOR_HALF_WIDTH_BINS)
            .min(bins - 1);
        bounds.push((low, high.max(low)));
    }
    let corridor = Corridor::new(bounds, bins).expect("corridor from the truth curve");

    // Track, then compare against the same curve the audio was built from.
    let config = RidgeConfig::default();
    let ridge = track_ridge(&zmap, &corridor, &config).expect("ridge");
    assert_eq!(
        ridge.method(),
        RidgeMethod::Peaks,
        "a corridor around the truth should be clean at level one"
    );
    assert_eq!(ridge.frame_count(), zmap.frame_count());

    // Each frame is compared against the band its window spans. Level one takes
    // the argmax of the z-map, and a bin the sweep has only just reached has the
    // smallest window median, so the greedy peak sits at the leading edge of the
    // smear rather than at the window centre. That bias is a property of peak
    // picking, not of the transform; the estimator fits the whole curve instead.
    let half_window_s = (stft.frame_len as f64 - 1.0) / 2.0 / ANALYSIS_RATE_HZ;
    for (frame, point) in ridge.points().iter().enumerate() {
        let centre_s = zmap.frame_times_s()[frame] + origin_s;
        let before_hz = predicted_hz(centre_s - half_window_s);
        let after_hz = predicted_hz(centre_s + half_window_s);
        let low_hz = before_hz.min(after_hz) - TOLERANCE_BINS * bin_width_hz;
        let high_hz = before_hz.max(after_hz) + TOLERANCE_BINS * bin_width_hz;
        assert!(
            point.z >= config.min_peak_z,
            "frame {frame} z {} is below the usable level",
            point.z
        );
        assert!(
            (low_hz..=high_hz).contains(&point.frequency_hz),
            "frame {frame} tracked {} Hz outside the swept band {low_hz}..{high_hz} Hz",
            point.frequency_hz
        );
    }

    // The same input must give the same curve.
    let again = track_ridge(&zmap, &corridor, &config).expect("ridge again");
    assert_eq!(again.method(), ridge.method());
    for (first, second) in ridge.points().iter().zip(again.points().iter()) {
        assert_eq!(first.bin, second.bin);
        assert_eq!(first.frequency_hz, second.frequency_hz);
    }
}
