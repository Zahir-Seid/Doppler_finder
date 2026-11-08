//! OpenSky archive client (FR-DA-01).
//!
//! The client requests the `/api/states/all` endpoint for a bounding box and
//! converts state vectors into [`FlightTrack`]s. It is synchronous because
//! every library crate is synchronous (SDD section 3), and it only ever talks
//! through an [`HttpTransport`] guarded by an [`HttpPolicy`]. Tests pass a
//! recorded response, never a live socket.
//!
//! OpenSky returns one snapshot per request, so the window start is sent as
//! `time` and states outside the requested window are dropped after parsing.

use std::collections::BTreeMap;
use std::fmt;

use core_types::UtcMillis;
use serde_json::Value;

use crate::error::AdsbError;
use crate::http::{HttpPolicy, HttpRequest, HttpTransport};
use crate::source::{RegionQuery, TrackSource};
use crate::tracks::{FlightTrack, TrackSample};

/// OpenSky host that the default policy must allow.
pub const OPENSKY_HOST: &str = "opensky-network.org";

/// OpenSky state-vector endpoint.
const STATES_URL: &str = "https://opensky-network.org/api/states/all";

/// A client for the OpenSky `/api/states/all` endpoint.
pub struct OpenSkyClient<T> {
    transport: T,
    policy: HttpPolicy,
    bearer_token: Option<String>,
}

impl<T> fmt::Debug for OpenSkyClient<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenSkyClient")
            .field("authenticated", &self.bearer_token.is_some())
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl<T: HttpTransport> OpenSkyClient<T> {
    /// Build a client over a transport and its request policy.
    ///
    /// `bearer_token` is an OpenSky OAuth2 access token. Token acquisition and
    /// refresh belong to the caller; passing `None` uses anonymous access.
    pub fn new(transport: T, policy: HttpPolicy, bearer_token: Option<String>) -> Self {
        Self {
            transport,
            policy,
            bearer_token,
        }
    }

    /// The request policy applied to every fetch.
    pub fn policy(&self) -> &HttpPolicy {
        &self.policy
    }

    /// The transport this client fetches through.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Fetch and parse tracks for a region and window.
    fn fetch_tracks(&self, query: &RegionQuery) -> Result<Vec<FlightTrack>, AdsbError> {
        query.validate()?;
        let request = HttpRequest::new(opensky_states_url(query)?, self.bearer_token.clone());
        let response = self.policy.fetch(&self.transport, &request)?;
        let payload = std::str::from_utf8(response.body())
            .map_err(|error| AdsbError::Parse(format!("response is not UTF-8: {error}")))?;
        parse_opensky_states(payload, query)
    }
}

impl<T: HttpTransport> TrackSource for OpenSkyClient<T> {
    fn tracks(&self, query: &RegionQuery) -> Result<Vec<FlightTrack>, AdsbError> {
        self.fetch_tracks(query)
    }
}

/// Build the OpenSky state-vector URL for a query.
///
/// The bounding-box bounds use decimal degrees and `time` uses Unix seconds,
/// as the OpenSky REST API expects.
///
/// # Errors
///
/// Returns [`AdsbError::InvalidQuery`] when the region or window is invalid.
pub fn opensky_states_url(query: &RegionQuery) -> Result<String, AdsbError> {
    query.validate()?;
    let mut url = reqwest::Url::parse(STATES_URL)
        .map_err(|error| AdsbError::InvalidRequest(format!("invalid base URL: {error}")))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("lamin", &query.min_lat_deg.to_string());
        pairs.append_pair("lomin", &query.min_lon_deg.to_string());
        pairs.append_pair("lamax", &query.max_lat_deg.to_string());
        pairs.append_pair("lomax", &query.max_lon_deg.to_string());
        pairs.append_pair("time", &(query.from.0.div_euclid(1000)).to_string());
    }
    Ok(url.into())
}

/// Parse an OpenSky states response into tracks.
///
/// State vectors without a position, a geometric altitude, or a timestamp are
/// skipped because [`TrackSample`] cannot represent them. Surface reports
/// (`on_ground`), malformed rows and samples outside `query`'s window are also
/// skipped. Tracks are ordered by ICAO24 and samples by time.
///
/// # Errors
///
/// Returns [`AdsbError::InvalidQuery`] for a bad query and
/// [`AdsbError::Parse`] for invalid JSON, a missing or non-array `states`
/// field, a malformed state vector, or a track that fails validation.
pub fn parse_opensky_states(
    payload: &str,
    query: &RegionQuery,
) -> Result<Vec<FlightTrack>, AdsbError> {
    query.validate()?;
    let root: Value = serde_json::from_str(payload)
        .map_err(|error| AdsbError::Parse(format!("invalid JSON: {error}")))?;
    let states = root
        .get("states")
        .ok_or_else(|| AdsbError::Parse("response has no `states` field".to_owned()))?;
    if states.is_null() {
        return Ok(Vec::new());
    }
    let rows = states
        .as_array()
        .ok_or_else(|| AdsbError::Parse("`states` is not an array".to_owned()))?;

    let mut tracks: BTreeMap<String, FlightTrack> = BTreeMap::new();
    for row in rows {
        let Some(parsed) = parse_state(row, query)? else {
            continue;
        };
        let entry = tracks
            .entry(parsed.icao24.clone())
            .or_insert_with(|| FlightTrack {
                icao24: parsed.icao24,
                callsign: None,
                samples: Vec::new(),
            });
        if entry.callsign.is_none() {
            entry.callsign = parsed.callsign;
        }
        entry.samples.push(parsed.sample);
    }

    let mut result = Vec::with_capacity(tracks.len());
    for (_, mut track) in tracks {
        track.samples.sort_by_key(|sample| sample.utc.0);
        track.validate()?;
        result.push(track);
    }
    Ok(result)
}

