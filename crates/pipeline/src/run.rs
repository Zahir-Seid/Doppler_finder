//! End-to-end stage orchestration (PIP-01, FR-OUT-04, SDD 10.1).
//!
//! [`analyze`] runs `ingest → decode → detect → fit → match → localize` on a
//! synchronous [`JobCtx`], reporting progress, checking cancellation at every
//! stage boundary and inside the fit loop, and timing each stage for the run
//! manifest. The silence term is not yet computed per candidate, so scoring
//! runs with silence evidence absent — an honest gap, not a fabricated term.

use std::path::Path;
use std::time::Instant;

use adsb::FlightTrack;
use core_types::manifest::StageTiming;
use core_types::{CancelReason, JobCtx, UtcMillis};
use dsp::{FrontEndConfig, MediaDecoder, StftConfig, WhiteningConfig, spectrogram, whiten};
use events::{
    DetectAllConfig, DirectConfig, EventFit, QualityAssessment, QualityConfig, detect_events,
    fit_event_direct,
};
use geodesy::GeoidModel;
use search::{
    Calibration, CandidateCluster, CoarseEnv, ErrorBudget, GridSpec, JoinCandidate, JoinConfig,
    LandPrior, PosteriorEnv, RankedAlternative, RefineBounds, RefineEnv, RefinedSolution,
    ScoredCandidate, VerdictDecision, VerdictInput, VerdictThresholds, WorkBudget, best_assignment,
    coarse_hypotheses, decide, ellipse_semi_major_95, ellipsoidal_track, join_pairs,
    ranked_alternatives, refine, score_candidates, top_clusters,
};

use crate::budget::PipelineConfig;
use crate::error::PipelineError;
use crate::progress::ProgressSink;
use crate::stages::Stage;

/// Everything the front-end and event stages need for one recording.
#[derive(Debug, Clone, Copy)]
pub struct FrontEndSpec {
    /// Band-limit and decimation settings for the decoded audio.
    pub front_end: FrontEndConfig,
    /// Spectrogram geometry.
    pub stft: StftConfig,
    /// Whitening settings.
    pub whiten: WhiteningConfig,
    /// Multi-event detection settings.
    pub detect: DetectAllConfig,
    /// Direct parametric fit settings.
    pub fit: DirectConfig,
    /// Quality gate settings.
    pub quality: QualityConfig,
    /// Harmonics passed to the direct estimator.
    pub harmonics: &'static [u32],
}

/// Everything the search stages need for one recording.
#[derive(Clone)]
pub struct LocalizeSpec<'a> {
    /// Candidate flight tracks; baro heights are corrected inside.
    pub tracks: &'a [FlightTrack],
    /// Coarse observer grid.
    pub grid: GridSpec,
    /// Shared coarse-stage environment.
    pub coarse_env: CoarseEnv<'a>,
    /// Pair-join settings.
    pub join: JoinConfig,
    /// `(v, d)` error budget reused by scoring and refinement.
    pub error_budget: &'a ErrorBudget,
    /// ADS-B timing sigma, seconds.
    pub adsb_timing_sigma_s: f64,
    /// Optional known recording time `(unix_s, sigma_s)`.
    pub known_time: Option<(f64, f64)>,
    /// Land prior.
    pub prior: &'a dyn LandPrior,
    /// Refinement bounds.
    pub refine_bounds: RefineBounds,
    /// Work budgets for every enumeration stage.
    pub work: WorkBudget,
    /// Cluster radius for posterior summarization, metres.
    pub cluster_radius_m: f64,
    /// Maximum clusters reported.
    pub top_k: usize,
    /// Verdict thresholds.
    pub verdict: VerdictThresholds,
    /// Ellipse calibration (`κ` and version) applied to the refined
    /// covariance before the verdict sees it.
    pub calibration: Calibration,
}

impl std::fmt::Debug for LocalizeSpec<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalizeSpec")
            .field("tracks", &self.tracks.len())
            .field("grid", &self.grid)
            .field("join", &self.join)
            .field("work", &self.work)
            .field("cluster_radius_m", &self.cluster_radius_m)
            .field("top_k", &self.top_k)
            .field("verdict", &self.verdict)
            .field("calibration", &self.calibration)
            .finish_non_exhaustive()
    }
}

/// All inputs for one [`analyze`] run.
pub struct AnalyzeSpec<'a> {
    /// Decoder used by the ingest stage.
    pub decoder: &'a dyn MediaDecoder,
    /// Path handed to the decoder.
    pub media_path: &'a Path,
    /// Front-end and event settings.
    pub front_end: FrontEndSpec,
    /// Search settings.
    pub localize: LocalizeSpec<'a>,
    /// Pipeline configuration (mode, public cell size).
    pub config: PipelineConfig,
}

