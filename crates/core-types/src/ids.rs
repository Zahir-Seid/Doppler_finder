//! Time-ordered identifiers.
//!
//! Every identifier is a UUIDv7 string so that it sorts by creation time and
//! never exposes a sequential count. The database stores the canonical text
//! form (SDD section 9.3).

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifier of an analysis job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(Uuid);

impl JobId {
    /// Create a new time-ordered identifier.
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }

    /// Wrap an existing UUID.
    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    /// Return the underlying UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Identifier of a detected Doppler event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(Uuid);

impl EventId {
    /// Create a new time-ordered identifier.
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }

    /// Wrap an existing UUID.
    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    /// Return the underlying UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Identifier of an uploaded or referenced recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecordingId(Uuid);

impl RecordingId {
    /// Create a new time-ordered identifier.
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }

    /// Wrap an existing UUID.
    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    /// Return the underlying UUID.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for RecordingId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}
