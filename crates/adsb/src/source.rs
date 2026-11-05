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

/// A source of archived ADS-B tracks.
///
/// Implementations are synchronous; network and filesystem clients run inside
/// the ingest task, not on the async executor directly.
pub trait TrackSource: std::fmt::Debug + Send + Sync {
    /// Return tracks for a region and time window.
    fn tracks(&self, query: &RegionQuery) -> Result<Vec<FlightTrack>, AdsbError>;
}