impl std::fmt::Debug for AnalyzeSpec<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalyzeSpec")
            .field("media_path", &self.media_path)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// A fitted event together with its quality assessment.
#[derive(Debug, Clone)]
pub struct FittedEvent {
    /// Fitted parameters and covariance.
    pub fit: EventFit,
    /// Quality gate outcome; `retained == false` events are excluded from
    /// localization but kept for the report.
    pub assessment: QualityAssessment,
}

/// The search half of a run: clusters, refinement, verdict.
#[derive(Debug, Clone)]
pub struct LocalizeOutcome {
    /// Scored candidates, best first.
    pub scored: Vec<ScoredCandidate>,
    /// Join candidates feeding the scores.
    pub join_candidates: Vec<JoinCandidate>,
    /// Posterior clusters.
    pub clusters: Vec<CandidateCluster>,
    /// Ranked alternatives with margins.
    pub ranked: Vec<RankedAlternative>,
    /// LM-refined winner, when a cluster existed to refine.
    pub refined: Option<RefinedSolution>,
    /// Calibrated 95% `(east, north)` ellipse from the refined covariance;
    /// `None` when nothing refined.
    pub ellipse_95: Option<[[f64; 2]; 2]>,
    /// SDD 6.5 verdict.
    pub decision: VerdictDecision,
    /// Indices into the caller's track slice for the aircraft in the winning
    /// assignment, sorted and deduplicated. Empty when nothing was refined.
    pub matched_track_indices: Vec<usize>,
    /// Any stage reported a work-budget truncation.
    pub truncated: bool,
}

/// Everything one [`analyze`] call produced.
#[derive(Debug)]
pub struct AnalyzeOutcome {
    /// Accepted detections, strongest first.
    pub detections: Vec<events::Detection>,
    /// Fitted events with quality assessments.
    pub events: Vec<FittedEvent>,
    /// Search outcome, or `None` when no event survived the quality gate.
    pub localize: Option<LocalizeOutcome>,
    /// Per-stage wall times for the run manifest.
    pub stage_timings: Vec<StageTiming>,
    /// Whitened spectrogram-time map the detect stage searched; kept for the
    /// render stage's figures.
    pub zmap: dsp::ZMap,
    /// Any input cap or work budget truncated the run.
    pub truncated: bool,
}

