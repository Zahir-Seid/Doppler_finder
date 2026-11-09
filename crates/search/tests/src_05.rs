//! SRC-05 / SDD 6.3 stage 1: coarse `(event, flight, cell)` hypotheses kept
//! inside the 4σ `(v, d)` Mahalanobis gate, carrying the implied recording
//! start for the pair join.

#![allow(clippy::expect_used)]

use adsb::{FlightTrack, TrackSample, TrackSourceKind};
use core_types::{AudioSeconds, EventId, Hertz, LatLon, Metres, UtcMillis};
use events::EventFit;
use geodesy::{ElevationModel, GeodesyError, GeoidModel};
use search::{
    CoarseEnv, DEFAULT_GATE_SIGMA, ErrorBudget, GridSpec, NOMINAL_EAR_HEIGHT_M, SearchError,
    WorkBudget, coarse_hypotheses,
};

const BASE_MS: i64 = 1_762_473_600_000;
const SOUND_SPEED: f64 = 340.0;

fn env(budget: &ErrorBudget) -> CoarseEnv<'_> {
    CoarseEnv {
        terrain: &FlatTerrain,
        geoid: &ZeroGeoid,
        ear_height_m: NOMINAL_EAR_HEIGHT_M,
        sound_speed_m_per_s: SOUND_SPEED,
        budget,
        gate_sigma: DEFAULT_GATE_SIGMA,
        adsb_timing_sigma_s: 1.0,
    }
}

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

fn sample(offset_ms: i64, lat_deg: f64, lon_deg: f64) -> TrackSample {
    TrackSample {
        utc: UtcMillis(BASE_MS + offset_ms),
        lat_deg,
        lon_deg,
        geometric_altitude_m: 3_000.0,
        barometric_altitude_m: None,
        ground_speed_m_per_s: None,
        track_deg: None,
        vertical_rate_m_per_s: None,
        latency_s: None,
        flags: 0,
    }
}

/// Same eastbound pass as src_01: crosses (52.0, 13.0) near t=0 at ~34 m/s,
/// 3 000 m altitude, slant range ~2 898 m to the observer there.
fn overflight_track() -> FlightTrack {
    let samples = (-60..=60)
        .step_by(10)
        .map(|offset_s| sample(offset_s * 1_000, 52.0, 13.0 + offset_s as f64 * 0.0005))
        .collect();
    FlightTrack {
        icao24: "a1b2c3".to_owned(),
        callsign: None,
        type_code: Some("C172".to_owned()),
        source: TrackSourceKind::OpenSky,
        samples,
    }
}

fn fit(speed_m_per_s: f64, closest_m: f64) -> EventFit {
    let mut covariance = [[0.0; 4]; 4];
    covariance[1][1] = 1.0; // v variance: 1 (m/s)^2
    covariance[2][2] = 100.0; // d variance: 100 m^2
    covariance[3][3] = 0.25; // heard-time variance: 0.25 s^2
    EventFit {
        event_id: EventId::new_v7(),
        fundamental_hz: Hertz(85.0),
        speed_m_per_s,
        closest_m: Metres(closest_m),
        heard_at: AudioSeconds(10.0),
        covariance,
    }
}

fn grid() -> GridSpec {
    GridSpec {
        min_lat_deg: 51.995,
        max_lat_deg: 52.005,
        min_lon_deg: 12.995,
        max_lon_deg: 13.005,
        cell_size_m: 250.0,
    }
}

#[test]
fn hypotheses_survive_near_true_observer_only() {
    // The prediction at (52, 13) is ~34.3 m/s and ~2898 m slant; the fit is
    // close enough that only cells within a few hundred metres pass 4σ.
    let events = [fit(34.0, 2_898.0)];
    let tracks = [overflight_track()];
    let budget = ErrorBudget::default();
    let hyps = coarse_hypotheses(
        &events,
        &tracks,
        &grid(),
        &env(&budget),
        &WorkBudget::default(),
    )
    .expect("coarse stage succeeds")
    .hypotheses;
    assert!(!hyps.is_empty(), "expected surviving hypotheses");
    // The (v, d) gate admits a band around the equal-slant-range curve, so
    // individual hypotheses may sit hundreds of metres off; what matters is
    // that the cloud reaches the true observer.
    let nearest_m = hyps
        .iter()
        .map(|hyp| {
            let dlat = (hyp.observer.lat_deg.0 - 52.0) * 111_320.0;
            let dlon = (hyp.observer.lon_deg.0 - 13.0) * 111_320.0 * 52.0_f64.to_radians().cos();
            (dlat * dlat + dlon * dlon).sqrt()
        })
        .fold(f64::INFINITY, f64::min);
    assert!(
        nearest_m < 300.0,
        "no hypothesis within 300 m of the true observer (nearest {nearest_m} m)"
    );
    for hyp in &hyps {
        assert!(hyp.maha_distance <= DEFAULT_GATE_SIGMA);
        // Implied start = heard UTC minus the 10 s audio offset.
        let heard_s = hyp.prediction.heard_utc.0 as f64 / 1_000.0;
        assert!((hyp.implied_start_unix_s - (heard_s - 10.0)).abs() < 1e-6);
        assert!(hyp.clock_sigma_s > 0.0);
    }
}

#[test]
fn mismatched_event_speed_yields_no_hypotheses() {
    // 200 m/s is ~45σ away from the ~34 m/s prediction — nothing survives.
    let events = [fit(200.0, 2_898.0)];
    let tracks = [overflight_track()];
    let budget = ErrorBudget::default();
    let hyps = coarse_hypotheses(
        &events,
        &tracks,
        &grid(),
        &env(&budget),
        &WorkBudget::default(),
    )
    .expect("ok")
    .hypotheses;
    assert!(hyps.is_empty());
}

#[test]
fn invalid_grid_and_gate_are_rejected() {
    let events = [fit(34.0, 2_898.0)];
    let tracks = [overflight_track()];
    let bad_grid = GridSpec {
        cell_size_m: 0.0,
        ..grid()
    };
    let budget = ErrorBudget::default();
    assert!(matches!(
        coarse_hypotheses(
            &events,
            &tracks,
            &bad_grid,
            &env(&budget),
            &WorkBudget::default()
        ),
        Err(SearchError::NotFinite)
    ));
    let mut bad_env = env(&budget);
    bad_env.gate_sigma = 0.0;
    assert!(matches!(
        coarse_hypotheses(&events, &tracks, &grid(), &bad_env, &WorkBudget::default()),
        Err(SearchError::NotFinite)
    ));
}
