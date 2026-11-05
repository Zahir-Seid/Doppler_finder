//! Simulator validation failures.

/// A failure in a scenario or synthesis configuration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SimError {
    /// The scenario name was empty.
    #[error("scenario name must not be empty")]
    EmptyName,
    /// The scenario duration was zero, negative or non-finite.
    #[error("scenario duration must be finite and greater than zero")]
    InvalidDuration,
    /// The scenario contained no aircraft passes.
    #[error("scenario must contain at least one aircraft pass")]
    NoPasses,
    /// The observer position was outside the geodetic domain.
    #[error("observer position is invalid")]
    InvalidObserver,
    /// One aircraft pass had non-finite or impossible geometry.
    #[error("aircraft pass {index} has invalid geometry")]
    InvalidPass {
        /// Zero-based index of the offending pass.
        index: usize,
    },
    /// One aircraft pass had no usable harmonic comb.
    #[error("aircraft pass {index} has an empty, negative or silent harmonic comb")]
    InvalidHarmonics {
        /// Zero-based index of the offending pass.
        index: usize,
    },
    /// One aircraft pass flew at or below the observer's ear height.
    #[error("aircraft pass {index} does not pass above the observer")]
    PassBelowObserver {
        /// Zero-based index of the offending pass.
        index: usize,
    },
    /// The requested peak amplitude was outside `(0, 1]`.
    #[error("peak amplitude {0} is outside (0, 1]")]
    InvalidAmplitude(f64),
    /// A sample rate or duration was non-finite.
    #[error("value must be finite")]
    NotFinite,
    /// The acoustics model rejected the geometry or the atmosphere.
    #[error("acoustics: {0}")]
    Acoustics(#[from] acoustics::AcousticsError),
}
