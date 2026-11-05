//! ADS-B ingest and store failures.

/// A failure while reading or caching flight tracks.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AdsbError {
    /// A transport or filesystem error.
    #[error("I/O error: {0}")]
    Io(String),
    /// The archive payload could not be parsed.
    #[error("could not parse ADS-B payload: {0}")]
    Parse(String),
    /// No source covers the requested region and date.
    #[error("no ADS-B coverage for the requested region and date")]
    NoCoverage,
    /// A track contained invalid coordinates or times.
    #[error("invalid track data: {0}")]
    InvalidTrack(String),
}
