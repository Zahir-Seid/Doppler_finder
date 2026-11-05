//! Geoid undulation sampler.

use core_types::LatLon;

use crate::error::GeodesyError;

/// A geoid model converting ellipsoidal height to orthometric height.
pub trait GeoidModel: std::fmt::Debug + Send + Sync {
    /// Return the geoid undulation in metres at a geodetic position.
    ///
    /// The geometric altitude used by the estimator is the orthometric
    /// altitude plus this undulation (SDD section 6.1).
    fn undulation_m(&self, position: LatLon) -> Result<f64, GeodesyError>;
}
