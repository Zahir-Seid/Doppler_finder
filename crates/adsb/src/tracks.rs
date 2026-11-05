//! Flight-track model (SDD section 9.2).

use core_types::UtcMillis;
use serde::{Deserialize, Serialize};

/// One sampled aircraft state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrackSample {
    /// Sample time in UTC milliseconds.
    pub utc: UtcMillis,
    /// WGS84 latitude in degrees.
    pub lat_deg: f64,
    /// WGS84 longitude in degrees.
    pub lon_deg: f64,
    /// Geometric altitude in metres, after the geoid correction.
    pub geometric_altitude_m: f64,
    /// Reported ground speed in metres per second, when available.
    pub ground_speed_m_per_s: Option<f64>,
    /// Reported track angle in degrees clockwise from north, when available.
    pub track_deg: Option<f64>,
}

/// One aircraft leg, ordered by sample time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightTrack {
    /// ICAO 24-bit address as lowercase hexadecimal.
    pub icao24: String,
    /// Callsign when the source provided one.
    pub callsign: Option<String>,
    /// Samples in increasing time order.
    pub samples: Vec<TrackSample>,
}

impl FlightTrack {
    /// Check that samples are ordered and usable by interpolation.
    ///
    /// Returns [`crate::AdsbError::InvalidTrack`] for an empty track,
    /// non-finite coordinates or decreasing times.
    pub fn validate(&self) -> Result<(), crate::AdsbError> {
        let mut previous: Option<UtcMillis> = None;
        for sample in &self.samples {
            if !sample.lat_deg.is_finite()
                || !sample.lon_deg.is_finite()
                || !sample.geometric_altitude_m.is_finite()
            {
                return Err(crate::AdsbError::InvalidTrack(
                    "coordinate or altitude is not finite".to_owned(),
                ));
            }
            if !(-90.0..=90.0).contains(&sample.lat_deg)
                || !(-180.0..=180.0).contains(&sample.lon_deg)
            {
                return Err(crate::AdsbError::InvalidTrack(
                    "coordinate outside its valid range".to_owned(),
                ));
            }
            if let Some(previous_time) = previous
                && sample.utc < previous_time
            {
                return Err(crate::AdsbError::InvalidTrack(
                    "samples are not in time order".to_owned(),
                ));
            }
            previous = Some(sample.utc);
        }
        if self.samples.is_empty() {
            return Err(crate::AdsbError::InvalidTrack("track is empty".to_owned()));
        }
        Ok(())
    }
}
