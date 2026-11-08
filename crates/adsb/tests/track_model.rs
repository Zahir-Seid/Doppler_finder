//! FR-DA-01: Hermite interpolation, derived legs and closest approach.
//!
//! Track-model tests use synthetic legs built through `geodesy` local frames;
//! no test opens a socket or reads a shared fixture.

#![allow(clippy::expect_used)]

use adsb::{
    AdsbError, EngineClass, FlightTrack, Leg, TrackInterpolator, TrackSample, TrackSourceKind,
    closest_approach, interpolate, legs_from_tracks,
};
use core_types::{LatLon, UtcMillis};
use geodesy::{Ecef, Ellipsoid, Enu, LocalFrame};

const BASE_MS: i64 = 1_762_473_600_000;
const OBSERVER: LatLon = LatLon::new(52.0, 13.0);

fn sample(offset_ms: i64, lat_deg: f64, lon_deg: f64, alt_m: f64) -> TrackSample {
    TrackSample {
        utc: UtcMillis(BASE_MS + offset_ms),
        lat_deg,
        lon_deg,
        geometric_altitude_m: alt_m,
        barometric_altitude_m: None,
        ground_speed_m_per_s: None,
        track_deg: None,
        vertical_rate_m_per_s: None,
        latency_s: None,
        flags: 0,
    }
}

fn track(icao24: &str, type_code: Option<&str>, samples: Vec<TrackSample>) -> FlightTrack {
    FlightTrack {
        icao24: icao24.to_owned(),
        callsign: None,
        type_code: type_code.map(str::to_owned),
        source: TrackSourceKind::OpenSky,
        samples,
    }
}

/// Convert an ENU displacement in the observer frame to a track sample.
fn enu_sample(frame: &LocalFrame, offset_ms: i64, enu: Enu) -> TrackSample {
    let ecef = frame.from_enu(enu).expect("enu to ecef");
    let (position, height_m) = Ellipsoid::WGS84.from_ecef(ecef).expect("ecef to geodetic");
    sample(offset_ms, position.lat_deg.0, position.lon_deg.0, height_m)
}

#[test]
fn fr_da_01_hermite_beats_the_chord_on_a_curved_leg() {
    // Samples every ten degrees on a 2 km-radius circle centred east of the
    // observer; the Hermite cubic should land closer to the true arc than the
    // straight chord between neighbouring samples.
    let frame = LocalFrame::wgs84(OBSERVER).expect("frame");
    let radius_m = 2_000.0;
    let centre = Enu {
        east_m: 2_000.0,
        north_m: 0.0,
        up_m: 0.0,
    };
    let mut samples = Vec::new();
    let mut angle = 0.0;
    while angle < 2.0 * std::f64::consts::PI {
        let enu = Enu {
            east_m: centre.east_m + radius_m * angle.cos(),
            north_m: centre.north_m + radius_m * angle.sin(),
            up_m: 1_000.0,
        };
        samples.push(enu_sample(&frame, (angle * 10_000.0) as i64, enu));
        angle += std::f64::consts::PI / 18.0;
    }
    let flight = track("a1b2c3", None, samples);
    let interp = TrackInterpolator::new(&flight).expect("valid track");

    // Mid-interval at 15 degrees: true point on the circle.
    let mid_angle = std::f64::consts::PI / 12.0;
    let truth_enu = Enu {
        east_m: centre.east_m + radius_m * mid_angle.cos(),
        north_m: centre.north_m + radius_m * mid_angle.sin(),
        up_m: 1_000.0,
    };
    let truth_ecef = frame.from_enu(truth_enu).expect("truth ecef");
    let unix_s = BASE_MS as f64 / 1_000.0 + mid_angle * 10.0;
    let hermite_ecef = interp.position_ecef(unix_s).expect("hermite position");

    // Chord approximation: midpoint of the two bracketing knot positions,
    // at 10 and 20 degrees on the circle.
    let knot_lo_s = BASE_MS as f64 / 1_000.0 + (std::f64::consts::PI / 18.0) * 10.0;
    let knot_hi_s = BASE_MS as f64 / 1_000.0 + (std::f64::consts::PI / 9.0) * 10.0;
    let before = interp.position_ecef(knot_lo_s).expect("before");
    let after = interp.position_ecef(knot_hi_s).expect("after");
    let chord = Ecef::new(
        f64::midpoint(before.x_m, after.x_m),
        f64::midpoint(before.y_m, after.y_m),
        f64::midpoint(before.z_m, after.z_m),
    );

    let hermite_error_m = hermite_ecef.distance_m(truth_ecef);
    let chord_error_m = chord.distance_m(truth_ecef);
    assert!(
        hermite_error_m < chord_error_m * 0.5,
        "hermite {hermite_error_m} m vs chord {chord_error_m} m"
    );
    // Sanity: the chord misses by metres on a 10-degree arc.
    assert!(chord_error_m > 1.0, "chord error {chord_error_m} m");
}

#[test]
fn fr_da_01_interpolate_returns_a_flagged_sample_inside_the_span() {
    let flight = track(
        "a1b2c3",
        None,
        vec![
            sample(0, 52.0, 13.0, 10_000.0),
            sample(1_000, 52.001, 13.0, 10_000.0),
        ],
    );
    let at = interpolate(&flight, UtcMillis(BASE_MS + 500)).expect("inside");
    assert_eq!(at.utc, UtcMillis(BASE_MS + 500));
    assert_ne!(at.flags & adsb::FLAG_INTERPOLATED, 0);
    assert!((at.lat_deg - 52.0005).abs() < 1e-6);
}

