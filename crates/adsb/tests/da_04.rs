//! FR-DA-04: the Parquet track store, its partitions and offline replay.
//!
//! Every test uses a temporary directory. No test opens a socket or reads a
//! shared fixture.

#![allow(clippy::expect_used)]

use adsb::{
    AdsbError, FLAG_INTERPOLATED, FLAG_MLAT, FlightTrack, RegionQuery, TrackSample,
    TrackSourceKind, TrackStore,
};
use core_types::UtcMillis;

const DAY_MS: i64 = 86_400_000;
/// 2025-11-07T00:00:00Z, a fixed date in the store's calendar.
const DAY_START_MS: i64 = 1_762_473_600_000;

fn sample(utc_ms: i64, lat_deg: f64, lon_deg: f64) -> TrackSample {
    TrackSample {
        utc: UtcMillis(utc_ms),
        lat_deg,
        lon_deg,
        geometric_altitude_m: 10_050.5,
        barometric_altitude_m: Some(10_000.25),
        ground_speed_m_per_s: Some(250.5),
        track_deg: Some(90.25),
        vertical_rate_m_per_s: Some(-1.75),
        flags: FLAG_MLAT,
    }
}

fn track(
    source: TrackSourceKind,
    icao24: &str,
    callsign: Option<&str>,
    type_code: Option<&str>,
    samples: Vec<TrackSample>,
) -> FlightTrack {
    FlightTrack {
        icao24: icao24.to_owned(),
        callsign: callsign.map(ToOwned::to_owned),
        type_code: type_code.map(ToOwned::to_owned),
        source,
        samples,
    }
}

fn query() -> RegionQuery {
    RegionQuery {
        min_lat_deg: 36.0,
        min_lon_deg: -123.0,
        max_lat_deg: 39.0,
        max_lon_deg: -121.0,
        from: UtcMillis(DAY_START_MS),
        to: UtcMillis(DAY_START_MS + DAY_MS - 1),
    }
}

#[test]
fn fr_da_04_round_trips_samples_and_both_altitudes() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = TrackStore::new(directory.path());
    let original = track(
        TrackSourceKind::AdsbLol,
        "abc123",
        Some("UAL123"),
        Some("B738"),
        vec![
            sample(DAY_START_MS + 1_000, 37.10, -122.10),
            sample(DAY_START_MS + 31_000, 37.12, -122.08),
        ],
    );
    let runs = store
        .write_tracks(std::slice::from_ref(&original))
        .expect("tracks write");
    assert_eq!(runs.len(), 1);

    let replay = store.read_tracks(&query()).expect("tracks replay");
    assert_eq!(replay.len(), 1);
    let replayed = replay.first().expect("one track");
    assert_eq!(replayed.icao24, "abc123");
    assert_eq!(replayed.callsign.as_deref(), Some("UAL123"));
    assert_eq!(replayed.type_code.as_deref(), Some("B738"));
    assert_eq!(replayed.source, TrackSourceKind::AdsbLol);
    assert_eq!(replayed.samples.len(), 2);

    for (before, after) in original.samples.iter().zip(&replayed.samples) {
        assert_eq!(before.utc, after.utc);
        assert!((before.lat_deg - after.lat_deg).abs() < 1.0e-12);
        assert!((before.lon_deg - after.lon_deg).abs() < 1.0e-12);
        assert!((before.geometric_altitude_m - after.geometric_altitude_m).abs() < 1.0e-3);
        assert!(
            (before.barometric_altitude_m.expect("barometric altitude")
                - after
                    .barometric_altitude_m
                    .expect("stored barometric altitude"))
            .abs()
                < 1.0e-3
        );
        assert!(
            (before.vertical_rate_m_per_s.expect("vertical rate")
                - after.vertical_rate_m_per_s.expect("stored vertical rate"))
            .abs()
                < 1.0e-4
        );
        assert!(
            (before.ground_speed_m_per_s.expect("speed")
                - after.ground_speed_m_per_s.expect("stored speed"))
            .abs()
                < 1.0e-3
        );
        assert!(
            (before.track_deg.expect("track") - after.track_deg.expect("stored track")).abs()
                < 1.0e-4
        );
        assert_eq!(before.flags, after.flags);
    }
}

