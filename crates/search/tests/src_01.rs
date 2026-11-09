//! SRC-01 / SDD 6.1: closest-approach prediction for `(observer, flight)`
//! hypotheses — terrain + ear height + geoid for the observer, geometric
//! altitude for the aircraft, light-time heard UTC.

#![allow(clippy::expect_used)]

use adsb::{FLAG_GEOM_FROM_BARO, FlightTrack, TrackSample, TrackSourceKind};
use core_types::{LatLon, Metres, UtcMillis};
use geodesy::{ElevationModel, GeodesyError, GeoidModel};
use search::{NOMINAL_EAR_HEIGHT_M, SearchError, ellipsoidal_track, predict};

const BASE_MS: i64 = 1_762_473_600_000;
const SOUND_SPEED: f64 = 340.0;

/// Flat terrain at 100 m orthometric, zero geoid undulation.
#[derive(Debug)]
struct FlatTerrain;

impl ElevationModel for FlatTerrain {
    fn elevation_m(&self, _position: LatLon) -> Result<f64, GeodesyError> {
        Ok(100.0)
    }
}

#[derive(Debug)]
struct ZeroGeoid;

impl GeoidModel for ZeroGeoid {
    fn undulation_m(&self, _position: LatLon) -> Result<f64, GeodesyError> {
        Ok(0.0)
    }
}

/// Geoid with a constant +30 m undulation, to prove the datum is applied.
#[derive(Debug)]
struct PlusGeoid;

impl GeoidModel for PlusGeoid {
    fn undulation_m(&self, _position: LatLon) -> Result<f64, GeodesyError> {
        Ok(30.0)
    }
}

fn sample(offset_ms: i64, lat_deg: f64, lon_deg: f64, alt_m: f64, flags: u8) -> TrackSample {
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
        flags,
    }
}

fn track(samples: Vec<TrackSample>) -> FlightTrack {
    FlightTrack {
        icao24: "a1b2c3".to_owned(),
        callsign: None,
        type_code: Some("C172".to_owned()),
        source: TrackSourceKind::OpenSky,
        samples,
    }
}

/// Straight eastbound leg at 3 000 m, 10 s sampling, crossing directly over
/// the observer's longitude.
fn overflight_track() -> FlightTrack {
    // At 52°N one degree of longitude is ~69 km; 0.05° steps are ~3.4 km,
    // giving ~343 m/s — a jet-like pass, fine for geometry checks.
    let samples = (-60..=60)
        .step_by(10)
        .map(|offset_s| {
            sample(
                offset_s * 1_000,
                52.0,
                13.0 + offset_s as f64 * 0.0005,
                3_000.0,
                0,
            )
        })
        .collect();
    track(samples)
}

#[test]
fn predict_returns_closest_approach_with_heard_utc() {
    let observer = LatLon::new(52.0, 13.0);
    let flight = overflight_track();
    let prediction = predict(
        observer,
        &flight,
        &FlatTerrain,
        &ZeroGeoid,
        NOMINAL_EAR_HEIGHT_M,
        SOUND_SPEED,
    )
    .expect("prediction succeeds")
    .expect("track is interpolable");

    // The pass crosses the observer's longitude near t=0 s offset, at 3 km
    // altitude: slant range must be close to the height difference.
    assert!(
        (prediction.slant_range_m - 2_898.0).abs() < 100.0,
        "slant range {} m, expected ~2898 m (3000 m aircraft minus ~102 m observer)",
        prediction.slant_range_m
    );
    // Observer ellipsoidal height = terrain 100 + ear 1.6 + geoid 0.
    assert!((prediction.observer_height_m - 101.6).abs() < 1e-9);
    // Heard time is emission plus range/c: ~2898/340 ≈ 8.5 s later.
    let delay_s = (prediction.heard_utc.0 - prediction.emission_utc.0) as f64 / 1_000.0;
    assert!(
        (delay_s - prediction.slant_range_m / SOUND_SPEED).abs() < 0.01,
        "heard delay {delay_s} s must equal range/c"
    );
    // ~0.0005°/s at 52°N is ~34 m/s — a propeller-like pass.
    assert!(
        (prediction.speed_m_per_s - 34.3).abs() < 5.0,
        "speed {} m/s",
        prediction.speed_m_per_s
    );
}

#[test]
fn predict_respects_terrain_and_geoid_height() {
    let observer = LatLon::new(52.0, 13.0);
    let flight = overflight_track();
    let low = predict(
        observer,
        &flight,
        &FlatTerrain,
        &ZeroGeoid,
        Metres(0.0),
        SOUND_SPEED,
    )
    .expect("ok")
    .expect("some");
    let high = predict(
        observer,
        &flight,
        &FlatTerrain,
        &PlusGeoid,
        NOMINAL_EAR_HEIGHT_M,
        SOUND_SPEED,
    )
    .expect("ok")
    .expect("some");
    // +30 m geoid undulation and +1.6 m ear height raise the observer, so the
    // slant range shrinks by roughly the height delta.
    assert!(high.observer_height_m > low.observer_height_m);
    assert!(high.slant_range_m < low.slant_range_m);
}

#[test]
fn ellipsoidal_track_adds_geoid_only_to_baro_rows() {
    let mut samples = overflight_track().samples;
    // Mark every other sample as baro-derived.
    for (index, sample) in samples.iter_mut().enumerate() {
        if index % 2 == 0 {
            sample.flags |= FLAG_GEOM_FROM_BARO;
        }
    }
    let flight = track(samples);
    let adjusted = ellipsoidal_track(&flight, &PlusGeoid).expect("adjusts");
    for (original, fixed) in flight.samples.iter().zip(adjusted.samples.iter()) {
        if original.flags & FLAG_GEOM_FROM_BARO != 0 {
            assert!(
                (fixed.geometric_altitude_m - (original.geometric_altitude_m + 30.0)).abs() < 1e-9
            );
        } else {
            assert_eq!(fixed.geometric_altitude_m, original.geometric_altitude_m);
        }
    }
}

#[test]
fn predict_rejects_bad_sound_speed() {
    let result = predict(
        LatLon::new(52.0, 13.0),
        &overflight_track(),
        &FlatTerrain,
        &ZeroGeoid,
        NOMINAL_EAR_HEIGHT_M,
        f64::NAN,
    );
    assert!(matches!(result, Err(SearchError::NotFinite)));
}

#[test]
fn predict_returns_none_for_single_sample_track() {
    let flight = track(vec![sample(0, 52.0, 13.0, 3_000.0, 0)]);
    let result = predict(
        LatLon::new(52.0, 13.0),
        &flight,
        &FlatTerrain,
        &ZeroGeoid,
        NOMINAL_EAR_HEIGHT_M,
        SOUND_SPEED,
    )
    .expect("no error");
    assert!(result.is_none());
}
