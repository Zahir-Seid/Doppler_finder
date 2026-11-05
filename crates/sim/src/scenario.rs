//! Scenario model (`scenarios/*.json`, SDD section 7).

use core_types::LatLon;
use serde::{Deserialize, Serialize};

use crate::error::SimError;

/// Fundamental frequency assumed when a scenario omits the tone, hertz.
fn default_fundamental_hz() -> f64 {
    100.0
}

/// Harmonic comb assumed when a scenario omits one: the fundamental alone.
fn default_harmonics() -> Vec<f64> {
    vec![1.0]
}

/// The listener position and ear height.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Observer {
    /// Geodetic observer position.
    pub position: LatLon,
    /// Ear height above the terrain in metres.
    pub ear_height_m: f64,
}

/// One aircraft pass through the scenario.
///
/// The pass is straight and level: it flies a constant altitude and speed, and
/// its closest approach to the observer falls `offset_s` after the start of the
/// recording. `closest_approach_m` is the horizontal miss distance, so the
/// slant range at closest approach is `hypot(closest_approach_m,
/// altitude_m - ear_height_m)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AircraftPass {
    /// Callsign used in reports.
    pub callsign: String,
    /// True airspeed in metres per second.
    pub velocity_m_per_s: f64,
    /// Closest approach distance to the observer in metres.
    pub closest_approach_m: f64,
    /// Offset of the closest approach from the start of the recording, seconds.
    pub offset_s: f64,
    /// Geometric altitude in metres.
    pub altitude_m: f64,
    /// Emitted fundamental frequency of the propeller tone in hertz.
    ///
    /// Defaults to 100 Hz when a scenario omits it.
    #[serde(default = "default_fundamental_hz")]
    pub fundamental_hz: f64,
    /// Relative amplitude of harmonic 1, 2, ...; normalised to sum to one.
    ///
    /// Defaults to the fundamental alone when a scenario omits it.
    #[serde(default = "default_harmonics")]
    pub harmonics: Vec<f64>,
}

/// A complete simulated recording and its ground truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// Human-readable scenario name.
    pub name: String,
    /// Seed for every stochastic stage.
    pub seed: u64,
    /// Recording duration in seconds.
    pub duration_s: f64,
    /// Listener definition.
    pub observer: Observer,
    /// Aircraft passes in the recording.
    pub passes: Vec<AircraftPass>,
}

impl Scenario {
    /// Validate the scenario before synthesis.
    ///
    /// Returns [`SimError`] for an empty name, a non-positive duration, an
    /// invalid observer position, impossible pass geometry, a non-positive
    /// fundamental frequency, or a harmonic comb that is empty, negative or
    /// silent.
    pub fn validate(&self) -> Result<(), SimError> {
        if self.name.trim().is_empty() {
            return Err(SimError::EmptyName);
        }
        if !self.duration_s.is_finite() || self.duration_s <= 0.0 {
            return Err(SimError::InvalidDuration);
        }
        geodesy::ellipsoid::validate(self.observer.position)
            .map_err(|_| SimError::InvalidObserver)?;
        if !self.observer.ear_height_m.is_finite() {
            return Err(SimError::NotFinite);
        }
        if self.passes.is_empty() {
            return Err(SimError::NoPasses);
        }
        for (index, pass) in self.passes.iter().enumerate() {
            if !pass.velocity_m_per_s.is_finite()
                || !pass.closest_approach_m.is_finite()
                || !pass.offset_s.is_finite()
                || !pass.altitude_m.is_finite()
                || pass.velocity_m_per_s <= 0.0
                || pass.closest_approach_m < 0.0
                || !pass.fundamental_hz.is_finite()
                || pass.fundamental_hz <= 0.0
            {
                return Err(SimError::InvalidPass { index });
            }
            let total: f64 = pass.harmonics.iter().sum();
            if pass.harmonics.is_empty()
                || pass
                    .harmonics
                    .iter()
                    .any(|weight| !weight.is_finite() || *weight < 0.0)
                || !total.is_finite()
                || total <= 0.0
            {
                return Err(SimError::InvalidHarmonics { index });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../scenarios/example-single-pass.json");

    fn harmonic_pass() -> AircraftPass {
        AircraftPass {
            callsign: "TEST123".to_owned(),
            velocity_m_per_s: 90.0,
            closest_approach_m: 800.0,
            offset_s: 30.0,
            altitude_m: 600.0,
            fundamental_hz: 100.0,
            harmonics: vec![1.0, 0.5],
        }
    }

    fn scenario(pass: AircraftPass) -> Scenario {
        Scenario {
            name: "test".to_owned(),
            seed: 1,
            duration_s: 60.0,
            observer: Observer {
                position: LatLon::new(52.0, 13.0),
                ear_height_m: 1.7,
            },
            passes: vec![pass],
        }
    }

    #[test]
    fn example_scenario_parses_and_validates() {
        let parsed: Scenario = serde_json::from_str(EXAMPLE).expect("fixture is valid JSON");
        parsed.validate().expect("fixture is a valid scenario");
        assert_eq!(parsed.passes.len(), 1);
    }

    #[test]
    fn pass_without_tone_fields_gets_a_default_comb() {
        let json = r#"{
            "name": "defaults",
            "seed": 2,
            "duration_s": 10.0,
            "observer": { "position": { "lat_deg": 52.0, "lon_deg": 13.0 }, "ear_height_m": 1.7 },
            "passes": [{ "callsign": "X", "velocity_m_per_s": 90.0,
                         "closest_approach_m": 800.0, "offset_s": 5.0, "altitude_m": 600.0 }]
        }"#;
        let parsed: Scenario = serde_json::from_str(json).expect("older scenarios still load");
        let pass = parsed.passes.first().expect("one pass");
        assert!((pass.fundamental_hz - 100.0).abs() < 1e-12);
        assert_eq!(pass.harmonics, vec![1.0]);
    }

    #[test]
    fn scenario_rejects_silent_harmonic_comb() {
        let mut silent = harmonic_pass();
        silent.harmonics = vec![0.0, 0.0];
        assert!(matches!(
            scenario(silent).validate(),
            Err(SimError::InvalidHarmonics { index: 0 })
        ));

        let mut negative = harmonic_pass();
        negative.harmonics = vec![1.0, -0.5];
        assert!(matches!(
            scenario(negative).validate(),
            Err(SimError::InvalidHarmonics { index: 0 })
        ));

        let mut empty = harmonic_pass();
        empty.harmonics.clear();
        assert!(matches!(
            scenario(empty).validate(),
            Err(SimError::InvalidHarmonics { index: 0 })
        ));
    }

    #[test]
    fn scenario_rejects_a_non_positive_fundamental() {
        let mut pass = harmonic_pass();
        pass.fundamental_hz = 0.0;
        assert!(matches!(
            scenario(pass).validate(),
            Err(SimError::InvalidPass { index: 0 })
        ));
    }
}
