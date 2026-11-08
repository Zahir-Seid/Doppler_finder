//! FR-DA-01: archive clients, request bounds and sampling statistics.
//!
//! Every response here is recorded or scripted. No test opens a socket.

#![allow(clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use adsb::{
    AdsbError, FlightTrack, HttpPolicy, HttpRequest, HttpResponse, HttpTransport, OpenSkyClient,
    RegionQuery, SamplingStats, TrackSample, TrackSource, TrackSourceKind, parse_opensky_states,
    sampling_stats,
};
use core_types::UtcMillis;

const FIXTURE: &str = include_str!("fixtures/opensky_states.json");

#[derive(Debug)]
struct ScriptedTransport {
    responses: Mutex<VecDeque<Result<HttpResponse, AdsbError>>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl ScriptedTransport {
    fn new(responses: Vec<HttpResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("request log").len()
    }

    fn last_request(&self) -> HttpRequest {
        self.requests
            .lock()
            .expect("request log")
            .last()
            .cloned()
            .expect("a request was sent")
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

fn policy(max_attempts: usize, max_response_bytes: usize) -> HttpPolicy {
    HttpPolicy::new(
        vec!["opensky-network.org".to_owned()],
        Duration::from_secs(5),
        max_response_bytes,
        max_attempts,
        Duration::ZERO,
    )
    .expect("valid policy")
}

fn query() -> RegionQuery {
    RegionQuery {
        min_lat_deg: 37.0,
        min_lon_deg: -122.2,
        max_lat_deg: 37.5,
        max_lon_deg: -121.9,
        from: UtcMillis(1_762_500_000_000),
        to: UtcMillis(1_762_500_005_000),
    }
}

fn sample(utc_s: i64) -> TrackSample {
    TrackSample {
        utc: UtcMillis(utc_s * 1000),
        lat_deg: 37.0,
        lon_deg: -122.0,
        geometric_altitude_m: 10_000.0,
        barometric_altitude_m: Some(9_990.0),
        ground_speed_m_per_s: Some(250.0),
        track_deg: Some(90.0),
        vertical_rate_m_per_s: Some(1.5),
        flags: 0,
    }
}

#[test]
fn fr_da_01_opensky_parses_recorded_states() {
    let tracks = parse_opensky_states(FIXTURE, &query()).expect("fixture parses");
    assert_eq!(tracks.len(), 1);
    let track = tracks.first().expect("one track");
    assert_eq!(track.icao24, "abc123");
    assert_eq!(track.callsign.as_deref(), Some("UAL123"));
    assert_eq!(track.type_code, None);
    assert_eq!(track.source, TrackSourceKind::OpenSky);
    assert_eq!(track.samples.len(), 2);
    let first = track.samples.first().expect("first sample");
    assert_eq!(first.utc.0, 1_762_500_000_000);
    assert!((first.lat_deg - 37.10).abs() < 1.0e-12);
    assert!((first.lon_deg + 122.10).abs() < 1.0e-12);
    assert!((first.geometric_altitude_m - 10_050.0).abs() < 1.0e-12);
    assert!((first.barometric_altitude_m.expect("baro") - 10_000.0).abs() < 1.0e-12);
    assert_eq!(first.ground_speed_m_per_s, Some(250.0));
    assert_eq!(first.track_deg, Some(90.0));
    assert_eq!(first.vertical_rate_m_per_s, Some(0.0));
    assert_eq!(first.flags, 0);
    let second = track.samples.get(1).expect("second sample");
    assert_eq!(second.utc.0, 1_762_500_002_000);
}

#[test]
fn fr_da_01_opensky_drops_samples_outside_the_window() {
    let tracks = parse_opensky_states(
        FIXTURE,
        &RegionQuery {
            to: UtcMillis(1_762_500_000_500),
            ..query()
        },
    )
    .expect("fixture parses");
    let track = tracks.first().expect("one track");
    assert_eq!(track.samples.len(), 1);
    assert_eq!(
        track.samples.first().expect("sample").utc.0,
        1_762_500_000_000
    );

    let empty = parse_opensky_states(
        FIXTURE,
        &RegionQuery {
            from: UtcMillis(1_762_500_900_000),
            to: UtcMillis(1_762_500_901_000),
            ..query()
        },
    )
    .expect("fixture parses");
    assert_eq!(empty, Vec::<FlightTrack>::new());
}

#[test]
fn fr_da_01_opensky_accepts_an_empty_states_field() {
    assert_eq!(
        parse_opensky_states(r#"{"time": 1, "states": null}"#, &query()).expect("null states"),
        Vec::<FlightTrack>::new()
    );
}

#[test]
fn fr_da_01_opensky_rejects_malformed_payloads() {
    for payload in [
        "not json",
        r#"{"time": 1}"#,
        r#"{"states": {}}"#,
        r#"{"states": [["xyz"]]}"#,
        r#"{"states": [["abc123", null, null, 1762500000, 1762500001, 200.0, 37.0, null, false, null, null, null, null, 1000.0]]}"#,
    ] {
        assert!(
            matches!(
                parse_opensky_states(payload, &query()),
                Err(AdsbError::Parse(_))
            ),
            "payload should be rejected: {payload}"
        );
    }
}

#[test]
fn fr_da_01_http_policy_rejects_urls_outside_the_allow_list() {
    let transport = ScriptedTransport::new(vec![HttpResponse::new(200, Vec::new())]);
    let policy = policy(1, 1024);
    for (url, disallowed) in [
        ("http://opensky-network.org/api/states/all", false),
        ("https://evil.example/states", true),
        ("https://opensky-network.org:8443/api", false),
        ("https://user@opensky-network.org/api", false),
        ("https://opensky-network.org/api#fragment", false),
    ] {
        let request = HttpRequest::new(url, None);
        let result = policy.fetch(&transport, &request);
        if disallowed {
            assert!(
                matches!(result, Err(AdsbError::DisallowedHost(_))),
                "{url} should be rejected by host"
            );
        } else {
            assert!(
                matches!(result, Err(AdsbError::InvalidRequest(_))),
                "{url} should be rejected as a request"
            );
        }
    }
    assert_eq!(transport.request_count(), 0);
}

#[test]
fn fr_da_01_http_policy_caps_the_response_size() {
    let transport = ScriptedTransport::new(vec![HttpResponse::new(200, vec![0_u8; 101])]);
    let policy = policy(3, 100);
    let request = HttpRequest::new("https://opensky-network.org/api/states/all", None);
    assert!(matches!(
        policy.fetch(&transport, &request),
        Err(AdsbError::ResponseTooLarge { limit_bytes: 100 })
    ));
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn fr_da_01_http_policy_retries_are_bounded() {
    let transport = ScriptedTransport::new(vec![
        HttpResponse::new(503, Vec::new()),
        HttpResponse::new(503, Vec::new()),
        HttpResponse::new(200, br"{}".to_vec()),
    ]);
    let policy = policy(3, 1024);
    let request = HttpRequest::new("https://opensky-network.org/api/states/all", None);
    let response = policy.fetch(&transport, &request).expect("third attempt");
    assert_eq!(response.status(), 200);
    assert_eq!(transport.request_count(), 3);

    let failing = ScriptedTransport::new(vec![
        HttpResponse::new(503, Vec::new()),
        HttpResponse::new(503, Vec::new()),
        HttpResponse::new(503, Vec::new()),
        HttpResponse::new(503, Vec::new()),
    ]);
    assert!(matches!(
        policy.fetch(&failing, &request),
        Err(AdsbError::HttpStatus { status: 503 })
    ));
    assert_eq!(failing.request_count(), 3);

    let missing = ScriptedTransport::new(vec![HttpResponse::new(404, Vec::new())]);
    assert!(matches!(
        policy.fetch(&missing, &request),
        Err(AdsbError::HttpStatus { status: 404 })
    ));
    assert_eq!(missing.request_count(), 1);
}

#[test]
fn fr_da_01_http_policy_rejects_impossible_bounds() {
    for build in [
        HttpPolicy::new(Vec::new(), Duration::from_secs(1), 10, 1, Duration::ZERO),
        HttpPolicy::new(
            vec!["  ".to_owned()],
            Duration::from_secs(1),
            10,
            1,
            Duration::ZERO,
        ),
        HttpPolicy::new(
            vec!["opensky-network.org".to_owned()],
            Duration::ZERO,
            10,
            1,
            Duration::ZERO,
        ),
        HttpPolicy::new(
            vec!["opensky-network.org".to_owned()],
            Duration::from_secs(1),
            0,
            1,
            Duration::ZERO,
        ),
        HttpPolicy::new(
            vec!["opensky-network.org".to_owned()],
            Duration::from_secs(1),
            10,
            0,
            Duration::ZERO,
        ),
    ] {
        assert!(matches!(build, Err(AdsbError::InvalidRequest(_))));
    }
}

#[test]
fn fr_da_01_opensky_client_uses_the_allow_listed_endpoint() {
    let transport =
        ScriptedTransport::new(vec![HttpResponse::new(200, FIXTURE.as_bytes().to_vec())]);
    let client = OpenSkyClient::new(
        transport,
        policy(1, 1_048_576),
        Some("test-token".to_owned()),
    );
    let tracks = client.tracks(&query()).expect("recorded tracks");
    assert_eq!(tracks.len(), 1);
    let request = client.transport().last_request();
    assert!(
        request
            .url()
            .starts_with("https://opensky-network.org/api/states/all?")
    );
    assert!(request.url().contains("lamin=37"));
    assert!(request.url().contains("lomin=-122.2"));
    assert!(request.url().contains("lamax=37.5"));
    assert!(request.url().contains("lomax=-121.9"));
    assert!(request.url().contains("time=1762500000"));
    assert_eq!(request.bearer_token(), Some("test-token"));
    assert_eq!(client.transport().request_count(), 1);
}

#[test]
fn fr_da_01_opensky_client_rejects_a_bad_query_before_fetching() {
    let transport = ScriptedTransport::new(vec![HttpResponse::new(200, Vec::new())]);
    let client = OpenSkyClient::new(transport, policy(1, 1024), None);
    let bad = RegionQuery {
        min_lat_deg: 38.0,
        max_lat_deg: 37.0,
        ..query()
    };
    assert!(matches!(
        client.tracks(&bad),
        Err(AdsbError::InvalidQuery(_))
    ));
    assert_eq!(client.transport().request_count(), 0);
}

#[test]
fn fr_da_01_region_query_rejects_invalid_bounds() {
    for bad in [
        RegionQuery {
            min_lat_deg: -91.0,
            ..query()
        },
        RegionQuery {
            max_lon_deg: 181.0,
            ..query()
        },
        RegionQuery {
            min_lon_deg: 10.0,
            max_lon_deg: -10.0,
            ..query()
        },
        RegionQuery {
            from: UtcMillis(2),
            to: UtcMillis(1),
            ..query()
        },
        RegionQuery {
            min_lat_deg: f64::NAN,
            ..query()
        },
    ] {
        assert!(matches!(bad.validate(), Err(AdsbError::InvalidQuery(_))));
    }
}

#[test]
fn fr_da_01_sampling_stats_report_median_and_p90() {
    let tracks = vec![
        FlightTrack {
            icao24: "abc123".to_owned(),
            callsign: None,
            type_code: None,
            source: TrackSourceKind::OpenSky,
            samples: vec![sample(0), sample(1), sample(3)],
        },
        FlightTrack {
            icao24: "def456".to_owned(),
            callsign: None,
            type_code: None,
            source: TrackSourceKind::OpenSky,
            samples: vec![sample(0), sample(3), sample(7)],
        },
    ];
    let stats = sampling_stats(&tracks).expect("sampling intervals");
    assert_eq!(stats.sample_count, 6);
    assert_eq!(stats.interval_count, 4);
    assert!((stats.median_s - 2.5).abs() < 1.0e-12);
    assert!((stats.p90_s - 4.0).abs() < 1.0e-12);

    let source = stats.data_source("opensky", "2026-11-07", "00".repeat(32));
    assert_eq!(source.source, "opensky");
    assert_eq!(source.version, "2026-11-07");
    assert!((source.sampling_median_s - 2.5).abs() < 1.0e-12);
    assert!((source.sampling_p90_s - 4.0).abs() < 1.0e-12);

    let no_gaps = vec![FlightTrack {
        icao24: "abc123".to_owned(),
        callsign: None,
        type_code: None,
        source: TrackSourceKind::OpenSky,
        samples: vec![sample(1), sample(1)],
    }];
    assert_eq!(sampling_stats(&no_gaps), None);
    assert_eq!(
        sampling_stats(&[FlightTrack {
            icao24: "abc123".to_owned(),
            callsign: None,
            type_code: None,
            source: TrackSourceKind::OpenSky,
            samples: vec![sample(1)],
        }]),
        None
    );
}

#[test]
fn fr_da_01_sampling_stats_are_deterministic() {
    let tracks = vec![FlightTrack {
        icao24: "abc123".to_owned(),
        callsign: None,
        type_code: None,
        source: TrackSourceKind::OpenSky,
        samples: vec![sample(0), sample(1), sample(3), sample(6)],
    }];
    let first = sampling_stats(&tracks);
    let second = sampling_stats(&tracks);
    assert_eq!(first, second);
    assert!(matches!(first, Some(SamplingStats { .. })));
}
