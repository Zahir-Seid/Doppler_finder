//! DSP configuration failures.

/// A failure in a DSP configuration or stage.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DspError {
    /// A window or frame length was zero.
    #[error("length must be greater than zero")]
    ZeroLength,
    /// The hop length was zero or larger than the frame length.
    #[error("hop length {hop} is invalid for frame length {frame}")]
    InvalidHop {
        /// Requested hop length in samples.
        hop: usize,
        /// Requested frame length in samples.
        frame: usize,
    },
    /// A sample rate or other scalar was NaN or infinite.
    #[error("value must be finite")]
    NotFinite,
    /// The requested frame does not fit in the input.
    #[error("input of {samples} samples is shorter than frame length {frame}")]
    InputTooShort {
        /// Number of samples supplied.
        samples: usize,
        /// Required frame length in samples.
        frame: usize,
    },
}
