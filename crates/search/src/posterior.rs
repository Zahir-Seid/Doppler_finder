//! SDD 6.3 stages 3–4 and 6.2: full log-posterior over join survivors,
//! log-sum-exp over event→flight assignments, top-K clusters and the land
//! prior (FR-SL-01).
//!
//! For each [`JoinCandidate`] the score is
//!
//! ```text
//! logsumexp_a [ Σ_i log L_i(x, a_i) + Σ_i log N(s; s_i, σ²_s,i)
//!              + log L_silence + log π(x) ]
//! ```
//!
//! evaluated at the candidate's cell centre and precision-weighted
//! recording start `s`. `L_i` is the SRC-02 `(v, d)` Gaussian and the clock
//! terms are the SRC-03 Gaussians; the silence term is supplied per
//! candidate by the caller because it needs the leg set (SRC-04 owns the
//! model). `π` is a land prior — uniform over land until a land mask
//! exists, injected as a trait object so the mask can land later without
//! changing this module.
//!
//! Assignments are enumerated as the cartesian product of each covered
//! event's track hypotheses inside the candidate. The product is capped at
//! [`MAX_ASSIGNMENTS`]; a candidate that exceeds it is scored on a
//! deterministic prefix and flagged `truncated`, which SRC-12 turns into a
//! verdict cap — never a silent cut.

use std::collections::BTreeMap;

use core_types::LatLon;
use events::EventFit;

use crate::clock::clock_log_likelihood;
use crate::coarse::Hypothesis;
use crate::error::SearchError;
use crate::join::JoinCandidate;
use crate::likelihood::{ErrorBudget, event_log_likelihood};

/// Most assignments enumerated per candidate before the candidate is
/// truncated. Provisional(SRC-07/SRC-12): real groups are small; the cap
/// only guards pathological inputs.
pub const MAX_ASSIGNMENTS: usize = 256;

/// Number of top-scoring clusters returned by default.
pub const DEFAULT_TOP_K: usize = 8;

/// Cluster radius in metres for merging nearby candidates.
///
/// PROVISIONAL(T-4/SRC-07): the coarse cell is the natural cluster scale;
/// refine when the fine grid (SRC-08) defines its own neighbourhood.
pub const PROVISIONAL_CLUSTER_RADIUS_M: f64 = 1_000.0;

/// A land prior over observer position.
///
/// The SDD 6.2 prior is uniform over land in the region. No land mask
/// exists yet, so [`UniformPrior`] is the default; a real mask implements
/// this trait and returns `log π(x)` (0 for land, `-inf` for water).
pub trait LandPrior {
    /// `log π(observer)`; may return `-inf` to exclude a position.
    fn log_prior(&self, observer: LatLon) -> f64;
}

/// Uniform prior: every in-region position is equally likely a priori.
#[derive(Debug, Clone, Copy, Default)]
pub struct UniformPrior;

impl LandPrior for UniformPrior {
    fn log_prior(&self, _observer: LatLon) -> f64 {
        0.0
    }
}

/// One candidate after full log-posterior scoring.
#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    /// The join candidate this score belongs to (index into the input
    /// slice is implicit by position).
    pub candidate_index: usize,
    /// Log-posterior up to the shared normalization constant.
    pub log_posterior: f64,
    /// Assignments enumerated before log-sum-exp.
    pub assignments_evaluated: usize,
    /// True when the assignment product exceeded [`MAX_ASSIGNMENTS`] and
    /// only a deterministic prefix was scored.
    pub truncated: bool,
}

/// A cluster of nearby candidates with its total posterior mass.
#[derive(Debug, Clone)]
pub struct CandidateCluster {
    /// Indices into the scored-candidate slice, best first.
    pub member_indices: Vec<usize>,
    /// `log Σ_i exp(score_i)` over members — the cluster's unnormalized
    /// log mass.
    pub log_mass: f64,
    /// Posterior mass in `[0, 1]` after normalization over all clusters.
    pub posterior_mass: f64,
    /// Best member's log-posterior.
    pub best_log_posterior: f64,
}

