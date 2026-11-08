//! FR-DA-03 / ACO-01: measured atmosphere adapter.
//!
//! The payload below is a recorded fixture in the normalized ingest shape;
//! no test opens a network.

#![allow(clippy::expect_used)]

use acoustics::{AcousticsError, Wind, atmosphere_for_window, conditions_in_window, parse_weather};
use core_types::{AtmosphereSource, LatLon};

const PAYLOAD: &str = r#"{
    "samples": [
        {"unix_s": 100.0, "temperature_k": 290.0, "pressure_pa": 101000.0,
         "relative_humidity": 0.5, "wind_east_m_per_s": 4.0, "wind_north_m_per_s": 0.0},
        {"unix_s": 200.0, "temperature_k": 292.0, "pressure_pa": 100500.0,
         "relative_humidity": 0.6, "wind_east_m_per_s": 6.0, "wind_north_m_per_s": 2.0},
        {"unix_s": 300.0, "temperature_k": 294.0, "pressure_pa": 100000.0,
         "relative_humidity": 0.7, "wind_east_m_per_s": -2.0, "wind_north_m_per_s": 0.0}
    ]
}"#;

#[test]
fn fr_da_03_parse_and_window_average() {
    let samples = parse_weather(PAYLOAD).expect("valid payload");
    assert_eq!(samples.len(), 3);
    // Sorted by time even though the fixture lists them in order anyway.
    assert!(samples.windows(2).all(|pair| {
        pair.first().expect("first").unix_s <= pair.get(1).expect("second").unix_s
    }));

    let conditions = conditions_in_window(&samples, 150.0, 250.0).expect("one sample inside");
    assert_eq!(conditions.temperature_k, 292.0);
    assert_eq!(conditions.pressure_pa, 100_500.0);

    // All three samples: means over the window.
    let all = conditions_in_window(&samples, 0.0, 400.0).expect("all inside");
    assert!((all.temperature_k - 292.0).abs() < 1e-12);
    assert!((all.relative_humidity - 0.6).abs() < 1e-12);
    // Wind is averaged as a vector: (4+6-2)/3 east, (0+2+0)/3 north.
    assert!((all.wind.east_m_per_s - 8.0 / 3.0).abs() < 1e-12);
    assert!((all.wind.north_m_per_s - 2.0 / 3.0).abs() < 1e-12);
}

#[test]
fn fr_da_03_window_edges_are_inclusive() {
    let samples = parse_weather(PAYLOAD).expect("valid payload");
    let edge = conditions_in_window(&samples, 100.0, 100.0).expect("edge sample");
    assert_eq!(edge.temperature_k, 290.0);
    assert!(conditions_in_window(&samples, 400.0, 500.0).is_none());
    assert!(conditions_in_window(&samples, 500.0, 400.0).is_none());
}

#[test]
fn fr_da_03_measured_atmosphere_carries_provenance() {
    let samples = parse_weather(PAYLOAD).expect("valid payload");
    let atmosphere = atmosphere_for_window(&samples, 0.0, 400.0).expect("atmosphere");
    assert_eq!(atmosphere.source(), AtmosphereSource::Measured);
    assert!(!atmosphere.is_standard_fallback());
    assert!((atmosphere.temperature_k() - 292.0).abs() < 1e-12);
}

#[test]
fn fr_da_03_empty_window_falls_back_and_flags() {
    let samples = parse_weather(PAYLOAD).expect("valid payload");
    let atmosphere = atmosphere_for_window(&samples, 10_000.0, 20_000.0).expect("fallback");
    assert!(atmosphere.is_standard_fallback());
    assert_eq!(atmosphere.source(), AtmosphereSource::StandardFallback);
    assert!(conditions_in_window(&[], 0.0, 1.0).is_none());
}

#[test]
fn fr_da_03_wind_towards_observer_sign_follows_q10() {
    // Observer due east of the source: an eastward wind blows source ->
    // observer and must come out positive.
    let eastward = Wind {
        east_m_per_s: 5.0,
        north_m_per_s: 0.0,
    };
    let source = LatLon::new(52.0, 13.0);
    let observer = LatLon::new(52.0, 13.1);
    let component = eastward.towards(source, observer).expect("component");
    assert!(
        component > 0.0,
        "eastward wind should be positive, got {component}"
    );
    assert!((component - 5.0).abs() < 0.5);

    // Reversed path flips the sign; a northward wind is orthogonal.
    let reversed = eastward.towards(observer, source).expect("component");
    assert!(reversed < 0.0);
    let northward = Wind {
        east_m_per_s: 0.0,
        north_m_per_s: 5.0,
    };
    // A due-east path is not exactly along a parallel at 52 N, so the
    // orthogonal wind picks up a small projection; it stays far below the
    // 5 m/s signal.
    assert!(
        northward
            .towards(source, observer)
            .expect("component")
            .abs()
            < 0.5
    );
}

#[test]
fn fr_da_03_parse_rejects_malformed_payloads() {
    assert!(matches!(
        parse_weather("not json"),
        Err(AcousticsError::Parse(_))
    ));
    assert!(matches!(
        parse_weather(r#"{"other": []}"#),
        Err(AcousticsError::Parse(_))
    ));
    assert!(matches!(
        parse_weather(r#"{"samples": [{"unix_s": "soon"}]}"#),
        Err(AcousticsError::Parse(_))
    ));
    assert!(matches!(
        parse_weather(
            r#"{"samples": [{"unix_s": 1.0, "temperature_k": 290.0, "pressure_pa": 101000.0,
                          "relative_humidity": 0.5, "wind_east_m_per_s": 0.0,
                          "wind_north_m_per_s": null}]}"#
        ),
        Err(AcousticsError::Parse(_))
    ));
}

#[test]
fn fr_da_03_wind_rejects_a_degenerate_path() {
    let wind = Wind::default();
    let point = LatLon::new(52.0, 13.0);
    assert!(matches!(
        wind.towards(point, point),
        Err(AcousticsError::InvalidQuery(_))
    ));
}
