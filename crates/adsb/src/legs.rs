//! Derived flight legs (SDD section 9.2).
//!
//! A leg is the searchable unit the localization stages work on: one stored
//! [`FlightTrack`] reduced to the fields the `legs` table needs — identity,
//! time span, bounding box, point count, measured sampling gaps and engine
//! class. Legs are derived, never stored as truth: replaying the same track
//! always produces the same leg, including the same [`Leg::leg_id`].
//!
//! One leg is derived per track. Splitting a track on long reporting gaps is
//! deliberately not done here because no gap threshold is fixed yet (Q-15);
//! the per-leg `max_gap_s` exposes the worst gap so a later stage can decide.

use core_types::UtcMillis;
use serde::{Deserialize, Serialize};

use crate::error::AdsbError;
use crate::stats::stats_from_intervals;
use crate::tracks::{FlightTrack, TrackSourceKind};

/// Propulsion class used by the silence term (SDD section 6.2).
///
/// Only jets are excluded outright; [`EngineClass::Unknown`] is treated
/// probabilistically, so the class must stay honest rather than guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineClass {
    /// Aircraft with a propeller, including turboprops, which emit the
    /// propeller tone the detector listens for.
    Propeller,
    /// Jet aircraft, which do not produce a propeller fundamental.
    Jet,
    /// Type code missing or not in the classification table.
    Unknown,
}

/// One derived leg, reduced from a [`FlightTrack`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Leg {
    /// Deterministic identifier: `<source>:<icao24>:<start_ms>`.
    pub leg_id: String,
    /// ICAO 24-bit address as lowercase hexadecimal.
    pub icao24: String,
    /// Callsign when the source provided one.
    pub callsign: Option<String>,
    /// ICAO aircraft type code, when known.
    pub type_code: Option<String>,
    /// Archive the track came from.
    pub source: TrackSourceKind,
    /// First sample time, inclusive.
    pub start_utc: UtcMillis,
    /// Last sample time, inclusive.
    pub end_utc: UtcMillis,
    /// Southernmost sample latitude in degrees.
    pub min_lat_deg: f64,
    /// Northernmost sample latitude in degrees.
    pub max_lat_deg: f64,
    /// Westernmost sample longitude in degrees.
    pub min_lon_deg: f64,
    /// Easternmost sample longitude in degrees.
    pub max_lon_deg: f64,
    /// Number of stored samples.
    pub point_count: usize,
    /// Median positive sampling gap in seconds; `0.0` when the leg has fewer
    /// than two samples.
    pub median_gap_s: f64,
    /// Largest positive sampling gap in seconds; `0.0` when the leg has fewer
    /// than two samples.
    pub max_gap_s: f64,
    /// Propulsion class from the type code.
    pub engine_class: EngineClass,
}

impl Leg {
    /// Derive the leg record for one track.
    ///
    /// # Errors
    ///
    /// Returns [`AdsbError::InvalidTrack`] when the track fails validation.
    pub fn from_track(track: &FlightTrack) -> Result<Self, AdsbError> {
        track.validate()?;
        let first = track
            .samples
            .first()
            .ok_or_else(|| AdsbError::InvalidTrack("track is empty".to_owned()))?;
        let last = track
            .samples
            .last()
            .ok_or_else(|| AdsbError::InvalidTrack("track is empty".to_owned()))?;

        let mut min_lat = f64::INFINITY;
        let mut max_lat = f64::NEG_INFINITY;
        let mut min_lon = f64::INFINITY;
        let mut max_lon = f64::NEG_INFINITY;
        let mut gaps = Vec::new();
        for sample in &track.samples {
            min_lat = min_lat.min(sample.lat_deg);
            max_lat = max_lat.max(sample.lat_deg);
            min_lon = min_lon.min(sample.lon_deg);
            max_lon = max_lon.max(sample.lon_deg);
        }
        for pair in track.samples.windows(2) {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let gap_s = (b.utc.0 - a.utc.0) as f64 / 1_000.0;
            if gap_s.is_finite() && gap_s > 0.0 {
                gaps.push(gap_s);
            }
        }
        let median_gap_s = stats_from_intervals(track.samples.len(), &mut gaps)
            .map_or(0.0, |stats| stats.median_s);
        let max_gap_s = gaps.iter().copied().fold(0.0, f64::max);

        Ok(Self {
            leg_id: format!("{}:{}:{}", track.source.as_str(), track.icao24, first.utc.0),
            icao24: track.icao24.clone(),
            callsign: track.callsign.clone(),
            type_code: track.type_code.clone(),
            source: track.source,
            start_utc: first.utc,
            end_utc: last.utc,
            min_lat_deg: min_lat,
            max_lat_deg: max_lat,
            min_lon_deg: min_lon,
            max_lon_deg: max_lon,
            point_count: track.samples.len(),
            median_gap_s,
            max_gap_s,
            engine_class: engine_class(track.type_code.as_deref()),
        })
    }
}

