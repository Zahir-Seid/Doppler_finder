//! SDD 6.3 stage 4 and 6.4: continuous Levenberg–Marquardt refinement of
//! `(east, north, s)` on the negative log posterior, with bounds and the
//! land prior as constraints, and covariance from the inverse Gauss–Newton
//! Hessian (FR-SL-03).
//!
//! The residual vector is the whitened form of the SDD 6.2 terms for the
//! candidate's best assignment: two Cholesky-whitened `(v, d)` residuals
//! and one clock residual per event, plus the known-time residual when a
//! recording time is known. Constants (`log det Σ`, `2π` terms, prior,
//! silence) do not move the optimum but are accumulated so the returned
//! `log_posterior` stays comparable with [`score_candidates`].
//!
//! The silence term is *not* refined: it depends on the full leg set and is
//! evaluated per candidate upstream. A land prior returning `-inf` acts as
//! a constraint — trial steps onto water are rejected like out-of-bounds
//! steps.

use adsb::FlightTrack;
use core_types::{LatLon, Metres};
use events::EventFit;
use geodesy::{ElevationModel, GeoidModel, LocalFrame};

use crate::clock::{implied_recording_start, recording_start_sigma};
use crate::coarse::Hypothesis;
use crate::error::SearchError;
use crate::join::JoinCandidate;
use crate::likelihood::{ErrorBudget, vd_gaussian};
use crate::posterior::{LandPrior, PosteriorEnv, assignment_log_likelihood};
use crate::predict::predict;

/// Bounds and landing constraints for refinement, in the local ENU frame
/// centred on the candidate cell plus absolute bounds on `s`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefineBounds {
    /// Minimum east coordinate in metres.
    pub min_east_m: f64,
    /// Maximum east coordinate in metres.
    pub max_east_m: f64,
    /// Minimum north coordinate in metres.
    pub min_north_m: f64,
    /// Maximum north coordinate in metres.
    pub max_north_m: f64,
    /// Minimum recording start, Unix seconds.
    pub min_s_unix_s: f64,
    /// Maximum recording start, Unix seconds.
    pub max_s_unix_s: f64,
}

impl RefineBounds {
    /// Check that every bound is finite and ordered.
    ///
    /// # Errors
    /// Returns [`SearchError::NotFinite`] on any violation.
    pub fn validate(&self) -> Result<(), SearchError> {
        let values = [
            self.min_east_m,
            self.max_east_m,
            self.min_north_m,
            self.max_north_m,
            self.min_s_unix_s,
            self.max_s_unix_s,
        ];
        if values.iter().any(|v| !v.is_finite())
            || self.min_east_m > self.max_east_m
            || self.min_north_m > self.max_north_m
            || self.min_s_unix_s > self.max_s_unix_s
        {
            return Err(SearchError::NotFinite);
        }
        Ok(())
    }

    fn contains(&self, theta: [f64; 3]) -> bool {
        theta[0] >= self.min_east_m
            && theta[0] <= self.max_east_m
            && theta[1] >= self.min_north_m
            && theta[1] <= self.max_north_m
            && theta[2] >= self.min_s_unix_s
            && theta[2] <= self.max_s_unix_s
    }
}

/// Environment for [`refine`]: track access, atmosphere, priors and bounds.
pub struct RefineEnv<'a> {
    /// Flight tracks indexed by `Hypothesis::track_index`.
    pub tracks: &'a [FlightTrack],
    /// Terrain for observer height.
    pub terrain: &'a dyn ElevationModel,
    /// Geoid for the height datum.
    pub geoid: &'a dyn GeoidModel,
    /// Nominal ear height.
    pub ear_height_m: Metres,
    /// Sound speed from the atmosphere model.
    pub sound_speed_m_per_s: f64,
    /// `(v, d)` error budget.
    pub budget: &'a ErrorBudget,
    /// ADS-B timing sigma, seconds.
    pub adsb_timing_sigma_s: f64,
    /// Optional known recording time `(unix_s, sigma_s)`.
    pub known_time: Option<(f64, f64)>,
    /// Land prior; `-inf` excludes a position.
    pub prior: &'a dyn LandPrior,
    /// Refinement bounds.
    pub bounds: RefineBounds,
}

