//! Shared units, two-clock time types, identifiers and cancellation primitives.
//!
//! This crate has no I/O and no async runtime. It is the bottom of the
//! dependency graph in `docs/SDD.md` section 3, so every other crate may depend
//! on it.

pub mod clock;
pub mod ctx;
pub mod error;
pub mod ids;
pub mod time;
pub mod units;

pub use clock::{Clock, SystemClock};
pub use ctx::{Cancel, CancelReason, JobCtx};
pub use error::CoreError;
pub use ids::{EventId, JobId, RecordingId};
pub use time::{AudioSeconds, UtcMillis};
pub use units::{
    Hertz, LatLon, LatitudeDegrees, LongitudeDegrees, Metres, MetresPerSecond, Radians, Seconds,
};
