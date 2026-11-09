//! Result payload and manifest export (PIP-03, FR-OUT-04, SDD 10.4).
//!
//! The serialized shape is fixed here: the public variant physically cannot
//! carry exact coordinates — there is no field for them — while the research
//! variant adds observer position, the calibrated ellipse and the implied
//! recording start (D-1).

use core_types::LatLon;
use serde::{Deserialize, Serialize};

use search::{ReasonCode, Verdict};

use crate::run::{AnalyzeOutcome, FittedEvent};

/// Which geometry a result is allowed to disclose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// Public output: a coarse region cell and masses only (D-1).
    Public,
    /// Research output: exact observer, ellipse and recording start.
    Research,
}

/// Quality evidence for one detected event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEvidence {
    /// Fitted event identity.
    pub event_id: core_types::EventId,
    /// Combined quality score in `[0, 1]`.
    pub quality_score: f64,
    /// Whether the event survived the quality gate.
    pub retained: bool,
    /// Harmonics that contributed to the fit.
    pub harmonic_count: usize,
    /// Mean whitened curve SNR in decibels.
    pub curve_snr_db: f64,
    /// One-line explanation of the gate outcome.
    pub explanation: String,
}

/// Evidence for one matched aircraft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AircraftEvidence {
    /// ICAO 24-bit address.
    pub icao24: String,
    /// Callsign when the source provided one.
    pub callsign: Option<String>,
    /// ICAO type code when known.
    pub type_code: Option<String>,
    /// Archive the track came from.
    pub source: String,
}

/// A ranked alternative cluster with its margin over the best rival.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlternativeEvidence {
    /// 0-based rank by descending mass.
    pub rank: usize,
    /// Normalized posterior mass in `[0, 1]`.
    pub posterior_mass: f64,
    /// Log-mass margin over the best other cluster, in nats.
    pub margin_nats: f64,
}

/// The public region cell (D-1): a square of at least `size_m` on a side,
/// centred on the best estimate. No exact coordinate appears anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PublicCell {
    /// Cell centre latitude in degrees.
    pub lat_deg: f64,
    /// Cell centre longitude in degrees.
    pub lon_deg: f64,
    /// Cell side length in metres.
    pub size_m: f64,
}

/// What the result says about where the recording was made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    /// No locatable answer; `reason_codes` explains why.
    None,
    /// Public mode: a coarse cell and the winning mass.
    Public {
        /// Coarse output cell.
        cell: PublicCell,
        /// Posterior mass of the winning cluster.
        posterior_mass: f64,
    },
    /// Research mode: exact observer, calibrated ellipse, implied start.
    Research {
        /// Refined observer position.
        observer: LatLon,
        /// Calibrated 95% ellipse semi-axes `(east, north)` covariance.
        ellipse_95: [[f64; 2]; 2],
        /// 95% semi-major axis in metres.
        semi_major_95_m: f64,
        /// Implied recording start, Unix seconds.
        recording_start_unix_s: f64,
        /// Posterior mass of the winning cluster.
        posterior_mass: f64,
    },
}

/// The serialized result payload (SDD 10.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultPayload {
    /// SDD 6.5 verdict.
    pub verdict: Verdict,
    /// Stable reason codes behind the verdict.
    pub reason_codes: Vec<ReasonCode>,
    /// SHA-256 of the run manifest JSON.
    pub manifest_sha256: String,
    /// Per-event quality evidence.
    pub events: Vec<EventEvidence>,
    /// Matched-aircraft evidence.
    pub aircraft: Vec<AircraftEvidence>,
    /// Ranked alternatives with margins.
    pub alternatives: Vec<AlternativeEvidence>,
    /// Location disclosure for the requested mode.
    pub location: Location,
    /// Whether any stage truncated its work budget.
    pub truncated: bool,
    /// Figure links or artifact names produced by the render stage.
    pub figures: Vec<String>,
}

impl ResultPayload {
    /// Build the payload for one finished run.
    ///
    /// `tracks` is the same slice the localize stage saw, so
    /// `matched_track_indices` resolve to aircraft evidence. `location` is
    /// supplied by the caller because public coarsening (PIP-04) and the
    /// calibrated ellipse are computed outside this type; the [`OutputMode`]
    /// the caller intends is implied by the [`Location`] variant chosen.
    #[must_use]
    pub fn build(
        outcome: &AnalyzeOutcome,
        tracks: &[adsb::FlightTrack],
        manifest_sha256: &str,
        location: Location,
        figures: Vec<String>,
    ) -> Self {
        let (verdict, reason_codes, alternatives, truncated, matched) = match &outcome.localize {
            Some(localize) => (
                localize.decision.verdict,
                localize.decision.reason_codes.clone(),
                localize
                    .ranked
                    .iter()
                    .map(|alt| AlternativeEvidence {
                        rank: alt.rank,
                        posterior_mass: alt.cluster.posterior_mass,
                        margin_nats: alt.margin_nats,
                    })
                    .collect(),
                localize.truncated || outcome.truncated,
                localize.matched_track_indices.clone(),
            ),
            None => (
                Verdict::NotLocatable,
                vec![ReasonCode::NoFlightMatch],
                Vec::new(),
                outcome.truncated,
                Vec::new(),
            ),
        };
        Self {
            verdict,
            reason_codes,
            manifest_sha256: manifest_sha256.to_owned(),
            events: outcome.events.iter().map(event_evidence).collect(),
            aircraft: matched
                .iter()
                .filter_map(|index| tracks.get(*index))
                .map(aircraft_evidence)
                .collect(),
            alternatives,
            location,
            truncated,
            figures,
        }
    }

    /// Serialize the payload to JSON.
    ///
    /// # Errors
    ///
    /// Returns a [`serde_json::Error`] if serialization fails.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

fn event_evidence(event: &FittedEvent) -> EventEvidence {
    EventEvidence {
        event_id: event.fit.event_id,
        quality_score: event.assessment.quality.score,
        retained: event.assessment.retained,
        harmonic_count: event.assessment.harmonic_count,
        curve_snr_db: event.assessment.curve_snr_db,
        explanation: event.assessment.explanation(),
    }
}

fn aircraft_evidence(track: &adsb::FlightTrack) -> AircraftEvidence {
    AircraftEvidence {
        icao24: track.icao24.clone(),
        callsign: track.callsign.clone(),
        type_code: track.type_code.clone(),
        source: track.source.as_str().to_owned(),
    }
}
