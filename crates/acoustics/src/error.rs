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
    /// A temperature at or below absolute zero, or a non-finite one.
    #[error("temperature {0} K must be above absolute zero")]
    InvalidTemperature(f64),
    /// A pressure that is not positive.
    #[error("pressure {0} Pa must be positive")]
    InvalidPressure(f64),
    /// A humidity fraction outside `[0, 1]`.
    #[error("relative humidity {0} must lie in [0, 1]")]
    InvalidHumidity(f64),
    /// Harmonics are numbered from one; there is no zeroth harmonic.
    #[error("harmonic number must be at least 1")]
    ZeroHarmonic,
    /// No emission time satisfies the light-time equation for this geometry.
    #[error("no real retarded time for the requested geometry")]
    NoRealSolution,
}