#[test]
fn fr_da_01_legs_carry_span_bbox_counts_gaps_and_engine_class() {
    let flight = track(
        "a1b2c3",
        Some("C172"),
        vec![
            sample(0, 52.0, 13.0, 1_000.0),
            sample(1_000, 52.001, 13.001, 1_100.0),
            sample(4_000, 52.002, 13.000, 1_200.0),
        ],
    );
    let legs = legs_from_tracks(&[flight]).expect("legs");
    assert_eq!(legs.len(), 1);
    let leg = &legs[0];
    assert_eq!(leg.leg_id, "opensky:a1b2c3:1762473600000");
    assert_eq!(leg.point_count, 3);
    assert_eq!(leg.start_utc, UtcMillis(BASE_MS));
    assert_eq!(leg.end_utc, UtcMillis(BASE_MS + 4_000));
    assert!((leg.min_lat_deg - 52.0).abs() < 1e-12);
    assert!((leg.max_lat_deg - 52.002).abs() < 1e-12);
    assert!((leg.min_lon_deg - 13.0).abs() < 1e-12);
    assert!((leg.max_lon_deg - 13.001).abs() < 1e-12);
    // Gaps are 1 s and 3 s: median 2 s, max 3 s.
    assert!((leg.median_gap_s - 2.0).abs() < 1e-12);
    assert!((leg.max_gap_s - 3.0).abs() < 1e-12);
    assert_eq!(leg.engine_class, EngineClass::Propeller);
}

#[test]
fn fr_da_01_engine_class_marks_jets_and_unknowns() {
    let jet = Leg::from_track(&track(
        "a1b2c3",
        Some("B738"),
        vec![
            sample(0, 52.0, 13.0, 10_000.0),
            sample(1_000, 52.001, 13.0, 10_000.0),
        ],
    ))
    .expect("leg");
    assert_eq!(jet.engine_class, EngineClass::Jet);

    let turboprop = Leg::from_track(&track(
        "a1b2c3",
        Some("PC12"),
        vec![
            sample(0, 52.0, 13.0, 3_000.0),
            sample(1_000, 52.001, 13.0, 3_000.0),
        ],
    ))
    .expect("leg");
    assert_eq!(turboprop.engine_class, EngineClass::Propeller);

    let unknown = Leg::from_track(&track(
        "a1b2c3",
        None,
        vec![
            sample(0, 52.0, 13.0, 3_000.0),
            sample(1_000, 52.001, 13.0, 3_000.0),
        ],
    ))
    .expect("leg");
    assert_eq!(unknown.engine_class, EngineClass::Unknown);
}

#[test]
fn fr_da_01_closest_approach_finds_the_minimum_slant_range() {
    // Straight level pass due north: 300 m east, 900 m above the observer,
    // 250 m/s, closest approach at BASE_MS + 60 s.
    let frame = LocalFrame::wgs84_with_height(OBSERVER, 0.0).expect("frame");
    let t0_ms = BASE_MS + 60_000;
    let mut samples = Vec::new();
    for second in -60..=60 {
        let enu = Enu {
            east_m: 300.0,
            north_m: 250.0 * (second as f64),
            up_m: 900.0,
        };
        samples.push(enu_sample(&frame, t0_ms + second * 1_000 - BASE_MS, enu));
    }
    let flight = track("a1b2c3", Some("C172"), samples);

    let approach = closest_approach(&flight, OBSERVER, 0.0, 340.0)
        .expect("closest approach")
        .expect("a closest approach exists");

    let expected_range_m = 300.0_f64.hypot(900.0);
    assert!(
        (approach.slant_range_m - expected_range_m).abs() < 0.5,
        "range {} vs {expected_range_m}",
        approach.slant_range_m
    );
    assert!(
        (approach.emission_utc.0 - t0_ms).abs() <= 1_000,
        "emission {} vs {t0_ms}",
        approach.emission_utc.0
    );
    let expected_heard_ms = t0_ms + (expected_range_m / 340.0 * 1_000.0).round() as i64;
    assert!(
        (approach.heard_utc.0 - expected_heard_ms).abs() <= 1_000,
        "heard {} vs {expected_heard_ms}",
        approach.heard_utc.0
    );
    assert!((approach.speed_m_per_s - 250.0).abs() < 1.0);
}

#[test]
fn fr_da_01_closest_approach_rejects_bad_inputs_and_short_tracks() {
    let flight = track(
        "a1b2c3",
        None,
        vec![
            sample(0, 52.0, 13.0, 10_000.0),
            sample(1_000, 52.001, 13.0, 10_000.0),
        ],
    );
    assert!(matches!(
        closest_approach(&flight, OBSERVER, 0.0, 0.0),
        Err(AdsbError::InvalidQuery(_))
    ));
    assert!(matches!(
        closest_approach(&flight, OBSERVER, 0.0, f64::NAN),
        Err(AdsbError::InvalidQuery(_))
    ));
    assert!(matches!(
        closest_approach(&flight, OBSERVER, f64::NAN, 340.0),
        Err(AdsbError::InvalidQuery(_))
    ));

    let single = track("a1b2c3", None, vec![sample(0, 52.0, 13.0, 10_000.0)]);
    assert!(
        closest_approach(&single, OBSERVER, 0.0, 340.0)
            .expect("short track")
            .is_none()
    );
}
