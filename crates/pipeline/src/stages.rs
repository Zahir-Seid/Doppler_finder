//! Pipeline stage identifiers (SDD section 10.1).

use serde::{Deserialize, Serialize};

/// A stage of one analysis job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Stage {
    /// Locate and read the input media.
    Ingest,
    /// Decode, band-limit and decimate the audio.
    Decode,
    /// Detect Doppler events.
    Detect,
    /// Fit per-event parameters.
    Fit,
    /// Match events to archived flights.
    Match,
    /// Fuse hypotheses and refine the position.
    Localize,
    /// Build figures and the result payload.
    Render,
}

impl Stage {
    /// Stable stage name used in job events and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ingest => "ingest",
            Self::Decode => "decode",
            Self::Detect => "detect",
            Self::Fit => "fit",
            Self::Match => "match",
            Self::Localize => "localize",
            Self::Render => "render",
        }
    }

    /// Stage order for progress reporting.
    pub const ALL: &[Self] = &[
        Self::Ingest,
        Self::Decode,
        Self::Detect,
        Self::Fit,
        Self::Match,
        Self::Localize,
        Self::Render,
    ];
}