/// Run the synchronous pipeline for one recording (SDD 10.1).
///
/// Stages: ingest, decode, detect, fit, match+localize. The render stage is
/// PIP-05 and reports no work here. Cancellation is checked at every boundary
/// and inside the fit loop; the job deadline is enforced at the same points
/// and the mode wall-time limit after the last stage.
///
/// # Errors
///
/// Returns [`PipelineError::Cancelled`] on cancellation or deadline, and
/// [`PipelineError::StageFailed`] with the failing [`Stage`] otherwise.
pub fn analyze(
    ctx: &JobCtx,
    spec: &AnalyzeSpec<'_>,
    progress: &dyn ProgressSink,
) -> Result<AnalyzeOutcome, PipelineError> {
    spec.localize
        .verdict
        .validate()
        .map_err(|e| PipelineError::InvalidConfig(format!("verdict thresholds: {e}")))?;
    spec.localize
        .refine_bounds
        .validate()
        .map_err(|e| PipelineError::InvalidConfig(format!("refine bounds: {e}")))?;
    let started = Instant::now();
    let mut timings = Vec::new();
    let mut truncated = false;

    // -- ingest ---------------------------------------------------------
    check(ctx)?;
    let stage_start = Instant::now();
    let decoded = spec
        .decoder
        .decode(spec.media_path)
        .map_err(|e| failed(Stage::Ingest, e))?;
    finish(&mut timings, Stage::Ingest, stage_start);
    progress.progress(Stage::Ingest, 1.0);

    // -- decode ---------------------------------------------------------
    check(ctx)?;
    let stage_start = Instant::now();
    let samples = dsp::resample_to_analysis_rate(decoded.samples(), &spec.front_end.front_end)
        .map_err(|e| failed(Stage::Decode, e))?;
    finish(&mut timings, Stage::Decode, stage_start);
    progress.progress(Stage::Decode, 1.0);

    // -- detect ---------------------------------------------------------
    check(ctx)?;
    let stage_start = Instant::now();
    let spectrogram =
        spectrogram(&samples, &spec.front_end.stft).map_err(|e| failed(Stage::Detect, e))?;
    let map = whiten(&spectrogram, &spec.front_end.whiten).map_err(|e| failed(Stage::Detect, e))?;
    let detections =
        detect_events(&map, &spec.front_end.detect).map_err(|e| failed(Stage::Detect, e))?;
    if detections.len() >= spec.front_end.detect.max_events {
        truncated = true;
    }
    finish(&mut timings, Stage::Detect, stage_start);
    progress.progress(Stage::Detect, 1.0);

    // -- fit ------------------------------------------------------------
    check(ctx)?;
    let stage_start = Instant::now();
    let mut fitted = Vec::with_capacity(detections.len());
    for detection in &detections {
        check(ctx)?;
        let report = fit_event_direct(
            &map,
            core_types::EventId::new_v7(),
            &detection.hypothesis,
            spec.front_end.harmonics,
            &spec.front_end.fit,
        )
        .map_err(|e| failed(Stage::Fit, e))?;
        let assessment = events::assess(&map, &report, &spec.front_end.quality)
            .map_err(|e| failed(Stage::Fit, e))?;
        fitted.push(FittedEvent {
            fit: report.fit,
            assessment,
        });
    }
    finish(&mut timings, Stage::Fit, stage_start);
    progress.progress(Stage::Fit, 1.0);

    // -- match + localize ----------------------------------------------
    let retained: Vec<EventFit> = fitted
        .iter()
        .filter(|event| event.assessment.retained)
        .map(|event| event.fit)
        .collect();
    let localize = if retained.is_empty() {
        progress.progress(Stage::Match, 1.0);
        progress.progress(Stage::Localize, 1.0);
        None
    } else {
        check(ctx)?;
        let outcome = localize(&retained, spec)?;
        truncated |= outcome.truncated;
        progress.progress(Stage::Match, 1.0);
        progress.progress(Stage::Localize, 1.0);
        Some(outcome)
    };

    // Wall-time budget: research 300 s, service 120 s (SDD 11.3).
    if started.elapsed().as_secs_f64() > spec.config.mode.wall_time_limit_s() {
        return Err(PipelineError::Cancelled(CancelReason::Deadline));
    }

    Ok(AnalyzeOutcome {
        detections,
        events: fitted,
        localize,
        stage_timings: timings,
        zmap: map,
        truncated,
    })
}

/// Compose the search stages for one retained-event set (SDD 6.3).
fn localize(events: &[EventFit], spec: &AnalyzeSpec<'_>) -> Result<LocalizeOutcome, PipelineError> {
    let l = &spec.localize;
    let geoid = l.coarse_env.geoid;
    let tracks: Vec<FlightTrack> = l
        .tracks
        .iter()
        .map(|track| ellipsoidal_track(track, geoid))
        .collect::<Result<_, _>>()
        .map_err(|e| failed(Stage::Match, e))?;

    let coarse = coarse_hypotheses(events, &tracks, &l.grid, &l.coarse_env, &l.work)
        .map_err(|e| failed(Stage::Match, e))?;
    let join =
        join_pairs(&coarse.hypotheses, &l.join, &l.work).map_err(|e| failed(Stage::Match, e))?;

    // Silence evidence is per-candidate and not yet computed; `None` is the
    // honest "no silence term" entry (SRC-04 semantics).
    let silence = vec![None; join.candidates.len()];
    let posterior_env = PosteriorEnv {
        budget: l.error_budget,
        adsb_timing_sigma_s: l.adsb_timing_sigma_s,
        known_time: l.known_time,
        silence: &silence,
        prior: l.prior,
    };
    let scored = score_candidates(
        &join.candidates,
        &coarse.hypotheses,
        events,
        &posterior_env,
        &l.work,
    )
    .map_err(|e| failed(Stage::Localize, e))?;
    let clusters = top_clusters(
        &scored.candidates,
        &join.candidates,
        l.top_k,
        l.cluster_radius_m,
    )
    .map_err(|e| failed(Stage::Localize, e))?;
    let ranked = ranked_alternatives(&clusters);

    // Refine the best member of the best cluster; remember how many distinct
    // events its assignment covers and which tracks it matched.
    let (refined, independent_events, matched_track_indices) = refine_winner(
        &clusters,
        &scored.candidates,
        &join.candidates,
        &coarse.hypotheses,
        events,
        &posterior_env,
        &tracks,
        l,
        geoid,
    );

    let (top_mass, rival_mass) = match clusters.as_slice() {
        [] => (0.0, 0.0),
        [first] => (first.posterior_mass, 0.0),
        [first, second, ..] => (first.posterior_mass, second.posterior_mass),
    };
    // The calibrated ellipse feeds both the verdict and the figures.
    let ellipse_95 = refined
        .as_ref()
        .map(|solution| l.calibration.ellipse_covariance(&solution.covariance));
    // Without a refined solution the ellipse is unbounded; `f64::MAX` stays
    // finite for `decide` and always exceeds the semi-major cap.
    let ellipse_m = ellipse_95.map_or(f64::MAX, |ellipse| ellipse_semi_major_95(&ellipse));
    let truncated = coarse.truncated || join.truncated || scored.truncated;
    let decision = decide(
        &VerdictInput {
            top_mass,
            rival_mass,
            independent_events,
            known_time: l.known_time.is_some(),
            silence_support: false,
            ellipse_semi_major_95_m: ellipse_m,
            truncated,
            veto: false,
        },
        &l.verdict,
    )
    .map_err(|e| failed(Stage::Localize, e))?;

    Ok(LocalizeOutcome {
        scored: scored.candidates,
        join_candidates: join.candidates,
        clusters,
        ranked,
        refined,
        ellipse_95,
        decision,
        matched_track_indices,
        truncated,
    })
}