/// The refined `(observer, s)` solution for one candidate.
#[derive(Debug, Clone)]
pub struct RefinedSolution {
    /// Refined observer position.
    pub observer: LatLon,
    /// Refined recording start, Unix seconds.
    pub s_unix_s: f64,
    /// 3×3 covariance over `(east_m, north_m, s_unix_s)` from the inverse
    /// Gauss–Newton Hessian at the optimum.
    pub covariance: [[f64; 3]; 3],
    /// Log-posterior at the optimum (same normalization as
    /// [`crate::score_candidates`], minus the assignment sum which is
    /// collapsed to the best assignment here).
    pub log_posterior: f64,
    /// LM iterations performed.
    pub iterations: usize,
    /// True when LM converged before the iteration cap.
    pub converged: bool,
}

/// The maximum-likelihood assignment for a candidate: one hypothesis per
/// covered event maximizing the joint `(v, d)` + clock score at the
/// candidate's implied start.
///
/// Returns the hypothesis indices and whether the assignment space was
/// truncated at [`crate::MAX_ASSIGNMENTS`].
///
/// # Errors
///
/// Propagates scoring errors.
pub fn best_assignment(
    candidate: &JoinCandidate,
    hypotheses: &[Hypothesis],
    events: &[EventFit],
    env: &PosteriorEnv<'_>,
) -> Result<(Vec<usize>, bool), SearchError> {
    let option_lists: Vec<Vec<usize>> =
        crate::posterior::assignment_options(candidate, hypotheses)?
            .into_values()
            .collect();
    let total: usize = option_lists.iter().map(Vec::len).product();
    let truncated = total > crate::MAX_ASSIGNMENTS;
    let limit = if truncated {
        crate::MAX_ASSIGNMENTS
    } else {
        total
    };
    let s = candidate.implied_start_unix_s;
    let mut best: Option<(f64, Vec<usize>)> = None;
    let mut current = Vec::with_capacity(option_lists.len());
    let mut visited = 0usize;
    best_assignment_rec(
        &option_lists,
        0,
        hypotheses,
        events,
        s,
        env,
        &mut current,
        &mut best,
        &mut visited,
        limit,
    )?;
    match best {
        Some((_, indices)) => Ok((indices, truncated)),
        None => Err(SearchError::NotFinite),
    }
}

#[allow(clippy::too_many_arguments)]
fn best_assignment_rec(
    option_lists: &[Vec<usize>],
    level: usize,
    hypotheses: &[Hypothesis],
    events: &[EventFit],
    s: f64,
    env: &PosteriorEnv<'_>,
    current: &mut Vec<usize>,
    best: &mut Option<(f64, Vec<usize>)>,
    visited: &mut usize,
    limit: usize,
) -> Result<(), SearchError> {
    if *visited >= limit {
        return Ok(());
    }
    let Some(options) = option_lists.get(level) else {
        *visited += 1;
        let score = assignment_log_likelihood(current, hypotheses, events, s, env)?;
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            *best = Some((score, current.clone()));
        }
        return Ok(());
    };
    for &hyp_index in options {
        current.push(hyp_index);
        best_assignment_rec(
            option_lists,
            level + 1,
            hypotheses,
            events,
            s,
            env,
            current,
            best,
            visited,
            limit,
        )?;
        current.pop();
        if *visited >= limit {
            return Ok(());
        }
    }
    Ok(())
}