/// Shared scoring inputs for [`score_candidates`].
pub struct PosteriorEnv<'a> {
    /// `(v, d)` error budget.
    pub budget: &'a ErrorBudget,
    /// ADS-B timing sigma for clock terms, seconds.
    pub adsb_timing_sigma_s: f64,
    /// Optional known recording time `(unix_s, sigma_s)`.
    pub known_time: Option<(f64, f64)>,
    /// Silence log-likelihood per candidate index; `None` entries mean no
    /// silence evidence was computed for that candidate.
    pub silence: &'a [Option<f64>],
    /// Land prior.
    pub prior: &'a dyn LandPrior,
}

/// Score every join candidate with the full log-posterior and return the
/// candidates sorted by descending score.
///
/// `hypotheses` and `events` are the slices the join ran over;
/// `candidate.hypothesis_indices` index into `hypotheses`, and each
/// hypothesis's `event_index`/`track_index` select the `EventFit` and the
/// prediction used in the likelihoods.
///
/// # Errors
///
/// Returns [`SearchError::NotFinite`] for invalid env values, and
/// propagates likelihood errors from malformed fits or predictions.
pub fn score_candidates(
    candidates: &[JoinCandidate],
    hypotheses: &[Hypothesis],
    events: &[EventFit],
    env: &PosteriorEnv<'_>,
) -> Result<Vec<ScoredCandidate>, SearchError> {
    if !env.adsb_timing_sigma_s.is_finite() || env.adsb_timing_sigma_s < 0.0 {
        return Err(SearchError::NotFinite);
    }
    if let Some((s, sigma)) = env.known_time
        && (!s.is_finite() || !sigma.is_finite() || sigma <= 0.0)
    {
        return Err(SearchError::NotFinite);
    }
    if env.silence.len() != candidates.len() {
        return Err(SearchError::NotFinite);
    }

    let mut scored = Vec::with_capacity(candidates.len());
    for (candidate_index, candidate) in candidates.iter().enumerate() {
        let observer = LatLon::new(candidate.cell_lat_deg, candidate.cell_lon_deg);
        let log_prior = env.prior.log_prior(observer);
        if log_prior == f64::NEG_INFINITY {
            // Excluded by the prior: score -inf honestly rather than drop.
            scored.push(ScoredCandidate {
                candidate_index,
                log_posterior: f64::NEG_INFINITY,
                assignments_evaluated: 0,
                truncated: false,
            });
            continue;
        }

        // One option list per distinct event: each entry is a hypothesis
        // index. Multiple hypotheses of the same event are alternative
        // track assignments at this cell.
        let mut options_per_event: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &hyp_index in &candidate.hypothesis_indices {
            let Some(hypothesis) = hypotheses.get(hyp_index) else {
                return Err(SearchError::NotFinite);
            };
            options_per_event
                .entry(hypothesis.event_index)
                .or_default()
                .push(hyp_index);
        }
        // Deterministic order: lowest Mahalanobis first so truncation keeps
        // the best assignments.
        for options in options_per_event.values_mut() {
            options.sort_by(|&a, &b| {
                let maha = |index: usize| {
                    hypotheses
                        .get(index)
                        .map_or(f64::INFINITY, |h| h.maha_distance)
                };
                maha(a).total_cmp(&maha(b))
            });
        }

        let s = candidate.implied_start_unix_s;
        let silence_term = env
            .silence
            .get(candidate_index)
            .copied()
            .flatten()
            .unwrap_or(0.0);
        let known_term = match env.known_time {
            Some((s_known, sigma)) => crate::clock::known_time_log_likelihood(s, s_known, sigma)?,
            None => 0.0,
        };

        let option_lists: Vec<Vec<usize>> = options_per_event.into_values().collect();
        let (log_posterior, assignments_evaluated, truncated) = enumerate_assignments(
            &option_lists,
            hypotheses,
            events,
            s,
            env,
            silence_term + known_term + log_prior,
        )?;
        scored.push(ScoredCandidate {
            candidate_index,
            log_posterior,
            assignments_evaluated,
            truncated,
        });
    }

    scored.sort_by(|a, b| b.log_posterior.total_cmp(&a.log_posterior));
    Ok(scored)
}

