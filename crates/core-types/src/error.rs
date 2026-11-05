//! Error type shared by the unit and time newtypes.

/// A validation failure in a shared type.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// A latitude fell outside the closed interval `[-90, 90]`.
    #[error("latitude {0} degrees is outside [-90, 90]")]
    LatitudeOutOfRange(f64),
    /// A longitude fell outside the closed interval `[-180, 180]`.
    #[error("longitude {0} degrees is outside [-180, 180]")]
    LongitudeOutOfRange(f64),
    /// A value was NaN or infinite where a finite number is required.
    #[error("value must be finite")]
    NotFinite,
}
