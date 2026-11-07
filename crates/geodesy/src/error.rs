//! Geodesy failures.

/// A coordinate or sampler failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GeodesyError {
    /// Latitude is outside the closed interval `[-90, 90]` degrees.
    #[error("latitude {0} degrees is outside [-90, 90]")]
    LatitudeOutOfRange(f64),
    /// Longitude is outside the closed interval `[-180, 180]` degrees.
    #[error("longitude {0} degrees is outside [-180, 180]")]
    LongitudeOutOfRange(f64),
    /// A value was NaN or infinite where a finite number is required.
    #[error("value must be finite")]
    NotFinite,
    /// The terrain or geoid model has no coverage at the requested point.
    #[error("model has no coverage at ({lat_deg}, {lon_deg})")]
    NoCoverage {
        /// Requested latitude in degrees.
        lat_deg: f64,
        /// Requested longitude in degrees.
        lon_deg: f64,
    },
    /// A geoid grid is malformed or has inconsistent metadata.
    #[error("malformed geoid grid: {0}")]
    MalformedGeoidGrid(String),
    /// A terrain grid is malformed or has inconsistent metadata.
    #[error("malformed DEM grid: {0}")]
    MalformedDem(String),
    /// An observer ear height is negative or not finite.
    #[error("observer ear height {0} m must be finite and non-negative")]
    InvalidEarHeight(f64),
}