/// Cartesian product over per-event options, log-sum-exp of the joint
/// score. Returns `(log_posterior, assignments_evaluated, truncated)`.
fn enumerate_assignments(
    option_lists: &[Vec<usize>],
    hypotheses: &[Hypothesis],
    events: &[EventFit],
    s_unix_s: f64,
    env: &PosteriorEnv<'_>,
    constant_terms: f64,
) -> Result<(f64, usize, bool), SearchError> {
    let total: usize = option_lists.iter().map(Vec::len).product();
    let truncated = total > MAX_ASSIGNMENTS;
    let limit = if truncated { MAX_ASSIGNMENTS } else { total };

    // Depth-first enumeration with early cap; the per-event option lists are
    // already sorted best-first, so the prefix is the best assignments.
    let mut log_terms = Vec::with_capacity(limit);
    let mut current: Vec<usize> = Vec::with_capacity(option_lists.len());
    enumerate_rec(
        option_lists,
        0,
        hypotheses,
        events,
        s_unix_s,
        env,
        &mut current,
        &mut log_terms,
        limit,
    )?;

    // log-sum-exp over assignment scores.
    let max = log_terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if max == f64::NEG_INFINITY {
        return Ok((f64::NEG_INFINITY, log_terms.len(), truncated));
    }
    let sum: f64 = log_terms.iter().map(|term| (term - max).exp()).sum();
    Ok((max + sum.ln() + constant_terms, log_terms.len(), truncated))
}

#[allow(clippy::too_many_arguments)]
fn enumerate_rec(
    option_lists: &[Vec<usize>],
    level: usize,
    hypotheses: &[Hypothesis],
    events: &[EventFit],
    s_unix_s: f64,
    env: &PosteriorEnv<'_>,
    current: &mut Vec<usize>,
    out: &mut Vec<f64>,
    limit: usize,
) -> Result<(), SearchError> {
    if out.len() >= limit {
        return Ok(());
    }
    let Some(options) = option_lists.get(level) else {
        // Leaf: score this assignment.
        let mut log_l = 0.0;
        for &hyp_index in current.iter() {
            let Some(hypothesis) = hypotheses.get(hyp_index) else {
                return Err(SearchError::NotFinite);
            };
            let Some(fit) = events.get(hypothesis.event_index) else {
                return Err(SearchError::NotFinite);
            };
            log_l += event_log_likelihood(fit, &hypothesis.prediction, env.budget)?;
            log_l += clock_log_likelihood(
                fit,
                &hypothesis.prediction,
                s_unix_s,
                env.adsb_timing_sigma_s,
            )?;
        }
        out.push(log_l);
        return Ok(());
    };
    for &hyp_index in options {
        current.push(hyp_index);
        enumerate_rec(
            option_lists,
            level + 1,
            hypotheses,
            events,
            s_unix_s,
            env,
            current,
            out,
            limit,
        )?;
        current.pop();
        if out.len() >= limit {
            return Ok(());
        }
    }
    Ok(())
}

