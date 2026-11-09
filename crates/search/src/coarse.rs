//! SDD 6.3 stage 1: coarse hypotheses and the 4σ gate (FR-SL-01).
//!
//! For every `(event, flight, observer cell)` triple the prediction stage
//! produces `(v, d, heard UTC)`; the hypothesis survives when the `(v, d)`
//! residual lies inside a `gate_sigma`σ Mahalanobis ellipsoid of the event
//! likelihood's total covariance. Surviving hypotheses carry the implied
//! recording start `s_i` and its sigma, which is what the SRC-06 pair join
//! bins on.
//!
//! Tracks must already be ellipsoid-corrected with [`ellipsoidal_track`];
//! this module does not repeat that per-cell work.

use adsb::FlightTrack;
use core_types::{LatLon, Metres};
use events::EventFit;
use geodesy::{ElevationModel, GeoidModel};

use crate::budget::WorkBudget;
use crate::clock::{implied_recording_start, recording_start_sigma};
use crate::error::SearchError;
use crate::likelihood::{ErrorBudget, check_fit_covariance, vd_gaussian};
use crate::predict::{Prediction, predict};

/// Default Mahalanobis gate: keep residuals within 4σ of the `(v, d)`
/// Gaussian (SDD 6.3 stage 1).
pub const DEFAULT_GATE_SIGMA: f64 = 4.0;

/// Mean metres per degree of latitude, used to size grid cells.
const METRES_PER_DEG_LAT: f64 = 111_320.0;

/// Coarse observer grid over a lat/lon bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSpec {
    /// Southern bound in degrees.
    pub min_lat_deg: f64,
    /// Northern bound in degrees.
    pub max_lat_deg: f64,
    /// Western bound in degrees.
    pub min_lon_deg: f64,
    /// Eastern bound in degrees.
    pub max_lon_deg: f64,
    /// Nominal cell size in metres; converted to degrees at each row's
    /// latitude so cells stay roughly square.
    pub cell_size_m: f64,
}

impl GridSpec {
    /// Check that the box is ordered and the cell size is positive.
    ///
    /// # Errors
    /// Returns [`SearchError::NotFinite`] for non-finite bounds or a
    /// non-positive cell size.
    pub fn validate(&self) -> Result<(), SearchError> {
        if !self.min_lat_deg.is_finite()
            || !self.max_lat_deg.is_finite()
            || !self.min_lon_deg.is_finite()
            || !self.max_lon_deg.is_finite()
            || !self.cell_size_m.is_finite()
            || self.cell_size_m <= 0.0
            || self.min_lat_deg >= self.max_lat_deg
            || self.min_lon_deg >= self.max_lon_deg
        {
            return Err(SearchError::NotFinite);
        }
        Ok(())
    }
}

/// One surviving `(event, flight, observer cell)` hypothesis.
#[derive(Debug, Clone)]
pub struct Hypothesis {
    /// Index into the `events` slice passed to [`coarse_hypotheses`].
    pub event_index: usize,
    /// Index into the `tracks` slice passed to [`coarse_hypotheses`].
    pub track_index: usize,
    /// Observer position at the cell centre.
    pub observer: LatLon,
    /// The 6.1 prediction for this triple.
    pub prediction: Prediction,
    /// Implied recording start `s_i` in Unix seconds.
    pub implied_start_unix_s: f64,
    /// Timing sigma `σ_s,i` in seconds.
    pub clock_sigma_s: f64,
    /// Mahalanobis distance of the `(v, d)` residual (gate units of σ).
    pub maha_distance: f64,
}

/// Environment inputs shared by every hypothesis evaluation.
#[derive(Debug, Clone, Copy)]
pub struct CoarseEnv<'a> {
    /// Terrain elevations for observer height.
    pub terrain: &'a dyn ElevationModel,
    /// Geoid undulations for the height datum.
    pub geoid: &'a dyn GeoidModel,
    /// Nominal ear height above terrain.
    pub ear_height_m: Metres,
    /// Sound speed from the atmosphere model, m/s.
    pub sound_speed_m_per_s: f64,
    /// `(v, d)` error budget for the likelihood covariance.
    pub budget: &'a ErrorBudget,
    /// Mahalanobis gate in σ; [`DEFAULT_GATE_SIGMA`] unless validation says
    /// otherwise.
    pub gate_sigma: f64,
    /// ADS-B timing sigma for the clock term, seconds.
    pub adsb_timing_sigma_s: f64,
}

/// The coarse stage output: surviving hypotheses plus the truncation flag
/// (SRC-12). When the 4σ gate emits more than `budget.max_hypotheses`, the
/// lowest-Mahalanobis hypotheses are kept and `truncated` is set.
#[derive(Debug, Clone)]
pub struct CoarseOutcome {
    /// Gate survivors, best first when truncated.
    pub hypotheses: Vec<Hypothesis>,
    /// True when `budget.max_hypotheses` cut the survivor list.
    pub truncated: bool,
}