/// Residual evaluation context built once per [`refine`] call.
struct ResidualCtx<'a> {
    frame: &'a LocalFrame,
    assignment: &'a [usize],
    hypotheses: &'a [Hypothesis],
    events: &'a [EventFit],
    tracks: &'a [FlightTrack],
    terrain: &'a dyn ElevationModel,
    geoid: &'a dyn GeoidModel,
    ear_height_m: Metres,
    sound_speed_m_per_s: f64,
    budget: &'a ErrorBudget,
    adsb_timing_sigma_s: f64,
    known_time: Option<(f64, f64)>,
    prior: &'a dyn LandPrior,
}

/// Evaluate the whitened residual vector and the constant log-posterior
/// offset at `theta = (east_m, north_m, s_unix_s)`.
///
/// Returns `(residuals, constant_terms)` where `log_posterior =
/// constant_terms − ½‖residuals‖²`.
fn eval_theta(ctx: &ResidualCtx<'_>, theta: [f64; 3]) -> Result<(Vec<f64>, f64), SearchError> {
    let enu = geodesy::Enu {
        east_m: theta[0],
        north_m: theta[1],
        up_m: 0.0,
    };
    let (observer, _height) = ctx
        .frame
        .enu_to_geodetic(enu)
        .map_err(|e| SearchError::Geodesy(e.to_string()))?;

    let mut residuals = Vec::with_capacity(3 * ctx.assignment.len() + 1);
    let mut constants = ctx.prior.log_prior(observer);
    if constants == f64::NEG_INFINITY {
        return Err(SearchError::NotFinite);
    }
    constants += match ctx.known_time {
        Some(_) => -0.5 * (std::f64::consts::TAU).ln(),
        None => 0.0,
    };

    for &hyp_index in ctx.assignment {
        let Some(hypothesis) = ctx.hypotheses.get(hyp_index) else {
            return Err(SearchError::NotFinite);
        };
        let Some(fit) = ctx.events.get(hypothesis.event_index) else {
            return Err(SearchError::NotFinite);
        };
        let Some(track) = ctx.tracks.get(hypothesis.track_index) else {
            return Err(SearchError::NotFinite);
        };
        let Some(prediction) = predict(
            observer,
            track,
            ctx.terrain,
            ctx.geoid,
            ctx.ear_height_m,
            ctx.sound_speed_m_per_s,
        )?
        else {
            return Err(SearchError::Adsb("track is not interpolable".to_owned()));
        };
        let gaussian = vd_gaussian(fit, &prediction, ctx.budget)?;
        residuals.extend_from_slice(&gaussian.whitened_residuals());
        // event_log_likelihood = -0.5·(maha + log det Σ + 2 ln 2π): the
        // constant part is -0.5·log det Σ - ln 2π.
        constants += -0.5 * gaussian.log_determinant() - std::f64::consts::TAU.ln();

        let s_i = implied_recording_start(fit, &prediction)?;
        let sigma_i = recording_start_sigma(fit, ctx.adsb_timing_sigma_s)?;
        if sigma_i <= 0.0 {
            return Err(SearchError::InvalidCovariance);
        }
        residuals.push((theta[2] - s_i) / sigma_i);
        // clock_log_likelihood constant: -0.5·ln 2π per event.
        constants += -0.5 * std::f64::consts::TAU.ln();
    }

    if let Some((s_known, sigma_known)) = ctx.known_time {
        residuals.push((theta[2] - s_known) / sigma_known);
    }

    Ok((residuals, constants))
}