/// Cluster scored candidates by observer proximity and normalize masses.
///
/// Candidates are processed best-first; a candidate joins the first
/// cluster whose members are all within `cluster_radius_m` of the cluster's
/// running centroid, otherwise it starts a new cluster. Only the top `k`
/// clusters by mass are returned.
///
/// # Errors
///
/// Returns [`SearchError::NotFinite`] for a non-finite radius or `k == 0`.
pub fn top_clusters(
    scored: &[ScoredCandidate],
    candidates: &[JoinCandidate],
    k: usize,
    cluster_radius_m: f64,
) -> Result<Vec<CandidateCluster>, SearchError> {
    if !cluster_radius_m.is_finite() || cluster_radius_m <= 0.0 || k == 0 {
        return Err(SearchError::NotFinite);
    }
    // Candidate observer position in a local metre frame relative to the
    // first candidate, for cheap distance checks.
    let origin = candidates
        .first()
        .map(|c| LatLon::new(c.cell_lat_deg, c.cell_lon_deg));
    let frame = origin.map(geodesy::LocalFrame::wgs84);

    let mut clusters: Vec<(f64, f64, Vec<usize>)> = Vec::new(); // (sum_e, sum_n, members)
    for (scored_index, scored_candidate) in scored.iter().enumerate() {
        if scored_candidate.log_posterior == f64::NEG_INFINITY {
            continue;
        }
        let Some(candidate) = candidates.get(scored_candidate.candidate_index) else {
            return Err(SearchError::NotFinite);
        };
        let observer = LatLon::new(candidate.cell_lat_deg, candidate.cell_lon_deg);
        let (east, north) = match &frame {
            Some(Ok(frame)) => {
                let enu = frame
                    .to_enu(observer, 0.0)
                    .map_err(|e| SearchError::Geodesy(e.to_string()))?;
                (enu.east_m, enu.north_m)
            }
            _ => (0.0, 0.0),
        };

        let mut placed = false;
        for (sum_e, sum_n, members) in &mut clusters {
            let count = members.len() as f64;
            let cx = *sum_e / count;
            let cy = *sum_n / count;
            if ((east - cx).powi(2) + (north - cy).powi(2)).sqrt() <= cluster_radius_m {
                *sum_e += east;
                *sum_n += north;
                members.push(scored_index);
                placed = true;
                break;
            }
        }
        if !placed {
            clusters.push((east, north, vec![scored_index]));
        }
    }

    // Cluster log-mass via logsumexp of member scores; sort desc, keep k.
    let mut cluster_scored: Vec<(f64, Vec<usize>)> = clusters
        .into_iter()
        .map(|(_, _, members)| {
            let max = members
                .iter()
                .filter_map(|&i| scored.get(i))
                .map(|s| s.log_posterior)
                .fold(f64::NEG_INFINITY, f64::max);
            let log_mass = if max == f64::NEG_INFINITY {
                f64::NEG_INFINITY
            } else {
                let sum: f64 = members
                    .iter()
                    .filter_map(|&i| scored.get(i))
                    .map(|s| (s.log_posterior - max).exp())
                    .sum();
                max + sum.ln()
            };
            (log_mass, members)
        })
        .collect();
    cluster_scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    cluster_scored.truncate(k);

    // Normalize masses over the returned clusters.
    let max_mass = cluster_scored
        .iter()
        .map(|(m, _)| *m)
        .fold(f64::NEG_INFINITY, f64::max);
    let total: f64 = cluster_scored
        .iter()
        .map(|(m, _)| (*m - max_mass).exp())
        .sum();
    let norm = if total > 0.0 {
        max_mass + total.ln()
    } else {
        f64::NEG_INFINITY
    };

    let mut out = Vec::with_capacity(cluster_scored.len());
    for (log_mass, members) in cluster_scored {
        let best = members
            .iter()
            .filter_map(|&i| scored.get(i))
            .map(|s| s.log_posterior)
            .fold(f64::NEG_INFINITY, f64::max);
        out.push(CandidateCluster {
            member_indices: members,
            log_mass,
            posterior_mass: if norm == f64::NEG_INFINITY {
                0.0
            } else {
                (log_mass - norm).exp()
            },
            best_log_posterior: best,
        });
    }
    Ok(out)
}
