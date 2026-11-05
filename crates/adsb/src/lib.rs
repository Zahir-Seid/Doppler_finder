//! ADS-B archive ingest, cached tracks and interpolation.
//!
//! Requirements FR-DA-01 and FR-DA-04. Network clients and the Parquet store
//! arrive with the ingest task; this crate fixes the source trait, the track
//! model and the interpolation used by the light-time solver.

pub mod error;
pub mod interpolate;
pub mod source;
pub mod tracks;

pub use error::AdsbError;
pub use interpolate::interpolate;
pub use source::{RegionQuery, TrackSource};
pub use tracks::{FlightTrack, TrackSample};
