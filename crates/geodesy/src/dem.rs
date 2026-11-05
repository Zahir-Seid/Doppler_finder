//! Terrain elevation sampler.

use core_types::LatLon;

use crate::error::GeodesyError;

/// A public digital elevation model.
///
/// Implementations wrap a local DEM cache. The trait is synchronous because
/// all library crates are synchronous (SDD section 3).
pub trait ElevationModel: std::fmt::Debug + Send + Sync {
    /// Return terrain elevation above the ellipsoid in metres.
    ///
    /// Returns [`GeodesyError::NoCoverage`] outside the model's bounds.
    fn elevation_m(&self, position: LatLon) -> Result<f64, GeodesyError>;
}
