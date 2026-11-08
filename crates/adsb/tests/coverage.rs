//! ADSB-04 / T-4: region support flags from measured coverage.
//!
//! An empty ingest must never read as a complete empty set.

#![allow(clippy::expect_used)]

use adsb::{FlightTrack, RegionSupport, TrackSample, TrackSourceKind, assess_coverage};
use core_types::UtcMillis;

const BASE_MS: i64 = 1_762_473_600_000;

fn sample(offset_ms: i64) -> TrackSample {
    TrackSample {
        utc: UtcMillis(BASE_MS + offset_ms),
        lat_deg: 52.0,
        lon_deg: 13.0,
        geometric_altitude_m: 10_000.0,
        barometric_altitude_m: None,
        ground_speed_m_per_s: None,
        track_deg: None,
        vertical_rate_m_per_s: None,
        latency_s: None,
        flags: 0,
    }
}

fn track(offsets_ms: &[i64]) -> FlightTrack {
    FlightTrack {
        icao24: "a1b2c3".to_owned(),
        callsign: None,
        type_code: None,
        source: TrackSourceKind::OpenSky,
        samples: offsets_ms.iter().map(|offset| sample(*offset)).collect(),
    }
}

#[test]
fn fr_da_01_empty_ingest_is_unsupported_not_complete() {
    let report = assess_coverage(&[]);
    assert_eq!(report.support, RegionSupport::Unsupported);
    assert_eq!(report.track_count, 0);
    assert_eq!(report.sample_count, 0);
    assert_eq!(report.median_gap_s, None);
    assert_eq!(report.max_gap_s, None);
}

#[test]
fn fr_da_01_dense_ingest_is_supported() {
    let tracks = vec![
        track(&[0, 1_000, 2_000, 3_000]),
        track(&[500, 1_500, 2_500]),
    ];
    let report = assess_coverage(&tracks);
    assert_eq!(report.support, RegionSupport::Supported);
    assert_eq!(report.track_count, 2);
    assert_eq!(report.sample_count, 7);
    assert_eq!(report.median_gap_s, Some(1.0));
    assert_eq!(report.max_gap_s, Some(1.0));
}

#[test]
fn fr_da_01_sparse_ingest_is_insufficient() {
    // A region served only by receivers tens of seconds apart cannot support
    // interpolation; the flag says so instead of implying coverage.
    let tracks = vec![track(&[0, 60_000, 120_000])];
    let report = assess_coverage(&tracks);
    assert_eq!(report.support, RegionSupport::Insufficient);
    assert_eq!(report.median_gap_s, Some(60.0));
    assert_eq!(report.max_gap_s, Some(60.0));
}

#[test]
fn fr_da_01_single_samples_cannot_prove_sampling() {
    // One sample per track leaves no measured gap, so support stays
    // insufficient rather than silently supported.
    let tracks = vec![track(&[0]), track(&[1_000])];
    let report = assess_coverage(&tracks);
    assert_eq!(report.support, RegionSupport::Insufficient);
    assert_eq!(report.median_gap_s, None);
}

#[test]
fn fr_da_01_gap_at_the_provisional_bound_is_supported() {
    let tracks = vec![track(&[0, 10_000, 20_000])];
    let report = assess_coverage(&tracks);
    assert_eq!(report.support, RegionSupport::Supported);
}
