//! Simulator validation and output failures.

/// A failure in a scenario, a synthesis configuration, an injected error
/// profile, a Cramér–Rao computation or an output file.
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
    /// The absorption coefficient was negative or non-finite.
    #[error("absorption coefficient {0} dB/km is negative or non-finite")]
    InvalidAbsorption(f64),
    /// The noise level was non-finite.
    #[error("noise level {0} dB is non-finite")]
    InvalidNoise(f64),
    /// The output sample rate could not be written to a WAV file.
    #[error("sample rate {0} Hz cannot be written to a 16-bit PCM WAV file")]
    InvalidSampleRate(f64),
    /// A Fisher information matrix was singular, so no bound exists.
    #[error("Fisher information matrix is singular")]
    SingularFisher,
    /// One field of an injected-error profile was impossible.
    #[error("error profile field {field} = {value} is not allowed")]
    InvalidProfile {
        /// Name of the offending field.
        field: &'static str,
        /// The rejected value.
        value: f64,
    },
    /// A feed held fewer than two points for the requested callsign.
    #[error("feed has too few points for the requested callsign")]
    EmptyFeed,
    /// The scenario has no pass with the requested callsign.
    #[error("scenario has no pass for callsign {0}")]
    UnknownCallsign(String),
    /// A sample rate, duration or window was non-finite or empty.
    #[error("value must be finite")]
    NotFinite,
    /// The acoustics model rejected the geometry or the atmosphere.
    #[error("acoustics: {0}")]
    Acoustics(#[from] acoustics::AcousticsError),
    /// The geodesy crate rejected a position.
    #[error("geodesy: {0}")]
    Geodesy(#[from] geodesy::GeodesyError),
    /// A file could not be created or written.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The WAV encoder rejected a write.
    #[error("wav: {0}")]
    Wav(#[from] hound::Error),
    /// A truth file could not be serialized or parsed.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
