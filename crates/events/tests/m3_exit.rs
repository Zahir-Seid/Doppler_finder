//! M3 exit criterion (FR-DM-01 to FR-DM-06): one simulated pass travels from
//! audio to a quality-scored event.
//!
//! A phase-continuous chord of the first two propeller harmonics is synthesised
//! at the analysis rate on a straight-line emission-time curve over seeded
//! noise. The test forms the survey spectrogram, whitens it, runs the detector,
//! seeds the direct estimator with the detection and finally scores the fitted
//! event. The recovered curve and fundamental must agree with truth, the
//! covariance must be usable, and a noise-only recording must be rejected.

#![allow(clippy::expect_used)]

use acoustics::StraightPass;
use core_types::{EventId, Hertz};
use dsp::{StftConfig, WhiteningConfig, spectrogram, whiten};
use events::{DetectorConfig, DirectConfig, QualityConfig, assess, detect, fit_event_direct};

/// Analysis sample rate, in hertz.
const SAMPLE_RATE_HZ: f64 = 4_000.0;
/// Recording length, in seconds.
const DURATION_S: f64 = 8.0;
/// Speed of sound, in metres per second.
const SOUND_SPEED_M_PER_S: f64 = 343.0;
/// Aircraft speed, in metres per second.
const SPEED_M_PER_S: f64 = 80.0;
/// Closest approach distance, in metres.
const CLOSEST_M: f64 = 500.0;
/// Fundamental frequency at emission, in hertz.
const FUNDAMENTAL_HZ: f64 = 200.0;
/// Heard closest-approach time on the audio clock, in seconds.
const HEARD_AT_S: f64 = 4.0;
/// Amplitude of the fundamental, as a fraction of full scale.
const FUNDAMENTAL_AMPLITUDE: f64 = 0.3;
/// Amplitude of the second harmonic, as a fraction of full scale.
const SECOND_AMPLITUDE: f64 = 0.1;
/// Noise amplitude, as a fraction of full scale.
const NOISE_AMPLITUDE: f64 = 0.01;
/// Harmonics passed to the direct estimator.
const HARMONICS: [u32; 2] = [1, 2];

fn pass() -> StraightPass {
    StraightPass {
        closest_approach_m: CLOSEST_M,
        speed_m_per_s: SPEED_M_PER_S,
    }
}

/// True received frequency of one harmonic at one audio time.
fn model_hz(harmonic: u32, time_s: f64) -> f64 {
    let received = pass()
        .received_offset_from_heard_offset(time_s - HEARD_AT_S, SOUND_SPEED_M_PER_S)
        .expect("valid geometry")
        .0;
    pass()
        .received_frequency(
            Hertz(FUNDAMENTAL_HZ),
            harmonic,
            received,
            SOUND_SPEED_M_PER_S,
        )
        .expect("valid geometry")
        .0
}

/// Deterministic white noise in `[-1, 1]`.
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

/// A phase-continuous two-harmonic pass over seeded noise.
fn pass_samples(with_tone: bool) -> Vec<f32> {
    let count = (DURATION_S * SAMPLE_RATE_HZ) as usize;
    let noise = white_noise(count, 20_261);
    let mut phase = [0.0_f64; HARMONICS.len()];
    let mut samples = Vec::with_capacity(count);
    for (index, &noise_value) in noise.iter().enumerate() {
        let time_s = index as f64 / SAMPLE_RATE_HZ;
        let mut value = NOISE_AMPLITUDE * noise_value;
        if with_tone {
            for (slot, &harmonic) in phase.iter_mut().zip(HARMONICS.iter()) {
                let amplitude = if harmonic == 1 {
                    FUNDAMENTAL_AMPLITUDE
                } else {
                    SECOND_AMPLITUDE
                };
                value += amplitude * slot.sin();
                let frequency_hz = model_hz(harmonic, time_s);
                *slot = (*slot + std::f64::consts::TAU * frequency_hz / SAMPLE_RATE_HZ)
                    .rem_euclid(std::f64::consts::TAU);
            }
        }
        samples.push(value.clamp(-1.0, 1.0) as f32);
    }
    samples
}

/// Whitened survey map of one recording.
fn whitened_map(with_tone: bool) -> dsp::ZMap {
    let stft = StftConfig::survey(SAMPLE_RATE_HZ).expect("survey geometry");
    let spectrogram = spectrogram(&pass_samples(with_tone), &stft).expect("spectrogram");
    whiten(&spectrogram, &WhiteningConfig::default()).expect("whitening")
}

