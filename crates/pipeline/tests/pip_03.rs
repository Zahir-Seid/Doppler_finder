//! PIP-03: the result payload serializes verdict, reason codes, event and
//! aircraft evidence, alternatives, manifest hash and the mode-appropriate
//! location — and the public variant cannot leak exact coordinates.

#![allow(clippy::expect_used)]

use core_types::{EventId, LatLon, UtcMillis};
use events::{EventFit, QualityAssessment};
use pipeline::run::LocalizeOutcome;
use pipeline::{Location, PublicCell, ResultPayload};
use search::{ReasonCode, Verdict, VerdictDecision};

fn event_fit() -> EventFit {
    EventFit {
        event_id: EventId::new_v7(),
        fundamental_hz: core_types::Hertz(85.0),
        speed_m_per_s: 40.0,
        closest_m: core_types::Metres(2_000.0),
        heard_at: core_types::AudioSeconds(10.0),
        covariance: [[0.0; 4]; 4],
    }
}

fn assessment(retained: bool) -> QualityAssessment {
    QualityAssessment {
        quality: events::EventQuality::from_score(0.8).expect("score"),
        retained,
        reasons: Vec::new(),
        curve_snr_db: 12.5,
        residual_autocorrelation: 0.0,
        covariance_inflation: 1.0,
        harmonic_count: 2,
    }
}

fn outcome_with_localize() -> pipeline::AnalyzeOutcome {
    pipeline::AnalyzeOutcome {
        detections: Vec::new(),
        events: vec![pipeline::FittedEvent {
            fit: event_fit(),
            assessment: assessment(true),
        }],
        localize: Some(LocalizeOutcome {
            scored: Vec::new(),
            join_candidates: Vec::new(),
            clusters: Vec::new(),
            ranked: Vec::new(),
            refined: None,
            decision: VerdictDecision {
                verdict: Verdict::Locatable,
                reason_codes: vec![ReasonCode::TruncatedSearch],
            },
            matched_track_indices: vec![0],
            truncated: true,
        }),
        stage_timings: Vec::new(),
        truncated: true,
    }
}

fn track(icao24: &str) -> adsb::FlightTrack {
    adsb::FlightTrack {
        icao24: icao24.to_owned(),
        callsign: Some("TEST".to_owned()),
        type_code: Some("C208".to_owned()),
        source: adsb::TrackSourceKind::AdsbLol,
        samples: vec![adsb::TrackSample {
            utc: UtcMillis(1_700_000_000_000),
            lat_deg: 52.0,
            lon_deg: 13.0,
            geometric_altitude_m: 3_000.0,
            barometric_altitude_m: None,
            ground_speed_m_per_s: Some(40.0),
            track_deg: None,
            vertical_rate_m_per_s: None,
            latency_s: None,
            flags: 0,
        }],
    }
}

#[test]
fn pip_03_payload_carries_verdict_evidence_and_manifest_hash() {
    let outcome = outcome_with_localize();
    let tracks = vec![track("aabbcc")];
    let payload = ResultPayload::build(
        &outcome,
        &tracks,
        "deadbeef",
        Location::Public {
            cell: PublicCell {
                lat_deg: 52.0,
                lon_deg: 13.0,
                size_m: 2_000.0,
            },
            posterior_mass: 0.95,
        },
        vec!["spectrogram.svg".to_owned()],
    );

    assert_eq!(payload.verdict, Verdict::Locatable);
    assert_eq!(payload.reason_codes, vec![ReasonCode::TruncatedSearch]);
    assert_eq!(payload.manifest_sha256, "deadbeef");
    assert!(payload.truncated);
    assert_eq!(payload.events.len(), 1);
    assert_eq!(payload.events[0].quality_score, 0.8);
    assert!(payload.events[0].retained);
    assert_eq!(payload.aircraft.len(), 1);
    assert_eq!(payload.aircraft[0].icao24, "aabbcc");
    assert_eq!(payload.figures, vec!["spectrogram.svg".to_owned()]);

    // The public location is a cell, not a point.
    match payload.location {
        Location::Public {
            cell,
            posterior_mass,
        } => {
            assert_eq!(cell.size_m, 2_000.0);
            assert_eq!(posterior_mass, 0.95);
        }
        other => panic!("expected public location, got {other:?}"),
    }
}

#[test]
fn pip_03_public_payload_json_has_no_exact_coordinate_fields() {
    let outcome = outcome_with_localize();
    let payload = ResultPayload::build(
        &outcome,
        &[],
        "hash",
        Location::Public {
            cell: PublicCell {
                lat_deg: 52.0,
                lon_deg: 13.0,
                size_m: 2_000.0,
            },
            posterior_mass: 0.9,
        },
        Vec::new(),
    );
    let json = payload.to_json().expect("serialize");
    // The public variant must not contain the research-only keys.
    for forbidden in [
        "\"observer\"",
        "\"ellipse_95\"",
        "\"recording_start_unix_s\"",
    ] {
        assert!(
            !json.contains(forbidden),
            "public payload leaked {forbidden}"
        );
    }
    let round_trip: ResultPayload = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(round_trip, payload);
}

#[test]
fn pip_03_research_payload_carries_exact_geometry() {
    let outcome = outcome_with_localize();
    let payload = ResultPayload::build(
        &outcome,
        &[],
        "hash",
        Location::Research {
            observer: LatLon::new(52.0, 13.0),
            ellipse_95: [[100.0, 0.0], [0.0, 50.0]],
            semi_major_95_m: 100.0,
            recording_start_unix_s: 1_700_000_000.0,
            posterior_mass: 0.9,
        },
        Vec::new(),
    );
    match payload.location {
        Location::Research {
            observer,
            semi_major_95_m,
            ..
        } => {
            assert_eq!(observer.lat_deg.0, 52.0);
            assert_eq!(semi_major_95_m, 100.0);
        }
        other => panic!("expected research location, got {other:?}"),
    }
}

#[test]
fn pip_03_no_localize_is_not_locatable_with_reason() {
    let outcome = pipeline::AnalyzeOutcome {
        detections: Vec::new(),
        events: Vec::new(),
        localize: None,
        stage_timings: Vec::new(),
        truncated: false,
    };
    let payload = ResultPayload::build(&outcome, &[], "hash", Location::None, Vec::new());
    assert_eq!(payload.verdict, Verdict::NotLocatable);
    assert_eq!(payload.reason_codes, vec![ReasonCode::NoFlightMatch]);
    assert!(matches!(payload.location, Location::None));
}