/// Generate coarse hypotheses for every `(event, track, cell)` triple and
/// keep those inside the `gate_sigma`σ `(v, d)` gate (SDD 6.3 stage 1).
///
/// `tracks` must be ellipsoid-corrected already (see [`ellipsoidal_track`]);
/// the sound speed and error budget come from the atmosphere model and the
/// FR-SIM-02 budget. `env.gate_sigma` defaults to [`DEFAULT_GATE_SIGMA`];
/// pass a different value only with validation evidence.
///
/// Tracks with fewer than two samples produce no hypotheses. Cells whose
/// `predict` call fails are skipped rather than failing the whole stage:
/// a single bad track or terrain hole must not hide the surviving
/// hypotheses, and the pair join downstream decides what is consistent.
///
/// # Errors
///
/// Returns [`SearchError::NotFinite`] for a bad grid, gate or sound speed,
/// [`SearchError::InvalidThresholds`] for a zero [`WorkBudget`] limit, and
/// propagates fit-level errors (bad covariance, non-finite fit fields)
/// from the first event that triggers them.
pub fn coarse_hypotheses(
    events: &[EventFit],
    tracks: &[FlightTrack],
    grid: &GridSpec,
    env: &CoarseEnv<'_>,
    budget: &WorkBudget,
) -> Result<CoarseOutcome, SearchError> {
    grid.validate()?;
    if !env.gate_sigma.is_finite() || env.gate_sigma <= 0.0 {
        return Err(SearchError::NotFinite);
    }
    if !env.sound_speed_m_per_s.is_finite() || env.sound_speed_m_per_s <= 0.0 {
        return Err(SearchError::NotFinite);
    }
    if !env.adsb_timing_sigma_s.is_finite() || env.adsb_timing_sigma_s < 0.0 {
        return Err(SearchError::NotFinite);
    }
    budget.validate()?;

    let mut hypotheses = Vec::new();
    let gate_squared = env.gate_sigma * env.gate_sigma;
    for (event_index, fit) in events.iter().enumerate() {
        // A non-PD total covariance or bad heard-time variance fails
        // identically at every cell; surface it once instead of silently
        // emitting zero hypotheses for this event.
        check_fit_covariance(fit, env.budget)?;
        recording_start_sigma(fit, env.adsb_timing_sigma_s)?;
        for (track_index, track) in tracks.iter().enumerate() {
            for observer in grid_cells(grid) {
                let Ok(Some(prediction)) = predict(
                    observer,
                    track,
                    env.terrain,
                    env.geoid,
                    env.ear_height_m,
                    env.sound_speed_m_per_s,
                ) else {
                    continue;
                };
                let Ok(gaussian) = vd_gaussian(fit, &prediction, env.budget) else {
                    continue;
                };
                let maha_squared = gaussian.mahalanobis_squared();
                if !maha_squared.is_finite() || maha_squared > gate_squared {
                    continue;
                }
                // The Gaussian already validated finiteness of the fit's
                // (v, d); the clock helpers validate the heard-time fields.
                let Ok(implied_start_unix_s) = implied_recording_start(fit, &prediction) else {
                    continue;
                };
                let Ok(clock_sigma_s) = recording_start_sigma(fit, env.adsb_timing_sigma_s) else {
                    continue;
                };
                hypotheses.push(Hypothesis {
                    event_index,
                    track_index,
                    observer,
                    prediction,
                    implied_start_unix_s,
                    clock_sigma_s,
                    maha_distance: maha_squared.sqrt(),
                });
            }
        }
    }

    // Work budget: keep the best (lowest Mahalanobis) hypotheses when the
    // gate overflows; ties break on identity fields for determinism.
    let truncated = hypotheses.len() > budget.max_hypotheses;
    if truncated {
        hypotheses.sort_by(|a, b| {
            a.maha_distance
                .total_cmp(&b.maha_distance)
                .then(a.event_index.cmp(&b.event_index))
                .then(a.track_index.cmp(&b.track_index))
                .then(a.observer.lat_deg.0.total_cmp(&b.observer.lat_deg.0))
                .then(a.observer.lon_deg.0.total_cmp(&b.observer.lon_deg.0))
        });
        hypotheses.truncate(budget.max_hypotheses);
    }
    Ok(CoarseOutcome {
        hypotheses,
        truncated,
    })
}

/// Cell-centre observer positions covering `grid`, row by row.
///
/// Latitude steps are constant; longitude steps shrink with `cos(lat)` so
/// cells stay roughly square. The antimeridian is not supported — callers
/// must split regions crossing it.
fn grid_cells(grid: &GridSpec) -> Vec<LatLon> {
    let lat_step = grid.cell_size_m / METRES_PER_DEG_LAT;
    let mut cells = Vec::new();
    let mut lat = grid.min_lat_deg + lat_step / 2.0;
    while lat < grid.max_lat_deg {
        let metres_per_deg_lon = METRES_PER_DEG_LAT * lat.to_radians().cos().max(0.01);
        let lon_step = grid.cell_size_m / metres_per_deg_lon;
        let mut lon = grid.min_lon_deg + lon_step / 2.0;
        while lon < grid.max_lon_deg {
            cells.push(LatLon::new(lat, lon));
            lon += lon_step;
        }
        lat += lat_step;
    }
    cells
}
