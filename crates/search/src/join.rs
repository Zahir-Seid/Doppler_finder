//! SDD 6.3 stage 2: pair join on a spatial hash of `(cell, clock bin)`
//! (FR-SL-01).
//!
//! Two events at a known real time gap fix a point; one event fixes only a
//! curve. The join keeps only hypothesis groups where at least two distinct
//! events land in the same observer cell *and* the same clock bin, so the
//! surviving candidates already carry a consistent `(x, s)`.
//!
//! The clock bin width is provisional: it must comfortably exceed the
//! per-event `σ_s,i` or true pairs straddling a bin edge are lost. SRC-07
//! re-scores survivors with the exact Gaussian clock term, so a slightly
//! wide bin costs work, not correctness.

use std::collections::BTreeSet;
use std::collections::HashMap;

use crate::budget::WorkBudget;
use crate::coarse::Hypothesis;
use crate::error::SearchError;

/// PROVISIONAL(T-4/SRC-06): clock bin width in seconds. Fit heard-time
/// sigmas are expected to be ~0.5–2 s, so 4 s spans several σ while keeping
/// bins selective; refine when measured fit covariances exist.
pub const PROVISIONAL_CLOCK_BIN_S: f64 = 4.0;

/// Mean metres per degree of latitude, matching `coarse`.
const METRES_PER_DEG_LAT: f64 = 111_320.0;

/// Hash cell and clock-bin widths for the pair join.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JoinConfig {
    /// Observer cell size in metres. Pass the coarse grid's cell size so
    /// hypotheses generated on the same cell share a key.
    pub cell_size_m: f64,
    /// Reference latitude in degrees for the longitude cell width; use the
    /// grid centre so cells stay roughly square over the region.
    pub reference_lat_deg: f64,
    /// Clock bin width in seconds; [`PROVISIONAL_CLOCK_BIN_S`] unless
    /// validation says otherwise.
    pub clock_bin_s: f64,
}

impl JoinConfig {
    /// Check that all parameters are finite and positive.
    ///
    /// # Errors
    /// Returns [`SearchError::NotFinite`] on any violation.
    pub fn validate(&self) -> Result<(), SearchError> {
        if !self.cell_size_m.is_finite()
            || self.cell_size_m <= 0.0
            || !self.reference_lat_deg.is_finite()
            || self.reference_lat_deg.abs() >= 90.0
            || !self.clock_bin_s.is_finite()
            || self.clock_bin_s <= 0.0
        {
            return Err(SearchError::NotFinite);
        }
        Ok(())
    }
}

/// A `(cell, clock bin)` group shared by at least two distinct events.
#[derive(Debug, Clone)]
pub struct JoinCandidate {
    /// Indices into the hypothesis slice passed to [`join_pairs`].
    pub hypothesis_indices: Vec<usize>,
    /// Distinct event indices covered by the group.
    pub event_indices: BTreeSet<usize>,
    /// Observer cell centre in degrees (lower-left corner plus half a cell).
    pub cell_lat_deg: f64,
    /// See [`Self::cell_lat_deg`].
    pub cell_lon_deg: f64,
    /// Precision-weighted mean implied recording start over the distinct
    /// events, Unix seconds.
    pub implied_start_unix_s: f64,
    /// Standard deviation of the weighted mean, seconds.
    pub clock_sigma_s: f64,
}

/// Spatial hash key: observer cell plus clock bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CellKey {
    lat_cell: i64,
    lon_cell: i64,
    clock_bin: i64,
}

/// Join hypotheses that share an observer cell and clock bin (SDD 6.3
/// stage 2).
///
/// Groups containing hypotheses from fewer than two distinct events are
/// dropped: a single event cannot fix a position. Within a surviving group
/// the implied recording start is the inverse-variance weighted mean over
/// the best (lowest Mahalanobis) hypothesis of each distinct event, so a
/// duplicated event does not double-count its clock term.
///
/// The join stage output: surviving candidates plus the truncation flag
/// (SRC-12). When more than `budget.max_pairs` groups survive, the
/// smallest-`clock_sigma_s` candidates are kept and `truncated` is set.
#[derive(Debug, Clone)]
pub struct JoinOutcome {
    /// Multi-event candidates, sorted as before.
    pub candidates: Vec<JoinCandidate>,
    /// True when `budget.max_pairs` cut the candidate list.
    pub truncated: bool,
}