#[test]
fn fr_da_04_partitions_by_source_date_and_tile() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = TrackStore::new(directory.path());
    let tracks = vec![
        track(
            TrackSourceKind::OpenSky,
            "abc123",
            None,
            None,
            vec![sample(DAY_START_MS + 1_000, 37.5, -122.5)],
        ),
        track(
            TrackSourceKind::OpenSky,
            "def456",
            None,
            None,
            vec![sample(DAY_START_MS + 2_000, 38.5, -121.5)],
        ),
        track(
            TrackSourceKind::OpenSky,
            "aaa111",
            None,
            None,
            vec![sample(DAY_START_MS + 3_000, 37.5, -121.5)],
        ),
        track(
            TrackSourceKind::OpenSky,
            "bbb222",
            None,
            None,
            vec![sample(DAY_START_MS + DAY_MS + 1_000, 37.5, -122.5)],
        ),
        track(
            TrackSourceKind::AdsbLol,
            "ccc333",
            None,
            None,
            vec![sample(DAY_START_MS + 4_000, 37.5, -122.5)],
        ),
    ];
    let runs = store.write_tracks(&tracks).expect("tracks write");
    let paths: Vec<&str> = runs
        .iter()
        .flat_map(|run| run.files.iter().map(|file| file.path.as_str()))
        .collect();

    assert_eq!(runs.len(), 3, "two sources and two days");
    for expected in [
        "source=opensky/date=2025-11-07/lat=37/lon=-123/",
        "source=opensky/date=2025-11-07/lat=38/lon=-122/",
        "source=opensky/date=2025-11-07/lat=37/lon=-122/",
        "source=opensky/date=2025-11-08/lat=37/lon=-123/",
        "source=adsblol/date=2025-11-07/lat=37/lon=-123/",
    ] {
        assert!(
            paths.iter().any(|path| path.starts_with(expected)),
            "missing partition {expected} in {paths:?}"
        );
    }
    assert!(paths.iter().all(|path| path.ends_with(".parquet")));
}

#[test]
fn fr_da_04_replays_only_the_requested_region_and_window() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = TrackStore::new(directory.path());
    let tracks = vec![
        track(
            TrackSourceKind::OpenSky,
            "abc123",
            None,
            None,
            vec![
                sample(DAY_START_MS + 1_000, 37.10, -122.10),
                sample(DAY_START_MS + 2_000, 37.12, -122.08),
                sample(DAY_START_MS + 9_000, 37.90, -122.90),
            ],
        ),
        track(
            TrackSourceKind::OpenSky,
            "def456",
            None,
            None,
            vec![sample(DAY_START_MS + 1_500, 38.10, -121.10)],
        ),
    ];
    store.write_tracks(&tracks).expect("tracks write");

    let narrow = RegionQuery {
        min_lat_deg: 37.0,
        max_lat_deg: 37.2,
        min_lon_deg: -122.2,
        max_lon_deg: -122.0,
        from: UtcMillis(DAY_START_MS + 1_000),
        to: UtcMillis(DAY_START_MS + 2_000),
    };
    let replay = store.read_tracks(&narrow).expect("tracks replay");
    assert_eq!(replay.len(), 1);
    let samples = &replay.first().expect("one track").samples;
    assert_eq!(samples.len(), 2);
    assert_eq!(samples.first().expect("first").utc.0, DAY_START_MS + 1_000);
    assert_eq!(samples.get(1).expect("second").utc.0, DAY_START_MS + 2_000);

    let other = store
        .read_tracks(&RegionQuery {
            min_lat_deg: 38.0,
            max_lat_deg: 38.2,
            min_lon_deg: -121.2,
            max_lon_deg: -121.0,
            ..narrow
        })
        .expect("tracks replay");
    assert_eq!(other.len(), 1);
    assert_eq!(other.first().expect("one track").icao24, "def456");

    let empty = store
        .read_tracks(&RegionQuery {
            min_lat_deg: 10.0,
            max_lat_deg: 11.0,
            ..narrow
        })
        .expect("tracks replay");
    assert_eq!(empty, Vec::<FlightTrack>::new());
}

