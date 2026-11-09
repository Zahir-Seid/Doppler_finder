//! SRC-08 / SDD 6.3 stage 4 and 6.4: Levenberg–Marquardt refinement of
//! `(east, north, s)` on the negative log posterior, covariance from the
//! inverse Gauss–Newton Hessian, and bounds/prior constraints.

#![allow(clippy::expect_used)]

use std::collections::BTreeSet;

use adsb::{FlightTrack, TrackSample, TrackSourceKind};
use core_types::{AudioSeconds, EventId, Hertz, LatLon, Metres, UtcMillis};
use events::EventFit;
use geodesy::{ElevationModel, GeodesyError, GeoidModel};
use search::{
    ErrorBudget, JoinCandidate, LandPrior, NOMINAL_EAR_HEIGHT_M, RefineBounds, RefineEnv,
    UniformPrior, best_assignment, predict, refine,
};

const BASE_MS: i64 = 1_762_473_600_000;
const SOUND_SPEED: f64 = 340.0;
const TRUE_LAT: f64 = 52.0;
const TRUE_LON: f64 = 13.0;

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

/// Eastbound pass crossing (52, 13) near t=0 at ~34 m/s, 3 000 m altitude.
fn eastbound_track() -> FlightTrack {
    let samples = (-60..=60)
        .step_by(10)
        .map(|offset_s| {
            sample(
                offset_s * 1_000,
                TRUE_LAT,
                TRUE_LON + offset_s as f64 * 0.0005,
                3_000.0,
            )
        })
        .collect();
    FlightTrack {
        icao24: "a1b2c3".to_owned(),
        callsign: None,
        type_code: Some("C172".to_owned()),
        source: TrackSourceKind::OpenSky,
        samples,
    }
}

/// Northbound pass crossing (52, 13) near t=0 at ~34 m/s, 2 500 m.
fn northbound_track() -> FlightTrack {
    let samples = (-60..=60)
        .step_by(10)
        .map(|offset_s| {
            sample(
                offset_s * 1_000,
                TRUE_LAT + offset_s as f64 * 0.0005,
                TRUE_LON,
                2_500.0,
            )
        })
        .collect();
    FlightTrack {
        icao24: "d4e5f6".to_owned(),
        callsign: None,
        type_code: Some("C182".to_owned()),
        source: TrackSourceKind::OpenSky,
        samples,
    }
}

/// Diagonal pass crossing (52, 13) near t=0 at ~35 m/s, 3 500 m — a third
/// `(v, d)` curve so the system is overdetermined and the optimum unique.
fn diagonal_track() -> FlightTrack {
    let samples = (-60..=60)
        .step_by(10)
        .map(|offset_s| {
            sample(
                offset_s * 1_000,
                TRUE_LAT + offset_s as f64 * 0.0004,
                TRUE_LON - offset_s as f64 * 0.0003,
                3_500.0,
            )
        })
        .collect();
    FlightTrack {
        icao24: "b7c8d9".to_owned(),
        callsign: None,
        type_code: Some("PA28".to_owned()),
        source: TrackSourceKind::OpenSky,
        samples,
    }
}

fn fit_for(prediction: &search::Prediction, heard_at_s: f64) -> EventFit {
    let mut covariance = [[0.0; 4]; 4];
    covariance[1][1] = 1.0;
    covariance[2][2] = 100.0;
    covariance[3][3] = 0.25;
    EventFit {
        event_id: EventId::new_v7(),
        fundamental_hz: Hertz(85.0),
        speed_m_per_s: prediction.speed_m_per_s,
        closest_m: Metres(prediction.slant_range_m),
        heard_at: AudioSeconds(heard_at_s),
        covariance,
    }
}

/// A `(hypothesis, candidate)` pair seeded ~80 m east of the true observer
/// with `s` perturbed by +2 s. Two events on two different tracks pin the
/// `(v, d)` curves; the seed sits inside the truth's basin of attraction —
/// the curves' second intersection is a different, legitimate mode that a
/// coarse candidate on the other side would find instead.
fn seeded_candidate(
    tracks: &[FlightTrack],
) -> (Vec<search::Hypothesis>, Vec<EventFit>, JoinCandidate, f64) {
    let budget = ErrorBudget::default();
    let true_observer = LatLon::new(TRUE_LAT, TRUE_LON);
    // Recording start: BASE unix time.
    let s_true = BASE_MS as f64 / 1_000.0;

    let mut events = Vec::new();
    let mut hypotheses = Vec::new();
    for (track_index, track) in tracks.iter().enumerate() {
        let prediction = predict(
            true_observer,
            track,
            &FlatTerrain,
            &ZeroGeoid,
            NOMINAL_EAR_HEIGHT_M,
            SOUND_SPEED,
        )
        .expect("prediction at truth")
        .expect("track interpolable");
        // heard_at chosen so s_i = heard_utc - heard_at == s_true.
        let heard_unix_s = prediction.heard_utc.0 as f64 / 1_000.0;
        let heard_at = heard_unix_s - s_true;
        events.push(fit_for(&prediction, heard_at));

        // Candidate cell ~80 m east of the truth.
        let cell_lon = TRUE_LON + 80.0 / (111_320.0 * TRUE_LAT.to_radians().cos());
        let cell = LatLon::new(TRUE_LAT, cell_lon);
        let cell_prediction = predict(
            cell,
            track,
            &FlatTerrain,
            &ZeroGeoid,
            NOMINAL_EAR_HEIGHT_M,
            SOUND_SPEED,
        )
        .expect("prediction at cell")
        .expect("track interpolable");
        let cell_heard_unix_s = cell_prediction.heard_utc.0 as f64 / 1_000.0;
        hypotheses.push(search::Hypothesis {
            event_index: track_index,
            track_index,
            observer: cell,
            prediction: cell_prediction,
            implied_start_unix_s: cell_heard_unix_s - heard_at,
            clock_sigma_s: 1.118_034, // sqrt(0.25 + 1.0)
            maha_distance: 2.0,
        });
    }

    let candidate = JoinCandidate {
        hypothesis_indices: (0..hypotheses.len()).collect(),
        event_indices: (0..events.len()).collect::<BTreeSet<_>>(),
        cell_lat_deg: TRUE_LAT,
        cell_lon_deg: TRUE_LON + 80.0 / (111_320.0 * TRUE_LAT.to_radians().cos()),
        implied_start_unix_s: s_true + 2.0,
        clock_sigma_s: 1.118_034,
    };
    let _ = budget;
    (hypotheses, events, candidate, s_true)
}