/// Derive one [`Leg`] per track, preserving input order.
///
/// # Errors
///
/// Returns the first [`AdsbError`] produced by [`Leg::from_track`].
pub fn legs_from_tracks(tracks: &[FlightTrack]) -> Result<Vec<Leg>, AdsbError> {
    tracks.iter().map(Leg::from_track).collect()
}

/// Classify an ICAO type code into an [`EngineClass`].
///
/// The tables are a starter set of common designators, not the full ICAO
/// catalogue; anything unlisted is [`EngineClass::Unknown`] rather than a
/// guess. Turboprops count as [`EngineClass::Propeller`] because they emit
/// the propeller tone the detector listens for.
fn engine_class(type_code: Option<&str>) -> EngineClass {
    let Some(code) = type_code else {
        return EngineClass::Unknown;
    };
    let code = code.trim().to_ascii_uppercase();
    if JET_TYPE_CODES.contains(&code.as_str()) {
        EngineClass::Jet
    } else if PROPELLER_TYPE_CODES.contains(&code.as_str()) {
        EngineClass::Propeller
    } else {
        EngineClass::Unknown
    }
}

/// Jet type designators confident enough to exclude from the silence term.
const JET_TYPE_CODES: &[&str] = &[
    // Boeing airliners.
    "B712", "B721", "B722", "B732", "B733", "B734", "B735", "B736", "B737", "B738", "B739", "B741",
    "B742", "B743", "B744", "B748", "B74S", "B752", "B753", "B763", "B772", "B77L", "B77W", "B788",
    "B789", // Airbus airliners.
    "A306", "A310", "A318", "A319", "A20N", "A320", "A321", "A21N", "A332", "A333", "A338", "A339",
    "A343", "A345", "A346", "A359", "A35K", "A388", // Regional and business jets.
    "CRJ7", "CRJ9", "CRJX", "E170", "E175", "E190", "E195", "E290", "C25A", "C25B", "C25C", "C25M",
    "C510", "C525", "C550", "C551", "C560", "C56X", "C650", "C680", "C700", "C750", "GL5T", "G150",
    "G200", "G280", "FA7X", "FA8X", "PC24", "CL30", "CL35", "CL60", "LJ60", "LJ70", "LJ75",
];

/// Propeller and turboprop type designators that emit a propeller tone.
const PROPELLER_TYPE_CODES: &[&str] = &[
    // Cessna pistons and light twins.
    "C150", "C152", "C162", "C172", "C177", "C180", "C182", "C185", "C206", "C207", "C210", "C310",
    "C402", "C404", "C411", "C414", "C421", "C425", "C441",
    // Piper and other singles and twins.
    "P28A", "P28R", "PA34", "PA44", "SR20", "SR22", "BE33", "BE36", "BE58", "DA20", "DA40", "DA42",
    "EV55", "EV97", "EVSS",
    // Turboprops: propeller-driven, so they stay in the audible class.
    "AT43", "AT44", "AT45", "AT46", "AT72", "AT75", "AT76", "DH8A", "DH8B", "DH8C", "DH8D", "PC12",
    "TBM8", "TBM9", "BE20", "BE30", "BE35", "BE40", "BE60", "BE76", "BE80", "C208", "C208B",
    "C425", "SW3", "SW4",
];