#[test]
fn fr_da_04_preserves_original_timestamps_without_resampling() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = TrackStore::new(directory.path());
    let irregular = vec![1_000, 1_500, 7_250, 7_300, 60_000]
        .into_iter()
        .map(|offset| sample(DAY_START_MS + offset, 37.0, -122.0))
        .collect::<Vec<_>>();
    let original = track(
        TrackSourceKind::OpenSky,
        "abc123",
        None,
        None,
        irregular.clone(),
    );
    store
        .write_tracks(std::slice::from_ref(&original))
        .expect("tracks write");

    let replay = store.read_tracks(&query()).expect("tracks replay");
    let replayed = replay.first().expect("one track");
    assert_eq!(replayed.samples.len(), irregular.len());
    let original_times: Vec<i64> = irregular.iter().map(|sample| sample.utc.0).collect();
    let replayed_times: Vec<i64> = replayed.samples.iter().map(|sample| sample.utc.0).collect();
    assert_eq!(original_times, replayed_times);
}

#[test]
fn fr_da_04_recomputes_per_source_and_day_ingest_statistics() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = TrackStore::new(directory.path());
    let tracks = vec![
        track(
            TrackSourceKind::OpenSky,
            "abc123",
            None,
            None,
            vec![
                sample(DAY_START_MS + 1_000, 37.0, -122.0),
                sample(DAY_START_MS + 2_000, 37.0, -122.0),
                sample(DAY_START_MS + 4_000, 37.0, -122.0),
            ],
        ),
        track(
            TrackSourceKind::OpenSky,
            "def456",
            None,
            None,
            vec![sample(DAY_START_MS + DAY_MS + 1_000, 37.0, -122.0)],
        ),
        track(
            TrackSourceKind::AdsbLol,
            "aaa111",
            None,
            None,
            vec![
                sample(DAY_START_MS + 1_000, 37.0, -122.0),
                sample(DAY_START_MS + 6_000, 37.0, -122.0),
            ],
        ),
    ];
    let written = store.write_tracks(&tracks).expect("tracks write");
    let recomputed = store.ingest_runs().expect("ingest runs");

    assert_eq!(written, recomputed);
    let opensky_day = recomputed
        .iter()
        .find(|run| run.source == TrackSourceKind::OpenSky && run.date == "2025-11-07")
        .expect("OpenSky day");
    assert_eq!(opensky_day.rows, 3);
    assert!((opensky_day.sampling_median_s.expect("median") - 1.5).abs() < 1.0e-12);
    assert!((opensky_day.sampling_p90_s.expect("p90") - 2.0).abs() < 1.0e-12);
    assert_eq!(opensky_day.files.len(), 1);
    assert_eq!(opensky_day.file_sha256.len(), 64);

    let adsblol_day = recomputed
        .iter()
        .find(|run| run.source == TrackSourceKind::AdsbLol)
        .expect("adsb.lol day");
    assert_eq!(adsblol_day.rows, 2);
    assert!((adsblol_day.sampling_median_s.expect("median") - 5.0).abs() < 1.0e-12);
    assert!((adsblol_day.sampling_p90_s.expect("p90") - 5.0).abs() < 1.0e-12);

    let single = recomputed
        .iter()
        .find(|run| run.date == "2025-11-08")
        .expect("single-sample day");
    assert_eq!(single.rows, 1);
    assert_eq!(single.sampling_median_s, None);
    assert_eq!(single.sampling_p90_s, None);
}

