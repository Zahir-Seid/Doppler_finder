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
    /// A sample rate or duration was non-finite.
    #[error("value must be finite")]
    NotFinite,
}