fn detector_config() -> DetectorConfig {
    DetectorConfig {
        speeds_m_per_s: events::GridRange::new(60.0, 100.0, 20.0),
        closest_m: events::GridRange::new(400.0, 600.0, 100.0),
        fundamental_hz: events::GridRange::new(180.0, 220.0, 20.0),
        sound_speed_m_per_s: SOUND_SPEED_M_PER_S,
        ..DetectorConfig::default()
    }
}

fn direct_config() -> DirectConfig {
    DirectConfig {
        sound_speed_m_per_s: SOUND_SPEED_M_PER_S,
        ..DirectConfig::default()
    }
}

fn quality_config() -> QualityConfig {
    QualityConfig {
        sound_speed_m_per_s: SOUND_SPEED_M_PER_S,
        ..QualityConfig::default()
    }
}

#[test]
fn m3_exit_a_simulated_pass_is_detected_fitted_and_scored() {
    let map = whitened_map(true);
    let detection = detect(&map, &detector_config()).expect("detection");
    assert!(
        detection.accepted,
        "score {:.1}/{:.1}, approach {:.1}/{:.1}, recede {:.1}/{:.1}",
        detection.score,
        detection.threshold,
        detection.approach_score,
        detection.approach_threshold,
        detection.recede_score,
        detection.recede_threshold
    );

    let found = detection.hypothesis;
    eprintln!(
        "detected {:?} score {:.1}/{:.1} approach {:.1}/{:.1} recede {:.1}/{:.1}",
        found,
        detection.score,
        detection.threshold,
        detection.approach_score,
        detection.approach_threshold,
        detection.recede_score,
        detection.recede_threshold
    );
    assert!(
        (found.fundamental_hz.0 - FUNDAMENTAL_HZ).abs() <= 20.0,
        "detected fundamental {}",
        found.fundamental_hz.0
    );
    assert!(
        (found.heard_at.0 - HEARD_AT_S).abs() <= 0.5,
        "detected heard time {}",
        found.heard_at.0
    );

    let report = fit_event_direct(
        &map,
        EventId::new_v7(),
        &found,
        &HARMONICS,
        &direct_config(),
    )
    .expect("direct fit");
    let fit = &report.fit;
    assert!(
        (fit.fundamental_hz.0 - FUNDAMENTAL_HZ).abs() < 0.03 * FUNDAMENTAL_HZ,
        "fitted fundamental {}",
        fit.fundamental_hz.0
    );
    assert!(
        (fit.heard_at.0 - HEARD_AT_S).abs() < 0.1,
        "fitted heard time {}",
        fit.heard_at.0
    );
    for index in 0..4 {
        let variance = fit.covariance[index][index];
        assert!(
            variance.is_finite() && variance > 0.0,
            "covariance diagonal {index} is {variance}"
        );
    }

    // The fitted curve is the physical observable over this short window, so
    // compare it with the truth at several times rather than trust the
    // speed/distance valley. One bin is the map's resolution; the fit stays
    // inside it while the individual speed and distance drift along the valley.
    let fitted_pass = StraightPass {
        closest_approach_m: fit.closest_m.0,
        speed_m_per_s: fit.speed_m_per_s,
    };
    for time_s in [0.0, 2.0, 4.0, 6.0, 8.0] {
        let expected = model_hz(1, time_s);
        let fitted = curve_hz(
            &fitted_pass,
            fit.fundamental_hz.0,
            fit.heard_at.0,
            1,
            time_s,
        );
        assert!(
            (fitted - expected).abs() < map.config().bin_width_hz(),
            "curve at {time_s} s: {fitted} vs {expected} Hz"
        );
    }

    let assessment = assess(&map, &report, &quality_config()).expect("quality assessment");
    assert!(assessment.retained, "{:?}", assessment.reasons);
    assert_eq!(assessment.harmonic_count, HARMONICS.len());
    assert!(
        assessment.quality.score >= quality_config().retention_score,
        "quality {}",
        assessment.quality.score
    );
}

#[test]
fn m3_exit_noise_alone_is_not_detected() {
    let map = whitened_map(false);
    let detection = detect(&map, &detector_config()).expect("detection");
    assert!(
        !detection.accepted,
        "noise accepted at {} against threshold {}",
        detection.score, detection.threshold
    );
}

/// Received frequency of one harmonic for arbitrary fit parameters.
fn curve_hz(
    pass: &StraightPass,
    fundamental_hz: f64,
    heard_at_s: f64,
    harmonic: u32,
    time_s: f64,
) -> f64 {
    let received = pass
        .received_offset_from_heard_offset(time_s - heard_at_s, SOUND_SPEED_M_PER_S)
        .expect("valid geometry")
        .0;
    pass.received_frequency(
        Hertz(fundamental_hz),
        harmonic,
        received,
        SOUND_SPEED_M_PER_S,
    )
    .expect("valid geometry")
    .0
}
