//! Verdicts and refusal thresholds (FR-SL-02, FR-OUT-01, SDD 6.5).
//!
//! [`VerdictThresholds::classify`] handles the mass axis alone; [`decide`]
//! applies the full SDD 6.5 table: evidence requirements (two independent
//! events, or one event with known time *and* silence support), the 95%
//! ellipse size cap, truncation and veto caps, and reason codes for every
//! refusal. One event with no known time is always `NotLocatable`; mirror
//! pairs stay `Ambiguous` because the rival cluster holds real mass.

use serde::{Deserialize, Serialize};

use crate::error::SearchError;

/// The result of a localization attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Verdict {
    /// The best cluster passes the locatable threshold.
    Locatable,
    /// Evidence supports more than one cluster, or only one weak cluster.
    Ambiguous,
    /// The evidence is insufficient or inconsistent; the honest answer.
    NotLocatable,
}

/// Why a verdict was reached — stable codes for the result payload
/// (SDD 10.4 `reason_codes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReasonCode {
    /// A veto rejected the candidate outright.
    Veto,
    /// No cluster survived scoring (no consistent flight match).
    NoFlightMatch,
    /// Top-cluster posterior mass is below the Ambiguous bar.
    LowPosteriorMass,
    /// A single event with no known recording time can only fix a curve.
    SingleEvent,
    /// One event plus known time but no silence support fixes only a curve.
    CurveOnly,
    /// Fewer than two independent events without the known-time/silence path.
    InsufficientEvents,
    /// The 95% ellipse semi-major axis exceeds 1 km.
    EllipseTooLarge,
    /// A second cluster holds enough mass to keep the answer ambiguous.
    MultipleClusters,
    /// A work budget truncated a stage; the verdict is capped at Ambiguous.
    TruncatedSearch,
}

/// The verdict plus the reason codes that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct VerdictDecision {
    /// The verdict.
    pub verdict: Verdict,
    /// Reason codes, in the order they were evaluated.
    pub reason_codes: Vec<ReasonCode>,
}

/// Everything [`decide`] needs, gathered from the scored clusters, the
/// refined solution and the stage truncation flags.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)] // independent evidence flags, not a state machine
pub struct VerdictInput {
    /// Posterior mass of the best cluster, in `[0, 1]`.
    pub top_mass: f64,
    /// Posterior mass of the best rival cluster; `0.0` when there is none.
    pub rival_mass: f64,
    /// Distinct events covered by the winning assignment.
    pub independent_events: usize,
    /// A known recording time was supplied.
    pub known_time: bool,
    /// The silence term actively supports this candidate.
    pub silence_support: bool,
    /// 95% semi-major axis of the calibrated `(east, north)` ellipse, metres.
    pub ellipse_semi_major_95_m: f64,
    /// Any stage reported a work-budget truncation.
    pub truncated: bool,
    /// A veto rejected this candidate.
    pub veto: bool,
}

/// Posterior-mass thresholds for the verdict.
///
/// Intervals are half-open at the top: `Locatable` is
/// `[locatable_min, 1]`, `Ambiguous` is `[ambiguous_min, locatable_min)`, and
/// `NotLocatable` is `[0, ambiguous_min)`. `rival_min` is the mass a second
/// cluster must hold to cap the verdict at `Ambiguous`. Defaults are
/// provisional until v0 calibration (SDD section 6.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerdictThresholds {
    /// Minimum best-cluster mass for `Locatable`.
    pub locatable_min: f64,
    /// Minimum best-cluster mass for `Ambiguous`.
    pub ambiguous_min: f64,
    /// Minimum rival-cluster mass that counts as "several clusters".
    pub rival_min: f64,
    /// Maximum allowed 95% ellipse semi-major axis, metres.
    pub max_semi_major_95_m: f64,
}

impl Default for VerdictThresholds {
    fn default() -> Self {
        Self {
            locatable_min: 0.9,
            ambiguous_min: 0.5,
            rival_min: 0.05,
            max_semi_major_95_m: 1_000.0,
        }
    }
}

impl VerdictThresholds {
    /// Check that thresholds are finite and ordered.
    pub fn validate(&self) -> Result<(), SearchError> {
        let values = [
            self.locatable_min,
            self.ambiguous_min,
            self.rival_min,
            self.max_semi_major_95_m,
        ];
        if values.iter().any(|v| !v.is_finite()) {
            return Err(SearchError::NotFinite);
        }
        if !(0.0..=1.0).contains(&self.ambiguous_min)
            || !(0.0..=1.0).contains(&self.locatable_min)
            || !(0.0..=1.0).contains(&self.rival_min)
            || self.ambiguous_min > self.locatable_min
            || self.max_semi_major_95_m <= 0.0
        {
            return Err(SearchError::InvalidThresholds);
        }
        Ok(())
    }

