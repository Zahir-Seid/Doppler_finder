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
    /// Two arrays or slices that must line up did not.
    #[error("length {actual} does not match the expected {expected}")]
    LengthMismatch {
        /// Required length.
        expected: usize,
        /// Actual length.
        actual: usize,
    },
    /// Two tracks claimed the same harmonic index.
    #[error("harmonic {0} appears more than once")]
    DuplicateHarmonic(u32),
    /// A harmonic track held fewer points than the fit requires.
    #[error("harmonic {harmonic} has {actual} points, at least {needed} are needed")]
    InsufficientTrackPoints {
        /// Harmonic index.
        harmonic: u32,
        /// Minimum points per harmonic.
        needed: usize,
        /// Points the track actually has.
        actual: usize,
    },
    /// The whole fit held no more observations than free parameters.
    #[error("the fit needs more than {needed} observations, got {actual}")]
    InsufficientObservations {
        /// Number of free parameters.
        needed: usize,
        /// Observations supplied.
        actual: usize,
    },
    /// A frequency variance was zero, negative or non-finite.
    #[error("frequency variance {0} is not positive and finite")]
    InvalidVariance(f64),
    /// The normal equations carried no information about one or more parameters.
    #[error("the fit is singular: the observations do not constrain every parameter")]
    SingularFit,
    /// The emission-time model has no real solution for the fitted geometry.
    #[error("the Doppler model has no real solution for this geometry")]
    ModelUndefined,
    /// A spectral-map failure while rebuilding a suppressed view.
    #[error("spectral map failure: {0}")]
    Dsp(#[from] dsp::DspError),
}
