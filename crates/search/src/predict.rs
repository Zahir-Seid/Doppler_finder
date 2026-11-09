//! SDD 6.1 prediction: closest approach of a flight to an observer.
//!
//! For a hypothesis `(observer x, flight j)` the prediction stage finds the
//! instant of minimum 3-D slant range with the light-time relation, and
//! returns the predicted heard time as UTC, the closest distance and the
//! aircraft's horizontal speed there. Observer height is terrain plus a
//! nominal ear height converted to the ellipsoid by [`observer_height_m`];
//! aircraft height comes from the track's geometric altitude, which the ADS-B
//! ingest stores as ellipsoidal height for GNSS-reported rows.
//!
//! Rows whose geometric altitude was filled from barometric altitude carry
//! [`adsb::FLAG_GEOM_FROM_BARO`]: that value approximates orthometric height,
//! so [`ellipsoidal_track`] adds the geoid undulation once per track before
//! the hypothesis loop. Callers should prepare the track once per flight,
//! not once per observer cell.

use core_types::{LatLon, Metres, UtcMillis};
use geodesy::{ElevationModel, GeoidModel, observer_height_m};

use adsb::{AdsbError, FlightTrack, closest_approach};

use crate::error::SearchError;

/// Ear height above the terrain surface, in metres. A standing listener's ear
/// is roughly 1.6 m over ground level; this is a nominal model input, not a
/// measurement, and its error belongs to `Σ_model` in the likelihood.
pub const NOMINAL_EAR_HEIGHT_M: Metres = Metres(1.6);

/// The predicted observables of one `(observer, flight)` hypothesis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prediction {
    /// Emission time of minimum slant range, UTC milliseconds.
    pub emission_utc: UtcMillis,
    /// Time the sound emitted at closest approach is heard, UTC
    /// milliseconds.
    pub heard_utc: UtcMillis,
    /// Minimum 3-D slant range in metres — the predicted closest distance.
    pub slant_range_m: f64,
    /// Aircraft horizontal speed over ground at closest approach, m/s.
    pub speed_m_per_s: f64,
    /// Ellipsoidal observer height used for the prediction, metres.
    pub observer_height_m: f64,
}

/// Predict the closest approach of `track` to `observer` (SDD 6.1).
///
/// `terrain` and `geoid` supply the observer-height datum; `ear_height_m` is
/// the nominal height of the listener above the ground. `sound_speed_m_per_s`
/// comes from the atmosphere model — never a literal. Pass a track prepared
/// by [`ellipsoidal_track`] when the archive contains baro-derived heights.
///
/// Returns `Ok(None)` when the track has fewer than two samples and cannot
/// produce an interpolant.
///
/// # Errors
///
/// Returns [`SearchError::Adsb`] for an unusable track or query,
/// [`SearchError::Geodesy`] when terrain, geoid or the ellipsoid conversion
/// fails, and [`SearchError::NotFinite`] for a bad sound speed.
pub fn predict(
    observer: LatLon,
    track: &FlightTrack,
    terrain: &dyn ElevationModel,
    geoid: &dyn GeoidModel,
    ear_height_m: Metres,
    sound_speed_m_per_s: f64,
) -> Result<Option<Prediction>, SearchError> {
    if !sound_speed_m_per_s.is_finite() || sound_speed_m_per_s <= 0.0 {
        return Err(SearchError::NotFinite);
    }
    let height = observer_height_m(terrain, geoid, observer, ear_height_m)
        .map_err(|error| SearchError::Geodesy(error.to_string()))?;
    let approach =
        closest_approach(track, observer, height.0, sound_speed_m_per_s).map_err(|error| {
            match error {
                AdsbError::Geodesy(inner) => SearchError::Geodesy(inner.to_string()),
                other => SearchError::Adsb(other.to_string()),
            }
        })?;
    Ok(approach.map(|approach| Prediction {
        emission_utc: approach.emission_utc,
        heard_utc: approach.heard_utc,
        slant_range_m: approach.slant_range_m,
        speed_m_per_s: approach.speed_m_per_s,
        observer_height_m: height.0,
    }))
}

/// Return a copy of `track` with baro-derived geometric altitudes corrected
/// to ellipsoidal height.
///
/// Samples flagged [`adsb::FLAG_GEOM_FROM_BARO`] hold a barometric altitude
/// in `geometric_altitude_m`, which approximates orthometric height `H`.
/// The ellipsoid needs `h = H + N`, so the geoid undulation at each affected
/// sample's position is added. Samples with a real GNSS geometric altitude
/// are already ellipsoidal and are left untouched.
///
/// This is a per-flight preparation step: run it once per track before the
/// observer grid loop, not inside `predict`.
///
/// # Errors
///
/// Returns [`SearchError::Geodesy`] when the geoid has no coverage at a
/// flagged sample, or [`SearchError::Adsb`] when the track fails validation.
pub fn ellipsoidal_track(
    track: &FlightTrack,
    geoid: &dyn GeoidModel,
) -> Result<FlightTrack, SearchError> {
    track
        .validate()
        .map_err(|error| SearchError::Adsb(error.to_string()))?;
    let mut adjusted = track.clone();
    for sample in &mut adjusted.samples {
        if sample.flags & adsb::FLAG_GEOM_FROM_BARO == 0 {
            continue;
        }
        let undulation = geoid
            .undulation_m(LatLon::new(sample.lat_deg, sample.lon_deg))
            .map_err(|error| SearchError::Geodesy(error.to_string()))?;
        sample.geometric_altitude_m += undulation;
    }
    Ok(adjusted)
}