    /// Classify a posterior mass using the half-open intervals above.
    ///
    /// The caller must call [`Self::validate`] first.
    pub fn classify(&self, posterior_mass: f64) -> Verdict {
        if posterior_mass >= self.locatable_min {
            Verdict::Locatable
        } else if posterior_mass >= self.ambiguous_min {
            Verdict::Ambiguous
        } else {
            Verdict::NotLocatable
        }
    }
}

/// The 95% semi-major axis of a 2×2 `(east, north)` covariance ellipse, in
/// metres: `sqrt(λ_max · χ²₂(0.95))` with `χ²₂(0.95) = -2 ln 0.05`.
///
/// Returns `f64::INFINITY` for a non-finite or non-positive-definite input.
pub fn ellipse_semi_major_95(ellipse: &[[f64; 2]; 2]) -> f64 {
    let [[e00, e01], [e10, e11]] = *ellipse;
    if ![e00, e01, e10, e11].iter().all(|v| v.is_finite()) {
        return f64::INFINITY;
    }
    let trace = e00 + e11;
    let det = e00 * e11 - e01 * e10;
    let discriminant = (trace * trace - 4.0 * det).max(0.0);
    let lambda_max = f64::midpoint(trace, discriminant.sqrt());
    if lambda_max <= 0.0 {
        return 0.0;
    }
    // χ²₂(0.95) = -2 ln(0.05) ≈ 5.991464547107979.
    (lambda_max * -2.0 * 0.05_f64.ln()).sqrt()
}

/// Apply the full SDD 6.5 verdict table.
///
/// Order of evaluation:
/// 1. `veto` → `NotLocatable`.
/// 2. one event with no known time → `NotLocatable` (always).
/// 3. `top_mass` below the Ambiguous bar → `NotLocatable`.
/// 4. mass classifies `Locatable` or `Ambiguous`; every failed
///    locatable condition adds a reason code and caps at `Ambiguous`.
///
/// # Errors
///
/// Returns [`SearchError::InvalidThresholds`] for bad thresholds and
/// [`SearchError::NotFinite`] for non-finite input masses or ellipse size.
pub fn decide(
    input: &VerdictInput,
    thresholds: &VerdictThresholds,
) -> Result<VerdictDecision, SearchError> {
    thresholds.validate()?;
    if !input.top_mass.is_finite()
        || !input.rival_mass.is_finite()
        || !input.ellipse_semi_major_95_m.is_finite()
        || input.top_mass < 0.0
        || input.top_mass > 1.0
        || input.rival_mass < 0.0
        || input.rival_mass > 1.0
    {
        return Err(SearchError::NotFinite);
    }

    let mut reasons = Vec::new();
    let mut ambiguous_reasons = Vec::new();

    if input.veto {
        reasons.push(ReasonCode::Veto);
        return Ok(VerdictDecision {
            verdict: Verdict::NotLocatable,
            reason_codes: reasons,
        });
    }

    // One event with no known time fixes only a curve — always refused.
    if input.independent_events == 1 && !input.known_time {
        reasons.push(ReasonCode::SingleEvent);
        return Ok(VerdictDecision {
            verdict: Verdict::NotLocatable,
            reason_codes: reasons,
        });
    }

    if input.top_mass < thresholds.ambiguous_min {
        reasons.push(if input.top_mass <= 0.0 {
            ReasonCode::NoFlightMatch
        } else {
            ReasonCode::LowPosteriorMass
        });
        return Ok(VerdictDecision {
            verdict: Verdict::NotLocatable,
            reason_codes: reasons,
        });
    }

    // Locatable conditions; each failure caps at Ambiguous.
    let enough_events = input.independent_events >= 2
        || (input.independent_events == 1 && input.known_time && input.silence_support);
    if !enough_events {
        ambiguous_reasons.push(if input.independent_events == 1 {
            ReasonCode::CurveOnly
        } else {
            ReasonCode::InsufficientEvents
        });
    }
    if input.ellipse_semi_major_95_m > thresholds.max_semi_major_95_m {
        ambiguous_reasons.push(ReasonCode::EllipseTooLarge);
    }
    if input.rival_mass >= thresholds.rival_min {
        ambiguous_reasons.push(ReasonCode::MultipleClusters);
    }
    if input.truncated {
        ambiguous_reasons.push(ReasonCode::TruncatedSearch);
    }

    let verdict = if thresholds.classify(input.top_mass) == Verdict::Locatable
        && ambiguous_reasons.is_empty()
    {
        Verdict::Locatable
    } else {
        reasons.append(&mut ambiguous_reasons);
        Verdict::Ambiguous
    };
    Ok(VerdictDecision {
        verdict,
        reason_codes: reasons,
    })
}
