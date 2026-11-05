//! Terrain elevation sampler.

use core_types::LatLon;

use crate::error::GeodesyError;

/// A public digital elevation model.
///
/// Implementations wrap a local DEM cache. The trait is synchronous because
/// all library crates are synchronous (SDD section 3).
pub trait ElevationModel: std::fmt::Debug + Send + Sync {
    /// Return terrain elevation in metres above the geoid (orthometric
    /// height), which is the datum public DEMs publish.
    ///
    /// Ellipsoidal height is this value plus the [`crate::GeoidModel`]
    /// undulation at the same point; do not feed it to the ellipsoid as if it
    /// were already ellipsoidal, or the geoid correction is applied twice
    /// (SDD section 6.1).
    ///
    /// Returns [`GeodesyError::NoCoverage`] outside the model's bounds.
    fn elevation_m(&self, position: LatLon) -> Result<f64, GeodesyError>;
}