/// Refine a join candidate's `(observer, s)` by Levenberg–Marquardt on the
/// negative log posterior (SDD 6.3 stage 4).
///
/// `assignment` comes from [`best_assignment`]; `candidate` supplies the
/// starting cell and `s`. The ENU frame is centred on the candidate cell,
/// so `theta = (0, 0, s)` is the start. Steps that leave [`RefineBounds`]
/// or land on `-inf` prior are rejected like any failed trial.
///
/// # Errors
///
/// Returns [`SearchError::NotFinite`] for invalid bounds or non-finite
/// evaluations at the start point, [`SearchError::InvalidCovariance`] when
/// the final Hessian is singular, and propagates prediction errors.
pub fn refine(
    candidate: &JoinCandidate,
    assignment: &[usize],
    hypotheses: &[Hypothesis],
    events: &[EventFit],
    env: &RefineEnv<'_>,
) -> Result<RefinedSolution, SearchError> {
    validate_refine_env(env)?;

    let origin = LatLon::new(candidate.cell_lat_deg, candidate.cell_lon_deg);
    let frame = LocalFrame::wgs84(origin).map_err(|e| SearchError::Geodesy(e.to_string()))?;
    let ctx = ResidualCtx {
        frame: &frame,
        assignment,
        hypotheses,
        events,
        tracks: env.tracks,
        terrain: env.terrain,
        geoid: env.geoid,
        ear_height_m: env.ear_height_m,
        sound_speed_m_per_s: env.sound_speed_m_per_s,
        budget: env.budget,
        adsb_timing_sigma_s: env.adsb_timing_sigma_s,
        known_time: env.known_time,
        prior: env.prior,
    };

    let theta = [0.0, 0.0, candidate.implied_start_unix_s];
    if !env.bounds.contains(theta) {
        return Err(SearchError::NotFinite);
    }
    let (residuals, _constants0) = eval_theta(&ctx, theta)?;
    let outcome = lm_loop(&ctx, theta, residuals, &env.bounds)?;
    let theta = outcome.theta;

    // Final residual/Jacobian for covariance and log-posterior.
    let (final_residuals, final_constants) = eval_theta(&ctx, theta)?;
    let jacobian = match outcome.last_jacobian {
        Some(j) => j,
        None => numeric_jacobian(&ctx, theta, [25.0, 25.0, 0.25], &final_residuals)?,
    };
    let (j_t_j, _) = normal_equations(&jacobian, &final_residuals);
    let covariance = invert3(j_t_j).ok_or(SearchError::InvalidCovariance)?;

    let (observer, _) = frame
        .enu_to_geodetic(geodesy::Enu {
            east_m: theta[0],
            north_m: theta[1],
            up_m: 0.0,
        })
        .map_err(|e| SearchError::Geodesy(e.to_string()))?;
    let sse: f64 = final_residuals.iter().map(|r| r * r).sum();
    Ok(RefinedSolution {
        observer,
        s_unix_s: theta[2],
        covariance,
        log_posterior: final_constants - 0.5 * sse,
        iterations: outcome.iterations,
        converged: outcome.converged,
    })
}

/// Outcome of the LM loop: final `theta`, convergence flag, iteration
/// count and the last Jacobian (reused for the covariance Hessian).
struct LmOutcome {
    theta: [f64; 3],
    converged: bool,
    iterations: usize,
    last_jacobian: Option<Vec<[f64; 3]>>,
}

