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
    /// The propagation speed must be positive and finite.
    #[error("sound speed {0} m/s must be positive and finite")]
    InvalidSoundSpeed(f64),
    /// The source moves at least as fast as the medium propagates, so the
    /// closed form of SDD section 4 has no physical root.
    #[error("speed {0} m/s is not below the speed of sound")]
    SupersonicSource(f64),
    /// No emission time satisfies the light-time equation for this geometry.
    #[error("no real retarded time for the requested geometry")]
    NoRealSolution,
}