/// # Errors
///
/// Returns [`SearchError::NotFinite`] for an invalid [`JoinConfig`] or a
/// hypothesis with non-finite `implied_start_unix_s` or `clock_sigma_s`,
/// and [`SearchError::InvalidThresholds`] for a zero [`WorkBudget`] limit.
pub fn join_pairs(
    hypotheses: &[Hypothesis],
    config: &JoinConfig,
    budget: &WorkBudget,
) -> Result<JoinOutcome, SearchError> {
    config.validate()?;
    budget.validate()?;
    let lat_step_deg = config.cell_size_m / METRES_PER_DEG_LAT;
    let metres_per_deg_lon =
        METRES_PER_DEG_LAT * config.reference_lat_deg.to_radians().cos().abs().max(0.01);
    let lon_step_deg = config.cell_size_m / metres_per_deg_lon;

    let mut groups: HashMap<CellKey, Vec<(usize, &Hypothesis)>> = HashMap::new();
    for (index, hypothesis) in hypotheses.iter().enumerate() {
        if !hypothesis.implied_start_unix_s.is_finite()
            || !hypothesis.clock_sigma_s.is_finite()
            || hypothesis.clock_sigma_s <= 0.0
        {
            return Err(SearchError::NotFinite);
        }
        let key = CellKey {
            lat_cell: (hypothesis.observer.lat_deg.0 / lat_step_deg).floor() as i64,
            lon_cell: (hypothesis.observer.lon_deg.0 / lon_step_deg).floor() as i64,
            clock_bin: (hypothesis.implied_start_unix_s / config.clock_bin_s).floor() as i64,
        };
        groups.entry(key).or_default().push((index, hypothesis));
    }

    let mut candidates = Vec::new();
    for (key, members) in groups {
        let mut event_indices = BTreeSet::new();
        for (_, hypothesis) in &members {
            event_indices.insert(hypothesis.event_index);
        }
        if event_indices.len() < 2 {
            continue;
        }

        // Best hypothesis per distinct event for the clock mean.
        let mut best_per_event: HashMap<usize, &Hypothesis> = HashMap::new();
        for (_, hypothesis) in &members {
            match best_per_event.get(&hypothesis.event_index) {
                Some(current) if current.maha_distance <= hypothesis.maha_distance => {}
                _ => {
                    best_per_event.insert(hypothesis.event_index, hypothesis);
                }
            }
        }
        let (mut weight_sum, mut start_sum) = (0.0, 0.0);
        for hypothesis in best_per_event.values() {
            let weight = 1.0 / (hypothesis.clock_sigma_s * hypothesis.clock_sigma_s);
            weight_sum += weight;
            start_sum += weight * hypothesis.implied_start_unix_s;
        }
        candidates.push(JoinCandidate {
            hypothesis_indices: members.iter().map(|(index, _)| *index).collect(),
            event_indices,
            cell_lat_deg: (key.lat_cell as f64 + 0.5) * lat_step_deg,
            cell_lon_deg: (key.lon_cell as f64 + 0.5) * lon_step_deg,
            implied_start_unix_s: start_sum / weight_sum,
            clock_sigma_s: 1.0 / weight_sum.sqrt(),
        });
    }
    candidates.sort_by(|a, b| {
        a.implied_start_unix_s
            .total_cmp(&b.implied_start_unix_s)
            .then(a.cell_lat_deg.total_cmp(&b.cell_lat_deg))
            .then(a.cell_lon_deg.total_cmp(&b.cell_lon_deg))
    });

    // Work budget: keep the most clock-precise candidates when the join
    // overflows; ties break on the deterministic sort key.
    let truncated = candidates.len() > budget.max_pairs;
    if truncated {
        candidates.sort_by(|a, b| {
            a.clock_sigma_s
                .total_cmp(&b.clock_sigma_s)
                .then(a.implied_start_unix_s.total_cmp(&b.implied_start_unix_s))
                .then(a.cell_lat_deg.total_cmp(&b.cell_lat_deg))
                .then(a.cell_lon_deg.total_cmp(&b.cell_lon_deg))
        });
        candidates.truncate(budget.max_pairs);
        candidates.sort_by(|a, b| {
            a.implied_start_unix_s
                .total_cmp(&b.implied_start_unix_s)
                .then(a.cell_lat_deg.total_cmp(&b.cell_lat_deg))
                .then(a.cell_lon_deg.total_cmp(&b.cell_lon_deg))
        });
    }
    Ok(JoinOutcome {
        candidates,
        truncated,
    })
}
