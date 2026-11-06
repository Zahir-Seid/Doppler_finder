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
    /// A grid bound or step is unusable.
    #[error("invalid grid {min}..{max} step {step}")]
    InvalidGrid {
        /// Lower bound of the grid.
        min: f64,
        /// Upper bound of the grid.
        max: f64,
        /// Spacing between values.
        step: f64,
    },
    /// A grid would generate too many values to search.
    #[error("grid generates {0} values, which is over the search budget")]
    GridTooLarge(usize),
    /// A rate, shift or other tunable fell outside its usable range.
    #[error("value {0} is outside its usable range")]
    InvalidRate(f64),
    /// The z-map has no frames or no bins.
    #[error("the z-map is empty")]
    EmptyMap,
    /// The map is too short to hold both halves of a template.
    #[error("map has {actual} frames, at least {needed} are needed")]
    InsufficientFrames {
        /// Frames both halves together require.
        needed: usize,
        /// Frames the map actually has.
        actual: usize,
    },
    /// No hypothesis produced a usable curve.
    #[error("no hypothesis fits inside the map")]
    NoHypothesis,
    /// A length or count was zero where at least one is required.
    #[error("length must be at least one")]
    ZeroLength,
}
