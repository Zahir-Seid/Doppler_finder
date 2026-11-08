//! Flight-track model (SDD section 9.2).

use core_types::UtcMillis;
use serde::{Deserialize, Serialize};

/// Bit 0 of [`TrackSample::flags`]: position came from MLAT.
pub const FLAG_MLAT: u8 = 1 << 0;
/// Bit 1 of [`TrackSample::flags`]: sample was interpolated rather than
/// received from the source.
pub const FLAG_INTERPOLATED: u8 = 1 << 1;

/// The archive a track came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackSourceKind {
    /// OpenSky Network.
    OpenSky,
    /// adsb.lol.
    AdsbLol,
}

impl TrackSourceKind {
    /// Stable lowercase name used in manifests and store partitions.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenSky => "opensky",
            Self::AdsbLol => "adsblol",
        }
    }

    /// Parse the stable lowercase name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "opensky" => Some(Self::OpenSky),
            "adsblol" => Some(Self::AdsbLol),
            _ => None,
        }
    }
}

impl std::fmt::Display for TrackSourceKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

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
    /// Barometric altitude in metres, when the source reported it.
    pub barometric_altitude_m: Option<f64>,
    /// Reported ground speed in metres per second, when available.
    pub ground_speed_m_per_s: Option<f64>,
    /// Reported track angle in degrees clockwise from north, when available.
    pub track_deg: Option<f64>,
    /// Reported vertical rate in metres per second, positive climbing.
    pub vertical_rate_m_per_s: Option<f64>,
    /// Reported staleness of the position in seconds: the gap between when the
    /// source last heard the aircraft and when the position was measured
    /// (FR-DA-01 latency). `None` when the source does not report it.
    #[serde(default)]
    pub latency_s: Option<f64>,
    /// Quality flags, for example [`FLAG_MLAT`].
    pub flags: u8,
}

/// One aircraft leg, ordered by sample time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightTrack {
    /// ICAO 24-bit address as lowercase hexadecimal.
    pub icao24: String,
    /// Callsign when the source provided one.
    pub callsign: Option<String>,
    /// ICAO aircraft type code, when known.
    pub type_code: Option<String>,
    /// Archive this track came from.
    pub source: TrackSourceKind,
    /// Samples in increasing time order.
    pub samples: Vec<TrackSample>,
}

impl FlightTrack {
    /// Check that samples are ordered and usable by interpolation.
    ///
    /// Returns [`crate::AdsbError::InvalidTrack`] for an empty track,
    /// non-finite coordinates or altitudes, out-of-range coordinates, a
    /// negative speed, or decreasing times.
    pub fn validate(&self) -> Result<(), crate::AdsbError> {
        let mut previous: Option<UtcMillis> = None;
        for sample in &self.samples {
            if !sample.lat_deg.is_finite()
                || !sample.lon_deg.is_finite()
                || !sample.geometric_altitude_m.is_finite()
                || sample
                    .barometric_altitude_m
                    .is_some_and(|value| !value.is_finite())
                || sample
                    .ground_speed_m_per_s
                    .is_some_and(|value| !value.is_finite() || value < 0.0)
                || sample.track_deg.is_some_and(|value| !value.is_finite())
                || sample
                    .vertical_rate_m_per_s
                    .is_some_and(|value| !value.is_finite())
                || sample.latency_s.is_some_and(|value| !value.is_finite())
            {
                return Err(crate::AdsbError::InvalidTrack(
                    "coordinate, altitude, speed or rate is not finite".to_owned(),
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