fn env_for<'a>(
    tracks: &'a [FlightTrack],
    budget: &'a ErrorBudget,
    prior: &'a dyn LandPrior,
    bounds: RefineBounds,
) -> RefineEnv<'a> {
    RefineEnv {
        tracks,
        terrain: &FlatTerrain,
        geoid: &ZeroGeoid,
        ear_height_m: NOMINAL_EAR_HEIGHT_M,
        sound_speed_m_per_s: SOUND_SPEED,
        budget,
        adsb_timing_sigma_s: 1.0,
        known_time: None,
        prior,
        bounds,
    }
}

fn bounds() -> RefineBounds {
    RefineBounds {
        min_east_m: -5_000.0,
        max_east_m: 5_000.0,
        min_north_m: -5_000.0,
        max_north_m: 5_000.0,
        min_s_unix_s: 1_762_473_000.0,
        max_s_unix_s: 1_762_474_200.0,
    }
}

fn distance_m(lat_deg: f64, lon_deg: f64) -> f64 {
    let dlat = (lat_deg - TRUE_LAT) * 111_320.0;
    let dlon = (lon_deg - TRUE_LON) * 111_320.0 * TRUE_LAT.to_radians().cos();
    (dlat * dlat + dlon * dlon).sqrt()
}

#[test]
fn refine_recovers_true_observer_and_recording_start() {
    let tracks = [eastbound_track(), northbound_track(), diagonal_track()];
    let (hypotheses, events, candidate, s_true) = seeded_candidate(&tracks);
    let assignment = vec![0usize, 1usize, 2usize];
    let budget = ErrorBudget::default();
    let env = env_for(&tracks, &budget, &UniformPrior, bounds());

    let solution =
        refine(&candidate, &assignment, &hypotheses, &events, &env).expect("refinement succeeds");

    assert!(solution.converged, "LM did not converge");
    let error_m = distance_m(solution.observer.lat_deg.0, solution.observer.lon_deg.0);
    assert!(
        error_m < 100.0,
        "refined observer {error_m} m from truth (lat {}, lon {})",
        solution.observer.lat_deg.0,
        solution.observer.lon_deg.0
    );
    assert!(
        (solution.s_unix_s - s_true).abs() < 0.5,
        "refined s {} vs true {s_true}",
        solution.s_unix_s
    );
    // Covariance must be positive on the diagonal (inverse Hessian).
    for diag in [
        solution.covariance[0][0],
        solution.covariance[1][1],
        solution.covariance[2][2],
    ] {
        assert!(diag > 0.0 && diag.is_finite(), "covariance diagonal {diag}");
    }
    assert!(solution.log_posterior.is_finite());
}

#[test]
fn best_assignment_returns_one_hypothesis_per_event() {
    let tracks = [eastbound_track(), northbound_track(), diagonal_track()];
    let (hypotheses, events, candidate, _s_true) = seeded_candidate(&tracks);
    let budget = ErrorBudget::default();
    let prior = UniformPrior;
    let penv = search::PosteriorEnv {
        budget: &budget,
        adsb_timing_sigma_s: 1.0,
        known_time: None,
        silence: &[None, None, None],
        prior: &prior,
    };
    let (indices, truncated) =
        best_assignment(&candidate, &hypotheses, &events, &penv).expect("assignment succeeds");
    assert!(!truncated);
    assert_eq!(indices.len(), 3);
    // Each entry must reference a distinct event.
    let events_used: BTreeSet<usize> = indices.iter().map(|&i| hypotheses[i].event_index).collect();
    assert_eq!(events_used.len(), 3);
}

#[test]
fn refine_rejects_invalid_bounds() {
    let tracks = [eastbound_track(), northbound_track(), diagonal_track()];
    let (hypotheses, events, candidate, _s_true) = seeded_candidate(&tracks);
    let assignment = vec![0usize, 1usize, 2usize];
    let budget = ErrorBudget::default();
    let mut bad_bounds = bounds();
    bad_bounds.min_east_m = 10.0;
    bad_bounds.max_east_m = -10.0;
    let env = env_for(&tracks, &budget, &UniformPrior, bad_bounds);
    assert!(refine(&candidate, &assignment, &hypotheses, &events, &env).is_err());
}

#[test]
fn refine_rejects_excluding_prior() {
    struct Water;
    impl LandPrior for Water {
        fn log_prior(&self, _observer: LatLon) -> f64 {
            f64::NEG_INFINITY
        }
    }
    let tracks = [eastbound_track(), northbound_track(), diagonal_track()];
    let (hypotheses, events, candidate, _s_true) = seeded_candidate(&tracks);
    let assignment = vec![0usize, 1usize, 2usize];
    let budget = ErrorBudget::default();
    let env = env_for(&tracks, &budget, &Water, bounds());
    assert!(refine(&candidate, &assignment, &hypotheses, &events, &env).is_err());
}
