//! ADS-B archive ingest, cached tracks and interpolation.
//!
//! Requirements FR-DA-01 and FR-DA-04. Network clients and the Parquet store
//! arrive with the ingest task; this crate fixes the source trait, the track
//! model and the interpolation used by the light-time solver.

pub mod error;
pub mod http;
pub mod interpolate;
pub mod opensky;
pub mod source;
pub mod stats;
pub mod tracks;

pub use error::AdsbError;
pub use http::{HttpPolicy, HttpRequest, HttpResponse, HttpTransport, ReqwestTransport};
pub use interpolate::interpolate;
pub use opensky::{OPENSKY_HOST, OpenSkyClient, opensky_states_url, parse_opensky_states};
pub use source::{RegionQuery, TrackSource};
pub use stats::{SamplingStats, sampling_stats};
pub use tracks::{FlightTrack, TrackSample};
