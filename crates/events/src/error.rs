//! Event-model failures.

/// A failure in an event observation or fit.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EventError {
    /// A value was NaN or infinite.
    #[error("value must be finite")]
    NotFinite,
    /// The harmonic index must be one or greater.
    #[error("harmonic index must be at least 1, got {0}")]
    InvalidHarmonic(u32),
    /// A quality score fell outside `[0, 1]`.
    #[error("quality score {0} is outside [0, 1]")]
    QualityOutOfRange(f64),
}
