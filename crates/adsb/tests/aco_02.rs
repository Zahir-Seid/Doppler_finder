//! FR-DA-03 / ACO-02: Open-Meteo fetch adapter producing the normalized
//! payload `acoustics::weather` parses. All tests replay a recorded response
//! through a scripted transport; no test opens a socket.
#![allow(clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use acoustics::weather::{atmosphere_for_window, parse_weather};
use adsb::{
    ARCHIVE_API_URL, AdsbError, HttpPolicy, HttpRequest, HttpResponse, HttpTransport,
    OPEN_METEO_HOSTS, WeatherArchive, fetch_weather_payload, open_meteo_url,
    parse_open_meteo_response,
};

/// Recorded-shape Open-Meteo archive response: two complete hourly rows and
/// one row with a null wind direction (must be skipped, not fabricated).
const FIXTURE: &str = r#"{
  "latitude": 52.52,
  "longitude": 13.405,
  "generationtime_ms": 1.2,
  "utc_offset_seconds": 0,
  "timezone": "GMT",
  "timezone_abbreviation": "GMT",
  "elevation": 38.0,
  "hourly_units": {
    "time": "unixtime",
    "temperature_2m": "°C",
    "relative_humidity_2m": "%",
    "surface_pressure": "hPa",
    "wind_speed_10m": "m/s",
    "wind_direction_10m": "°"
  },
  "hourly": {
    "time": [1705312800, 1705316400, 1705320000],
    "temperature_2m": [5.0, 4.0, null],
    "relative_humidity_2m": [80.0, 85.0, 90.0],
    "surface_pressure": [1013.2, 1012.8, 1012.0],
    "wind_speed_10m": [10.0, 8.0, 6.0],
    "wind_direction_10m": [270.0, 180.0, null]
  }
}"#;

struct ScriptedTransport {
    requests: Mutex<Vec<HttpRequest>>,
    responses: Mutex<VecDeque<Result<HttpResponse, AdsbError>>>,
}

impl ScriptedTransport {
    fn new(responses: Vec<Result<HttpResponse, AdsbError>>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("request log").len()
    }

    fn last_url(&self) -> String {
        self.requests
            .lock()
            .expect("request log")
            .last()
            .expect("a request was sent")
            .url()
            .to_owned()
    }
}

impl HttpTransport for ScriptedTransport {
    fn get(&self, request: &HttpRequest) -> Result<HttpResponse, AdsbError> {
        self.requests
            .lock()
            .expect("request log")
            .push(request.clone());
        self.responses
            .lock()
            .expect("response script")
            .pop_front()
            .unwrap_or_else(|| Err(AdsbError::Io("script exhausted".to_owned())))
    }
}

fn policy(hosts: Vec<String>) -> HttpPolicy {
    HttpPolicy::new(hosts, Duration::from_secs(5), 1 << 20, 2, Duration::ZERO)
        .expect("valid policy")
}

fn weather_policy() -> HttpPolicy {
    policy(OPEN_METEO_HOSTS.map(str::to_owned).to_vec())
}

#[test]
fn fetch_produces_payload_parse_weather_accepts() {
    let transport = ScriptedTransport::new(vec![Ok(HttpResponse::new(
        200,
        FIXTURE.as_bytes().to_vec(),
    ))]);
    // 2024-01-15 10:00Z .. 2024-01-15 14:00Z window around the fixture rows.
    let payload = fetch_weather_payload(
        &weather_policy(),
        &transport,
        WeatherArchive::Archive,
        52.52,
        13.405,
        1_705_309_200.0,
        1_705_323_600.0,
    )
    .expect("fetch succeeds");

    let url = transport.last_url();
    assert!(url.starts_with(ARCHIVE_API_URL));
    assert!(url.contains("latitude=52.52"));
    assert!(url.contains("longitude=13.405"));
    assert!(url.contains("start_date=2024-01-15"));

    let samples = parse_weather(&payload).expect("normalized payload parses");
    // The null-temperature row is skipped: two samples survive.
    assert_eq!(samples.len(), 2);

    // 270° is a wind *from* the west: it blows toward the east at 10 m/s.
    let first = samples.first().expect("first sample");
    assert!((first.temperature_k - 278.15).abs() < 1e-9);
    assert!((first.pressure_pa - 101_320.0).abs() < 1e-6);
    assert!((first.relative_humidity - 0.8).abs() < 1e-9);
    assert!((first.wind_east_m_per_s - 10.0).abs() < 1e-9);
    assert!(first.wind_north_m_per_s.abs() < 1e-9);
}

#[test]
fn window_average_feeds_atmosphere() {
    let payload = parse_open_meteo_response(FIXTURE.as_bytes()).expect("parse fixture");
    let samples = parse_weather(&payload).expect("payload parses");
    // Window covering both surviving rows (2024-01-15 10:00Z and 11:00Z).
    let atmosphere = atmosphere_for_window(&samples, 1_705_309_200.0, 1_705_323_600.0)
        .expect("atmosphere builds");
    assert!(!atmosphere.is_standard_fallback());
    // Mean of 5 °C and 4 °C -> 277.65 K.
    assert!((atmosphere.temperature_k() - 277.65).abs() < 1e-6);
}

#[test]
fn empty_window_falls_back_explicitly() {
    let payload = parse_open_meteo_response(FIXTURE.as_bytes()).expect("parse fixture");
    let samples = parse_weather(&payload).expect("payload parses");
    let atmosphere = atmosphere_for_window(&samples, 1_000_000_000.0, 1_000_003_600.0)
        .expect("atmosphere builds");
    assert!(atmosphere.is_standard_fallback());
}

#[test]
fn disallowed_host_is_rejected_before_any_request() {
    let transport = ScriptedTransport::new(vec![]);
    let result = fetch_weather_payload(
        &policy(vec!["opensky-network.org".to_owned()]),
        &transport,
        WeatherArchive::Archive,
        52.52,
        13.405,
        1_705_309_200.0,
        1_705_323_600.0,
    );
    assert!(matches!(result, Err(AdsbError::DisallowedHost(_))));
    assert_eq!(transport.request_count(), 0);
}

#[test]
fn malformed_response_is_parse_error() {
    let transport = ScriptedTransport::new(vec![Ok(HttpResponse::new(200, b"not json".to_vec()))]);
    let result = fetch_weather_payload(
        &weather_policy(),
        &transport,
        WeatherArchive::Archive,
        52.52,
        13.405,
        1_705_309_200.0,
        1_705_323_600.0,
    );
    assert!(matches!(result, Err(AdsbError::Parse(_))));
}

#[test]
fn historical_forecast_endpoint_is_selectable() {
    let url = open_meteo_url(
        WeatherArchive::HistoricalForecast,
        40.0,
        -73.0,
        1_705_309_200.0,
        1_705_323_600.0,
    )
    .expect("valid window");
    assert!(url.starts_with(adsb::HIST_FORECAST_API_URL));
}
