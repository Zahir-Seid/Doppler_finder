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
    /// A geodetic conversion failed.
    #[error("geodesy error: {0}")]
    Geodesy(#[from] geodesy::GeodesyError),
    /// The Parquet track store could not be read or written.
    #[error("track store error: {0}")]
    Store(String),
    /// A region or time window is malformed.
    #[error("invalid track query: {0}")]
    InvalidQuery(String),
    /// A request URL is malformed or fails the transport policy.
    #[error("invalid HTTP request: {0}")]
    InvalidRequest(String),
    /// A request host is not on the archive allow-list.
    #[error("host `{0}` is not allow-listed for this archive")]
    DisallowedHost(String),
    /// The archive returned a non-success HTTP status.
    #[error("archive returned HTTP status {status}")]
    HttpStatus {
        /// Response status code.
        status: u16,
    },
    /// The archive response exceeded the configured size cap.
    #[error("archive response exceeds the {limit_bytes}-byte cap")]
    ResponseTooLarge {
        /// Maximum body size in bytes.
        limit_bytes: usize,
    },
}
