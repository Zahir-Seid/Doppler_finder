//! Audio and truth-file output (FR-SIM-01, SDD section 7).
//!
//! SDD section 7 makes one scenario and one seed produce three artefacts: a
//! WAV file, a truth file and a degraded ADS-B feed. The feed lives in
//! [`crate::inject`]; this module writes the audio and the truth.
//!
//! The truth file is the contract between the simulator and the tests that
//! score an estimator: it states, per pass, the emitted and heard closest
//! approach, the slant range there, and the tone parameters, all on the audio
//! clock. Nothing in it is estimated.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use acoustics::Atmosphere;
use serde::{Deserialize, Serialize};

use crate::error::SimError;
use crate::scenario::{Observer, Scenario};
use crate::synth::{Samples, SynthesisConfig};

/// The largest sample rate a 16-bit PCM WAV header can carry comfortably.
const MAX_SAMPLE_RATE_HZ: f64 = 384_000.0;

/// One pass as the synthesis actually produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TruthEvent {
    /// Callsign of the pass.
    pub callsign: String,
    /// Emission time of closest approach, seconds from the recording start.
    pub emitted_closest_approach_s: f64,
    /// Reception time of closest approach, seconds from the recording start.
    ///
    /// This is the time a listener hears the pass at its lowest pitch and
    /// greatest level, and it is what an estimator reports as `t0`.
    pub heard_closest_approach_s: f64,
    /// Sound travel time between the two, seconds.
    pub propagation_delay_s: f64,
    /// Slant range at closest approach, metres.
    pub closest_range_m: f64,
    /// Horizontal miss distance, metres.
    pub closest_approach_m: f64,
    /// Geometric altitude, metres.
    pub altitude_m: f64,
    /// True ground speed, metres per second.
    pub speed_m_per_s: f64,
    /// Emitted fundamental frequency, hertz.
    pub fundamental_hz: f64,
    /// Relative harmonic weights as the scenario gave them.
    pub harmonics: Vec<f64>,
}

/// Ground truth for one simulated recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Truth {
    /// Scenario name.
    pub scenario: String,
    /// Seed that produced the noise and the feed.
    pub seed: u64,
    /// Audio sample rate, hertz.
    pub sample_rate_hz: f64,
    /// Audio duration, seconds.
    pub duration_s: f64,
    /// Listener definition.
    pub observer: Observer,
    /// Speed of sound the synthesis used, metres per second.
    ///
    /// Recorded rather than recomputed so a change in the atmosphere model
    /// cannot silently reinterpret an old truth file.
    pub sound_speed_m_per_s: f64,
    /// One entry per pass, in scenario order.
    pub events: Vec<TruthEvent>,
}

/// Build the truth for a scenario without synthesizing the audio.
///
/// The geometry is the closed form of SDD section 4: the slant range at
/// closest approach is `hypot(closest_approach_m, altitude_m - ear_height_m)`,
/// and the heard time is the emitted time plus that range over the speed of
/// sound.
///
/// Returns [`SimError`] for an invalid scenario or configuration and
/// [`SimError::PassBelowObserver`] for a pass that does not clear the ear
/// height.
pub fn truth(
    scenario: &Scenario,
    config: &SynthesisConfig,
    atmosphere: &Atmosphere,
) -> Result<Truth, SimError> {
    scenario.validate()?;
    config.sample_count()?;
    let sound_speed_m_per_s = atmosphere.speed_of_sound_m_per_s();

    let mut events = Vec::with_capacity(scenario.passes.len());
    for (index, pass) in scenario.passes.iter().enumerate() {
        let height_m = pass.altitude_m - scenario.observer.ear_height_m;
        if !height_m.is_finite() || height_m <= 0.0 {
            return Err(SimError::PassBelowObserver { index });
        }
        let closest_range_m = pass.closest_approach_m.hypot(height_m);
        let propagation_delay_s = closest_range_m / sound_speed_m_per_s;
        events.push(TruthEvent {
            callsign: pass.callsign.clone(),
            emitted_closest_approach_s: pass.offset_s,
            heard_closest_approach_s: pass.offset_s + propagation_delay_s,
            propagation_delay_s,
            closest_range_m,
            closest_approach_m: pass.closest_approach_m,
            altitude_m: pass.altitude_m,
            speed_m_per_s: pass.velocity_m_per_s,
            fundamental_hz: pass.fundamental_hz,
            harmonics: pass.harmonics.clone(),
        });
    }

    Ok(Truth {
        scenario: scenario.name.clone(),
        seed: scenario.seed,
        sample_rate_hz: config.sample_rate_hz,
        duration_s: config.duration_s,
        observer: scenario.observer,
        sound_speed_m_per_s,
        events,
    })
}

