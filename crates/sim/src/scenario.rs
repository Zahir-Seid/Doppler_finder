//! Scenario model (`scenarios/*.json`, SDD section 7).

use core_types::LatLon;
use serde::{Deserialize, Serialize};

use crate::error::SimError;

/// The listener position and ear height.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Observer {
    /// Geodetic observer position.
    pub position: LatLon,
    /// Ear height above the terrain in metres.
    pub ear_height_m: f64,
}

/// One aircraft pass through the scenario.
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
    /// invalid observer position or impossible pass geometry.
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
            {
                return Err(SimError::InvalidPass { index });
            }
        }
        Ok(())
    }
}
