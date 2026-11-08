//! Flight-track source abstraction.

use core_types::UtcMillis;

use crate::error::AdsbError;
use crate::tracks::FlightTrack;

/// A region and time window to fetch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionQuery {
    /// Southern boundary in degrees.
    pub min_lat_deg: f64,
    /// Western boundary in degrees.
    pub min_lon_deg: f64,
    /// Northern boundary in degrees.
    pub max_lat_deg: f64,
    /// Eastern boundary in degrees.
    pub max_lon_deg: f64,
    /// Start of the window, inclusive.
    pub from: UtcMillis,
    /// End of the window, inclusive.
    pub to: UtcMillis,
}

impl RegionQuery {
    /// Check that the box is finite, ordered and inside WGS84 bounds.
    ///
    /// The box must not cross the antimeridian: `min_lon_deg` must be less
    /// than or equal to `max_lon_deg`, and both must lie in `[-180, 180]`.
    ///
    /// # Errors
    ///
    /// Returns [`AdsbError::InvalidQuery`] for a non-finite coordinate, a
    /// latitude outside `[-90, 90]`, a longitude outside `[-180, 180]`, a
    /// reversed box, or a window whose end precedes its start.
    pub fn validate(&self) -> Result<(), AdsbError> {
        for (name, value) in [
            ("min_lat_deg", self.min_lat_deg),
            ("min_lon_deg", self.min_lon_deg),
            ("max_lat_deg", self.max_lat_deg),
            ("max_lon_deg", self.max_lon_deg),
        ] {
            if !value.is_finite() {
                return Err(AdsbError::InvalidQuery(format!("{name} must be finite")));
            }
        }
        if !(-90.0..=90.0).contains(&self.min_lat_deg)
            || !(-90.0..=90.0).contains(&self.max_lat_deg)
        {
            return Err(AdsbError::InvalidQuery(
                "latitude bounds must lie in [-90, 90]".to_owned(),
            ));
        }
        if !(-180.0..=180.0).contains(&self.min_lon_deg)
            || !(-180.0..=180.0).contains(&self.max_lon_deg)
        {
            return Err(AdsbError::InvalidQuery(
                "longitude bounds must lie in [-180, 180]".to_owned(),
            ));
        }
        if self.min_lat_deg > self.max_lat_deg || self.min_lon_deg > self.max_lon_deg {
            return Err(AdsbError::InvalidQuery(
                "minimum bounds must not exceed maximum bounds".to_owned(),
            ));
        }
        if self.from.0 > self.to.0 {
            return Err(AdsbError::InvalidQuery(
                "window start must not follow its end".to_owned(),
            ));
        }
        Ok(())
    }
}

/// A source of archived ADS-B tracks.
///
/// Implementations are synchronous; network and filesystem clients run inside
/// the ingest task, not on the async executor directly.
pub trait TrackSource: std::fmt::Debug + Send + Sync {
    /// Return tracks for a region and time window.
    fn tracks(&self, query: &RegionQuery) -> Result<Vec<FlightTrack>, AdsbError>;
}