/// Levenberg–Marquardt on `0.5·‖residuals‖²` starting at `theta`, with
/// steps rejected outside `bounds` or when [`eval_theta`] fails (e.g. a
/// `-inf` prior). Converged means a vanishing gradient or a relative cost
/// improvement below `1e-9`; exhausting the damping ladder also stops the
/// loop but leaves `converged` false.
fn lm_loop(
    ctx: &ResidualCtx<'_>,
    mut theta: [f64; 3],
    mut residuals: Vec<f64>,
    bounds: &RefineBounds,
) -> Result<LmOutcome, SearchError> {
    let sumsq = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    let mut cost = 0.5 * sumsq(&residuals);
    let mut lambda = 1e-3;
    let steps = [25.0, 25.0, 0.25]; // numeric Jacobian steps (m, m, s)
    let mut converged = false;
    let mut iterations = 0;
    let mut last_jacobian = None;
    let mut previous_cost = cost;

    for iteration in 0..60 {
        iterations = iteration + 1;
        let jacobian = numeric_jacobian(ctx, theta, steps, &residuals)?;
        last_jacobian = Some(jacobian.clone());
        let (j_t_j, j_t_r) = normal_equations(&jacobian, &residuals);
        // At the floating-point noise floor no trial can improve the cost;
        // a vanishing gradient (∇cost = Jᵀr) means we are stationary.
        if j_t_r.iter().all(|g| g.abs() < 1e-6) {
            converged = true;
            break;
        }

        // Try damped steps until one is accepted.
        let mut accepted = false;
        for _ in 0..12 {
            let damped = damped_normal_matrix(&j_t_j, lambda);
            let Some(delta) = neg_solve3(damped, j_t_r) else {
                lambda *= 10.0;
                continue;
            };
            let trial = [
                theta[0] + delta[0],
                theta[1] + delta[1],
                theta[2] + delta[2],
            ];
            if !bounds.contains(trial) {
                lambda *= 10.0;
                continue;
            }
            let Ok((trial_residuals, _)) = eval_theta(ctx, trial) else {
                lambda *= 10.0;
                continue;
            };
            let trial_cost = 0.5 * sumsq(&trial_residuals);
            if trial_cost < cost {
                theta = trial;
                cost = trial_cost;
                residuals = trial_residuals;
                lambda = (lambda / 10.0).max(1e-9);
                accepted = true;
                break;
            }
            lambda *= 10.0;
        }
        if !accepted {
            break;
        }
        if (previous_cost - cost).abs() < 1e-9 * previous_cost.max(1.0) {
            converged = true;
            break;
        }
        previous_cost = cost;
    }

    Ok(LmOutcome {
        theta,
        converged,
        iterations,
        last_jacobian,
    })
}

/// Validate the numeric environment inputs of [`refine`].
fn validate_refine_env(env: &RefineEnv<'_>) -> Result<(), SearchError> {
    env.bounds.validate()?;
    if !env.sound_speed_m_per_s.is_finite() || env.sound_speed_m_per_s <= 0.0 {
        return Err(SearchError::NotFinite);
    }
    if !env.adsb_timing_sigma_s.is_finite() || env.adsb_timing_sigma_s < 0.0 {
        return Err(SearchError::NotFinite);
    }
    if let Some((s, sigma)) = env.known_time
        && (!s.is_finite() || !sigma.is_finite() || sigma <= 0.0)
    {
        return Err(SearchError::NotFinite);
    }
    Ok(())
}

// --- LM helpers ---------------------------------------------------------
// Fixed 3×3 linear algebra uses destructuring and literal indices only:
// the workspace denies `indexing_slicing`, and every access below is on a
// compile-time-sized array.

/// Central-difference Jacobian of the residual vector at `theta`, one
/// `[f64; 3]` row per residual (`[∂r/∂east, ∂r/∂north, ∂r/∂s]`).
fn numeric_jacobian(
    ctx: &ResidualCtx<'_>,
    theta: [f64; 3],
    steps: [f64; 3],
    base_residuals: &[f64],
) -> Result<Vec<[f64; 3]>, SearchError> {
    let column = |param: usize| -> Result<Vec<f64>, SearchError> {
        let h = match param {
            0 => steps[0],
            1 => steps[1],
            _ => steps[2],
        };
        let mut plus = theta;
        let mut minus = theta;
        match param {
            0 => {
                plus[0] += h;
                minus[0] -= h;
            }
            1 => {
                plus[1] += h;
                minus[1] -= h;
            }
            _ => {
                plus[2] += h;
                minus[2] -= h;
            }
        }
        let (r_plus, _) = eval_theta(ctx, plus)?;
        let (r_minus, _) = eval_theta(ctx, minus)?;
        if r_plus.len() != base_residuals.len() || r_minus.len() != base_residuals.len() {
            return Err(SearchError::NotFinite);
        }
        Ok(r_plus
            .iter()
            .zip(r_minus.iter())
            .map(|(&rp, &rm)| (rp - rm) / (2.0 * h))
            .collect())
    };
    let c0 = column(0)?;
    let c1 = column(1)?;
    let c2 = column(2)?;
    Ok(c0
        .iter()
        .zip(c1.iter())
        .zip(c2.iter())
        .map(|((&a, &b), &c)| [a, b, c])
        .collect())
}

