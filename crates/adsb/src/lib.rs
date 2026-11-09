//! ADS-B archive ingest, an offline Parquet track store and interpolation.
//!
//! Requirements FR-DA-01 and FR-DA-04. The crate provides the track-source
//! trait and guarded archive clients, a partitioned Parquet cache that replays
//! tracks offline, Hermite interpolation for the light-time solver, derived
//! legs and closest-approach prediction.

pub mod closest;
pub mod coverage;
pub mod dump;
pub mod error;
pub mod http;
pub mod interpolate;
pub mod legs;
pub mod opensky;
pub mod source;
pub mod stats;
pub mod track_store;
pub mod tracks;
pub mod weather;

pub use closest::{ClosestApproach, closest_approach};
pub use coverage::{CoverageReport, MAX_SUPPORTED_MEDIAN_GAP_S, RegionSupport, assess_coverage};
pub use dump::{AdsbLolArchive, DumpSpec, DumpStats, ingest_path, ingest_reader};
pub use error::AdsbError;
pub use http::{HttpPolicy, HttpRequest, HttpResponse, HttpTransport, ReqwestTransport};
pub use interpolate::{TrackInterpolator, interpolate};
pub use legs::{EngineClass, Leg, legs_from_tracks};
pub use opensky::{OPENSKY_HOST, OpenSkyClient, opensky_states_url, parse_opensky_states};
pub use source::{RegionQuery, TrackSource};
pub use stats::{SamplingStats, sampling_stats};
pub use track_store::{FileDigest, IngestRun, TRACK_STORE_SCHEMA_VERSION, TrackStore};
pub use tracks::{
    FLAG_GEOM_FROM_BARO, FLAG_INTERPOLATED, FLAG_MLAT, FLAG_NEW_LEG, FLAG_NEW_LEG_AFTER,
    FLAG_STALE, FLAG_TISB, FlightTrack, TrackSample, TrackSourceKind,
};
pub use weather::{
    ARCHIVE_API_URL, FORECAST_API_URL, HIST_FORECAST_API_URL, OPEN_METEO_HOSTS, WeatherArchive,
    fetch_weather_payload, open_meteo_url, parse_open_meteo_response,
};
