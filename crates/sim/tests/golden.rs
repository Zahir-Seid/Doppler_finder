//! Golden regression over the shipped example scenario (FR-SIM-04).
//!
//! The audio digest and the truth values below were recorded from the first
//! implementation that passed the physics tests. They are not a calibration:
//! they exist so a refactor cannot quietly change what the simulator emits.
//! Changing a constant is a deliberate act and belongs in the commit message.

use acoustics::Atmosphere;
use sha2::{Digest, Sha256};
use sim::{AirAbsorption, NoiseConfig, Scenario, SynthesisConfig, synthesize, truth};

/// Analysis rate of the golden run, hertz.
const SAMPLE_RATE_HZ: f64 = 8_000.0;

/// Peak amplitude of the golden run.
const AMPLITUDE: f64 = 0.5;

/// Signal-to-noise ratio of the golden run, decibels.
const SNR_DB: f64 = 30.0;

/// SHA-256 over the sample rate and every little-endian `f32` sample.
const AUDIO_DIGEST: &str = "2849cf04fae5cf46c73b7413c849d7ac2cf14396ef18ade54383435c428d12fe";

/// Samples in the sixty-second golden run.
const SAMPLE_COUNT: usize = 480_000;

/// Peak of the golden run: the pass peak lowered by absorption-free spreading
/// and slightly raised by the 30 dB noise.
const PEAK_AMPLITUDE: f32 = 0.447_416_87;

fn scenario() -> Result<Scenario, serde_json::Error> {
    serde_json::from_str(include_str!("../../../scenarios/example-single-pass.json"))
}

fn config(duration_s: f64) -> SynthesisConfig {
    SynthesisConfig {
        sample_rate_hz: SAMPLE_RATE_HZ,
        duration_s,
        amplitude: AMPLITUDE,
        absorption: AirAbsorption::none(),
        noise: Some(NoiseConfig::white(SNR_DB)),
    }
}

#[test]
fn example_scenario_audio_matches_the_recorded_digest() {
    let scenario = scenario().expect("the example scenario is valid JSON");
    let samples = synthesize(
        &scenario,
        &config(scenario.duration_s),
        &Atmosphere::standard(),
    )
    .expect("the example scenario synthesizes");

    let mut hasher = Sha256::new();
    hasher.update(samples.sample_rate_hz().to_le_bytes());
    for value in samples.values() {
        hasher.update(value.to_le_bytes());
    }
    assert_eq!(
        format!("{:x}", hasher.finalize()),
        AUDIO_DIGEST,
        "the example scenario's audio changed; update the digest only for an intended change"
    );
    assert_eq!(samples.len(), SAMPLE_COUNT);
    assert!(
        (samples.peak_amplitude() - PEAK_AMPLITUDE).abs() < 1e-6,
        "peak {}",
        samples.peak_amplitude()
    );
}

#[test]
fn example_scenario_truth_matches_the_recorded_values() {
    let scenario = scenario().expect("the example scenario is valid JSON");
    let truth = truth(
        &scenario,
        &config(scenario.duration_s),
        &Atmosphere::standard(),
    )
    .expect("the example scenario has truth");

    assert_eq!(truth.events.len(), 1);
    let event = truth.events.first().expect("one event");
    // The geometry is closed-form, so these are recorded to the last bit and
    // compared to a nanometre; a change means the model moved.
    assert!((truth.sound_speed_m_per_s - 340.275_080_511_860_5).abs() < 1e-9);
    assert!((event.closest_range_m - 998.980_925_743_830_2).abs() < 1e-9);
    assert!((event.propagation_delay_s - 2.935_803_950_853_845).abs() < 1e-9);
    assert!((event.heard_closest_approach_s - 32.935_803_950_853_845).abs() < 1e-9);
    assert!((event.emitted_closest_approach_s - 30.0).abs() < 1e-12);
    assert!((event.closest_approach_m - 800.0).abs() < 1e-12);
    assert!((event.speed_m_per_s - 90.0).abs() < 1e-12);
    assert!((event.fundamental_hz - 100.0).abs() < 1e-12);
    assert_eq!(event.harmonics, vec![1.0, 0.6, 0.3]);
}