/// Write mono samples as a 16-bit PCM WAV file.
///
/// Samples are clamped to `[-1, 1]` and quantized to `i16`, which is the
/// format the decode step of the pipeline will read back. The header rate is
/// the sample rate rounded to the nearest hertz.
///
/// Returns [`SimError::InvalidSampleRate`] for a rate outside
/// `1 Hz ..= 384 kHz` and the file errors of [`std::io`] and [`hound`].
pub fn write_wav(path: impl AsRef<Path>, samples: &Samples) -> Result<(), SimError> {
    let rate = samples.sample_rate_hz();
    if !rate.is_finite() || rate < 1.0 || rate > MAX_SAMPLE_RATE_HZ {
        return Err(SimError::InvalidSampleRate(rate));
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate.round() as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for value in samples.values() {
        let clamped = value.clamp(-1.0, 1.0);
        let scaled = (clamped * f32::from(i16::MAX)).round();
        writer.write_sample(scaled as i16)?;
    }
    writer.finalize()?;
    Ok(())
}

/// Write the truth as indented JSON.
///
/// Returns the file and serialization errors of [`std::io`] and
/// [`serde_json`].
pub fn write_truth(path: impl AsRef<Path>, truth: &Truth) -> Result<(), SimError> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, truth)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{AircraftPass, Observer};
    use crate::synth::synthesize;
    use core_types::LatLon;

    /// Amplitude tolerance for a 16-bit round trip: one least-significant bit
    /// is 1/32767, so half of it plus rounding is the whole error budget.
    const QUANTIZATION_TOL: f32 = 1.0 / 32_767.0;

    fn pass() -> AircraftPass {
        AircraftPass {
            callsign: "TEST123".to_owned(),
            velocity_m_per_s: 90.0,
            closest_approach_m: 800.0,
            offset_s: 30.0,
            altitude_m: 600.0,
            heading_deg: 90.0,
            fundamental_hz: 100.0,
            harmonics: vec![1.0, 0.5],
        }
    }

    fn scenario() -> Scenario {
        Scenario {
            name: "export".to_owned(),
            seed: 3,
            duration_s: 2.0,
            observer: Observer {
                position: LatLon::new(52.0, 13.0),
                ear_height_m: 1.7,
            },
            passes: vec![pass()],
        }
    }

    fn config() -> SynthesisConfig {
        SynthesisConfig {
            sample_rate_hz: 8_000.0,
            duration_s: 2.0,
            amplitude: 0.5,
            absorption: crate::AirAbsorption::none(),
            noise: None,
        }
    }

    #[test]
    fn truth_matches_the_closed_form_geometry() {
        let atmosphere = Atmosphere::standard();
        let truth = truth(&scenario(), &config(), &atmosphere).expect("valid scenario");
        let event = truth.events.first().expect("one event");
        let expected_range = 800.0_f64.hypot(600.0 - 1.7);
        let sound_speed = atmosphere.speed_of_sound_m_per_s();
        assert!((event.closest_range_m - expected_range).abs() < 1e-9);
        assert!((event.propagation_delay_s - expected_range / sound_speed).abs() < 1e-12);
        assert!(
            (event.heard_closest_approach_s - (30.0 + expected_range / sound_speed)).abs() < 1e-12
        );
        assert!(event.heard_closest_approach_s > event.emitted_closest_approach_s);
        assert!((truth.sound_speed_m_per_s - sound_speed).abs() < 1e-12);
        assert_eq!(truth.seed, 3);
    }

    #[test]
    fn wav_round_trips_through_the_decoder() {
        let atmosphere = Atmosphere::standard();
        let samples = synthesize(&scenario(), &config(), &atmosphere).expect("valid");
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("pass.wav");
        write_wav(&path, &samples).expect("writes");

        let mut reader = hound::WavReader::open(&path).expect("reads");
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 8_000);
        assert_eq!(reader.spec().bits_per_sample, 16);
        let decoded: Vec<i16> = reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .expect("valid samples");
        assert_eq!(decoded.len(), samples.len());
        let peak_decoded = decoded
            .iter()
            .map(|value| f32::from(*value).abs() / f32::from(i16::MAX))
            .fold(0.0_f32, f32::max);
        assert!((peak_decoded - samples.peak_amplitude()).abs() <= QUANTIZATION_TOL);
    }

    #[test]
    fn truth_file_round_trips_through_json() {
        let atmosphere = Atmosphere::standard();
        let truth = truth(&scenario(), &config(), &atmosphere).expect("valid scenario");
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("truth.json");
        write_truth(&path, &truth).expect("writes");
        let text = std::fs::read_to_string(&path).expect("reads");
        let parsed: Truth = serde_json::from_str(&text).expect("parses");
        assert_eq!(parsed, truth);
    }

    #[test]
    fn wav_writer_rejects_an_impossible_rate() {
        let atmosphere = Atmosphere::standard();
        // Half a hertz over two seconds is one sample: a legal buffer with a
        // header rate no player could use.
        let mut slow = config();
        slow.sample_rate_hz = 0.5;
        let samples = synthesize(&scenario(), &slow, &atmosphere).expect("one sample");
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("slow.wav");
        assert!(matches!(
            write_wav(&path, &samples),
            Err(SimError::InvalidSampleRate(_))
        ));
    }

    #[test]
    fn truth_rejects_a_pass_at_the_ear_height() {
        let atmosphere = Atmosphere::standard();
        let mut low = scenario();
        if let Some(pass) = low.passes.first_mut() {
            pass.altitude_m = 1.0;
        }
        assert!(matches!(
            truth(&low, &config(), &atmosphere),
            Err(SimError::PassBelowObserver { index: 0 })
        ));
    }
}
