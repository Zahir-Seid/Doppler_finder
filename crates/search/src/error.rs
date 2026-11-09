//! Search and verdict failures.

/// A failure in localization or verdict construction.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SearchError {
    /// A value was NaN or infinite.
    #[error("value must be finite")]
    NotFinite,
    /// Verdict thresholds were not ordered or outside `[0, 1]`.
    #[error("verdict thresholds must satisfy 0 <= ambiguous_min <= locatable_min <= 1")]
    InvalidThresholds,
    /// A posterior mass fell outside `[0, 1]`.
    #[error("posterior mass {0} is outside [0, 1]")]
    PosteriorOutOfRange(f64),
    /// An ADS-B track or query was unusable for prediction.
    #[error("track error: {0}")]
    Adsb(String),
    /// A geodesy lookup (terrain, geoid, ellipsoid) failed.
    #[error("geodesy error: {0}")]
    Geodesy(String),
}