/// One usable row from an OpenSky state-vector array.
struct ParsedState {
    icao24: String,
    callsign: Option<String>,
    sample: TrackSample,
}

/// Convert one state vector, returning `None` when it is unusable.
fn parse_state(row: &Value, query: &RegionQuery) -> Result<Option<ParsedState>, AdsbError> {
    let fields = row
        .as_array()
        .ok_or_else(|| AdsbError::Parse("state vector is not an array".to_owned()))?;
    if fields.is_empty() {
        return Err(AdsbError::Parse("state vector is empty".to_owned()));
    }

    let icao24 = text_at(row, 0)
        .ok_or_else(|| AdsbError::Parse("state vector has no ICAO24".to_owned()))?
        .trim()
        .to_ascii_lowercase();
    if icao24.len() != 6 || !icao24.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AdsbError::Parse(format!(
            "ICAO24 `{icao24}` is not six hexadecimal digits"
        )));
    }

    let Some(seconds) = integer_at(row, 3).or_else(|| integer_at(row, 4)) else {
        return Ok(None);
    };
    let utc_ms = seconds
        .checked_mul(1000)
        .ok_or_else(|| AdsbError::Parse(format!("timestamp {seconds} overflows milliseconds")))?;
    if utc_ms < query.from.0 || utc_ms > query.to.0 {
        return Ok(None);
    }

    let (Some(lon_deg), Some(lat_deg)) = (number_at(row, 5), number_at(row, 6)) else {
        return Ok(None);
    };
    if !(-90.0..=90.0).contains(&lat_deg) || !(-180.0..=180.0).contains(&lon_deg) {
        return Err(AdsbError::Parse(
            "state vector coordinate is outside WGS84 bounds".to_owned(),
        ));
    }
    if bool_at(row, 8).unwrap_or(false) {
        return Ok(None);
    }
    let Some(geometric_altitude_m) = number_at(row, 13) else {
        return Ok(None);
    };

    let ground_speed_m_per_s = optional_non_negative(row, 9, "ground speed")?;
    let track_deg = optional_track(row, 10)?;
    let callsign = text_at(row, 1)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    Ok(Some(ParsedState {
        icao24,
        callsign,
        sample: TrackSample {
            utc: UtcMillis(utc_ms),
            lat_deg,
            lon_deg,
            geometric_altitude_m,
            ground_speed_m_per_s,
            track_deg,
        },
    }))
}

/// Read a JSON array element.
fn value_at(row: &Value, index: usize) -> Option<&Value> {
    row.as_array()?.get(index)
}

/// Read a non-empty string field.
fn text_at(row: &Value, index: usize) -> Option<&str> {
    value_at(row, index)?.as_str()
}

/// Read a finite numeric field.
fn number_at(row: &Value, index: usize) -> Option<f64> {
    value_at(row, index)?
        .as_f64()
        .filter(|value| value.is_finite())
}

/// Read an integer field, accepting an unsigned representation.
fn integer_at(row: &Value, index: usize) -> Option<i64> {
    let value = value_at(row, index)?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
}

/// Read a boolean field.
fn bool_at(row: &Value, index: usize) -> Option<bool> {
    value_at(row, index)?.as_bool()
}

/// Read an optional non-negative finite value.
fn optional_non_negative(row: &Value, index: usize, field: &str) -> Result<Option<f64>, AdsbError> {
    match value_at(row, index) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => {
            let number = value
                .as_f64()
                .ok_or_else(|| AdsbError::Parse(format!("`{field}` is not a number: {value}")))?;
            if !number.is_finite() || number < 0.0 {
                return Err(AdsbError::Parse(format!(
                    "`{field}` must be finite and non-negative"
                )));
            }
            Ok(Some(number))
        }
    }
}

/// Read an optional track angle, wrapped into `[0, 360)`.
fn optional_track(row: &Value, index: usize) -> Result<Option<f64>, AdsbError> {
    match value_at(row, index) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => {
            let number = value
                .as_f64()
                .ok_or_else(|| AdsbError::Parse(format!("`track` is not a number: {value}")))?;
            if !number.is_finite() {
                return Err(AdsbError::Parse("`track` must be finite".to_owned()));
            }
            Ok(Some(number.rem_euclid(360.0)))
        }
    }
}