#[test]
fn fr_da_04_writes_are_deterministic() {
    let first_directory = tempfile::tempdir().expect("temp dir");
    let second_directory = tempfile::tempdir().expect("temp dir");
    let first = TrackStore::new(first_directory.path());
    let second = TrackStore::new(second_directory.path());
    let tracks = vec![track(
        TrackSourceKind::OpenSky,
        "abc123",
        Some("UAL123"),
        Some("B738"),
        vec![
            sample(DAY_START_MS + 1_000, 37.0, -122.0),
            sample(DAY_START_MS + 2_000, 37.0, -122.0),
        ],
    )];

    let first_runs = first.write_tracks(&tracks).expect("first write");
    let second_runs = second.write_tracks(&tracks).expect("second write");
    assert_eq!(first_runs, second_runs);
    assert_eq!(first_runs, first.write_tracks(&tracks).expect("rewrite"));

    let first_file = first_runs
        .first()
        .and_then(|run| run.files.first())
        .expect("one file");
    let second_file = second_runs
        .first()
        .and_then(|run| run.files.first())
        .expect("one file");
    assert_eq!(first_file.path, second_file.path);
    assert_eq!(first_file.sha256, second_file.sha256);
}

#[test]
fn fr_da_04_rejects_invalid_queries_and_tracks() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = TrackStore::new(directory.path());
    let bad_query = RegionQuery {
        min_lat_deg: 39.0,
        max_lat_deg: 38.0,
        ..query()
    };
    assert!(matches!(
        store.read_tracks(&bad_query),
        Err(AdsbError::InvalidQuery(_))
    ));

    let reversed = track(
        TrackSourceKind::OpenSky,
        "abc123",
        None,
        None,
        vec![
            sample(DAY_START_MS + 2_000, 37.0, -122.0),
            sample(DAY_START_MS + 1_000, 37.0, -122.0),
        ],
    );
    assert!(matches!(
        store.write_tracks(&[reversed]),
        Err(AdsbError::InvalidTrack(_))
    ));
}

#[test]
fn fr_da_04_empty_input_and_missing_store_are_empty() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = TrackStore::new(directory.path().join("not-created"));
    assert_eq!(store.read_tracks(&query()).expect("reads"), Vec::new());
    assert_eq!(store.ingest_runs().expect("runs"), Vec::new());
    assert_eq!(store.write_tracks(&[]).expect("empty write"), Vec::new());
}

#[test]
fn fr_da_04_samples_at_window_edges_are_inclusive() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = TrackStore::new(directory.path());
    let tracks = vec![track(
        TrackSourceKind::OpenSky,
        "abc123",
        None,
        None,
        vec![
            sample(DAY_START_MS, 36.0, -123.0),
            sample(DAY_START_MS + 1_000, 39.0, -121.0),
            sample(DAY_START_MS + 2_000, 39.5, -121.0),
        ],
    )];
    store.write_tracks(&tracks).expect("tracks write");

    let edge = RegionQuery {
        min_lat_deg: 36.0,
        min_lon_deg: -123.0,
        max_lat_deg: 39.0,
        max_lon_deg: -121.0,
        from: UtcMillis(DAY_START_MS),
        to: UtcMillis(DAY_START_MS + 1_000),
    };
    let replay = store.read_tracks(&edge).expect("tracks replay");
    assert_eq!(
        replay.first().expect("track").samples.len(),
        2,
        "the window and region edges are inclusive"
    );
}

#[test]
fn fr_da_04_interpolated_and_original_flags_round_trip() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = TrackStore::new(directory.path());
    let mut first = sample(DAY_START_MS + 1_000, 37.0, -122.0);
    first.flags = 0;
    let mut second = sample(DAY_START_MS + 2_000, 37.0, -122.0);
    second.flags = FLAG_MLAT | FLAG_INTERPOLATED;
    let tracks = vec![track(
        TrackSourceKind::OpenSky,
        "abc123",
        None,
        None,
        vec![first, second],
    )];
    store.write_tracks(&tracks).expect("tracks write");
    let replay = store.read_tracks(&query()).expect("tracks replay");
    let samples = &replay.first().expect("track").samples;
    assert_eq!(samples.first().expect("first").flags, 0);
    assert_eq!(
        samples.get(1).expect("second").flags,
        FLAG_MLAT | FLAG_INTERPOLATED
    );
}