/// LM-refine the best member of the best cluster.
///
/// Returns the refined solution, the number of distinct events its
/// assignment covers and the sorted track indices it matched; empty results
/// when there is nothing to refine or the assignment/refinement fails — a
/// failed refinement is uncertainty, not an error, because the coarse answer
/// still stands.
#[allow(clippy::too_many_arguments)]
fn refine_winner(
    clusters: &[CandidateCluster],
    scored: &[ScoredCandidate],
    join_candidates: &[JoinCandidate],
    hypotheses: &[search::Hypothesis],
    events: &[EventFit],
    posterior_env: &PosteriorEnv<'_>,
    tracks: &[FlightTrack],
    l: &LocalizeSpec<'_>,
    geoid: &dyn GeoidModel,
) -> (Option<RefinedSolution>, usize, Vec<usize>) {
    let empty = || (None, 0, Vec::new());
    let Some(cluster) = clusters.first() else {
        return empty();
    };
    let Some(&member) = cluster.member_indices.first() else {
        return empty();
    };
    let Some(scored_candidate) = scored.get(member) else {
        return empty();
    };
    let Some(candidate) = join_candidates.get(scored_candidate.candidate_index) else {
        return empty();
    };
    let Ok((assignment, _)) = best_assignment(candidate, hypotheses, events, posterior_env) else {
        return empty();
    };
    let covered = assignment.len();
    let mut matched: Vec<usize> = assignment
        .iter()
        .filter_map(|index| hypotheses.get(*index))
        .map(|hypothesis| hypothesis.track_index)
        .collect();
    matched.sort_unstable();
    matched.dedup();
    let refine_env = RefineEnv {
        tracks,
        terrain: l.coarse_env.terrain,
        geoid,
        ear_height_m: l.coarse_env.ear_height_m,
        sound_speed_m_per_s: l.coarse_env.sound_speed_m_per_s,
        budget: l.error_budget,
        adsb_timing_sigma_s: l.adsb_timing_sigma_s,
        known_time: l.known_time,
        prior: l.prior,
        bounds: l.refine_bounds,
    };
    (
        refine(candidate, &assignment, hypotheses, events, &refine_env).ok(),
        covered,
        matched,
    )
}

/// Cooperative-cancellation and deadline check at a stage boundary.
fn check(ctx: &JobCtx) -> Result<(), PipelineError> {
    if ctx.cancel.is_cancelled() {
        return Err(PipelineError::Cancelled(CancelReason::Client));
    }
    if let Some(deadline) = ctx.deadline
        && UtcMillis::now().get() > deadline.get()
    {
        return Err(PipelineError::Cancelled(CancelReason::Deadline));
    }
    Ok(())
}

/// Record one stage's wall time.
fn finish(timings: &mut Vec<StageTiming>, stage: Stage, started: Instant) {
    timings.push(StageTiming {
        stage: stage.as_str().to_owned(),
        elapsed_s: started.elapsed().as_secs_f64(),
    });
}

/// Wrap a stage error with its stage identity.
fn failed(stage: Stage, error: impl std::fmt::Display) -> PipelineError {
    PipelineError::StageFailed {
        stage,
        message: error.to_string(),
    }
}