/// `JᵀJ` (3×3) and `Jᵀr` from residual Jacobian rows.
fn normal_equations(jacobian: &[[f64; 3]], residuals: &[f64]) -> ([[f64; 3]; 3], [f64; 3]) {
    let mut jtj = [[0.0; 3]; 3];
    let mut jtr = [0.0; 3];
    for (row, &r) in jacobian.iter().zip(residuals.iter()) {
        let [j0, j1, j2] = *row;
        jtj[0][0] += j0 * j0;
        jtj[0][1] += j0 * j1;
        jtj[0][2] += j0 * j2;
        jtj[1][0] += j1 * j0;
        jtj[1][1] += j1 * j1;
        jtj[1][2] += j1 * j2;
        jtj[2][0] += j2 * j0;
        jtj[2][1] += j2 * j1;
        jtj[2][2] += j2 * j2;
        jtr[0] += j0 * r;
        jtr[1] += j1 * r;
        jtr[2] += j2 * r;
    }
    (jtj, jtr)
}

fn damped_normal_matrix(jtj: &[[f64; 3]; 3], lambda: f64) -> [[f64; 3]; 3] {
    let [[m00, m01, m02], [m10, m11, m12], [m20, m21, m22]] = *jtj;
    [
        [m00 + lambda * m00.max(1e-12), m01, m02],
        [m10, m11 + lambda * m11.max(1e-12), m12],
        [m20, m21, m22 + lambda * m22.max(1e-12)],
    ]
}

/// Inverse of a 3×3 via the adjugate; `None` when singular or non-finite.
fn invert3(a: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let [[a00, a01, a02], [a10, a11, a12], [a20, a21, a22]] = a;
    // Cofactors C[i][j] = (-1)^(i+j) M[i][j].
    let c00 = a11 * a22 - a12 * a21;
    let c01 = a12 * a20 - a10 * a22;
    let c02 = a10 * a21 - a11 * a20;
    let c10 = a02 * a21 - a01 * a22;
    let c11 = a00 * a22 - a02 * a20;
    let c12 = a01 * a20 - a00 * a21;
    let c20 = a01 * a12 - a02 * a11;
    let c21 = a02 * a10 - a00 * a12;
    let c22 = a00 * a11 - a01 * a10;
    let det = a00 * c00 + a01 * c01 + a02 * c02;
    if !det.is_finite() || det.abs() < 1e-300 {
        return None;
    }
    let inv_det = 1.0 / det;
    // Inverse is the transposed cofactor matrix over det.
    Some([
        [c00 * inv_det, c10 * inv_det, c20 * inv_det],
        [c01 * inv_det, c11 * inv_det, c21 * inv_det],
        [c02 * inv_det, c12 * inv_det, c22 * inv_det],
    ])
}

/// `-A⁻¹ g` for a 3×3 `A` and gradient `g`; `None` when `A` is singular.
fn neg_solve3(a: [[f64; 3]; 3], g: [f64; 3]) -> Option<[f64; 3]> {
    let inv = invert3(a)?;
    let [[i00, i01, i02], [i10, i11, i12], [i20, i21, i22]] = inv;
    let [g0, g1, g2] = g;
    Some([
        -(i00 * g0 + i01 * g1 + i02 * g2),
        -(i10 * g0 + i11 * g1 + i12 * g2),
        -(i20 * g0 + i21 * g1 + i22 * g2),
    ])
}
