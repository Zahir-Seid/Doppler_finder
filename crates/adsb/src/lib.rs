//! ADS-B archive ingest, an offline Parquet track store and interpolation.
//!
//! Requirements FR-DA-01 and FR-DA-04. The crate provides the track-source
//! trait and guarded archive clients, a partitioned Parquet cache that replays
//! tracks offline, and the interpolation used by the light-time solver.

pub mod error;
pub mod http;
pub mod interpolate;
pub mod opensky;
pub mod source;
pub mod stats;
pub mod track_store;
pub mod tracks;

pub use error::AdsbError;
pub use http::{HttpPolicy, HttpRequest, HttpResponse, HttpTransport, ReqwestTransport};
pub use interpolate::interpolate;
pub use opensky::{OPENSKY_HOST, OpenSkyClient, opensky_states_url, parse_opensky_states};
pub use source::{RegionQuery, TrackSource};
pub use stats::{SamplingStats, sampling_stats};
pub use track_store::{FileDigest, IngestRun, TRACK_STORE_SCHEMA_VERSION, TrackStore};
pub use tracks::{FLAG_INTERPOLATED, FLAG_MLAT, FlightTrack, TrackSample, TrackSourceKind};
