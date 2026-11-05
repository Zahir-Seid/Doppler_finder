//! Acoustics and propagation failures.

/// A failure in the Doppler or atmosphere model.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AcousticsError {
    /// One or more geometry values were NaN or infinite.
    #[error("geometry values must be finite")]
    NotFinite,
    /// Closest approach must not be negative.
    #[error("closest approach {0} m must be non-negative")]
    NegativeDistance(f64),
    /// The aircraft speed must be below the speed of light.
    #[error("speed {0} m/s must be below the speed of light")]
    SpeedTooHigh(f64),
    /// The closed-form solution has no real retarded time.
    #[error("no real retarded time for the requested geometry")]
    NoRealSolution,
}
