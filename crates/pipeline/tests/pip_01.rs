//! PIP-01: stage orchestration — ingest, decode, detect, fit, match,
//! localize run under a `JobCtx` with progress, timings and cooperative
//! cancellation. Offline and deterministic: the decoder is an in-memory
//! fixture and the localize stage gets no tracks, so the honest outcome is
//! "no locatable answer".

#![allow(clippy::expect_used)]

use std::path::Path;
use std::sync::Mutex;

use adsb::FlightTrack;
use core_types::{Cancel, JobCtx, JobId, UtcMillis};
use dsp::{DecodedAudio, DspError, MediaDecoder};
use events::{DetectAllConfig, DirectConfig, QualityConfig};
use geodesy::{ElevationModel, GeodesyError, GeoidModel};
use pipeline::{
    AnalyzeSpec, FrontEndSpec, LocalizeSpec, PipelineConfig, PipelineError, ProgressSink, Stage,
    analyze,
};
use search::{
    Calibration, CoarseEnv, ErrorBudget, GridSpec, JoinConfig, LandPrior, RefineBounds,
    UniformPrior, Verdict, VerdictThresholds, WorkBudget,
};

const RATE_HZ: f64 = 4_000.0;
const SECONDS: f64 = 4.0;

#[derive(Debug)]
struct FakeDecoder {
    samples: Vec<f32>,
}

impl MediaDecoder for FakeDecoder {
    fn decode(&self, _path: &Path) -> Result<DecodedAudio, DspError> {
        DecodedAudio::new(self.samples.clone(), RATE_HZ, Some("fake-1".to_owned()))
    }
}

#[derive(Debug, Default)]
struct RecordingProgress {
    seen: Mutex<Vec<(Stage, f64)>>,
}

impl ProgressSink for RecordingProgress {
    fn progress(&self, stage: Stage, fraction: f64) {
        self.seen
            .lock()
            .expect("progress lock")
            .push((stage, fraction));
    }
}

#[derive(Debug)]
struct FlatTerrain;

impl ElevationModel for FlatTerrain {
    fn elevation_m(&self, _position: core_types::LatLon) -> Result<f64, GeodesyError> {
        Ok(100.0)
    }
}

#[derive(Debug)]
struct ZeroGeoid;

impl GeoidModel for ZeroGeoid {
    fn undulation_m(&self, _position: core_types::LatLon) -> Result<f64, GeodesyError> {
        Ok(0.0)
    }
}

/// Deterministic white noise in `[-1, 1]` — no Doppler content.
fn noise(count: usize) -> Vec<f32> {
    let mut state = 0x9E37_79B9_u32 | 1;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (f64::from(state) / f64::from(u32::MAX)) * 2.0 - 1.0
        })
        .map(|v| (v * 0.02) as f32)
        .collect()
}

fn ctx(cancel: Cancel, deadline: Option<UtcMillis>) -> JobCtx {
    JobCtx {
        job_id: JobId::new_v7(),
        cancel,
        deadline,
        scratch_dir: std::path::PathBuf::from("/tmp"),
    }
}

fn spec<'a>(
    decoder: &'a FakeDecoder,
    tracks: &'a [FlightTrack],
    terrain: &'a FlatTerrain,
    geoid: &'a ZeroGeoid,
    budget: &'a ErrorBudget,
    prior: &'a dyn LandPrior,
) -> AnalyzeSpec<'a> {
    AnalyzeSpec {
        decoder,
        media_path: Path::new("noise.wav"),
        front_end: FrontEndSpec {
            front_end: dsp::FrontEndConfig::for_input_rate(RATE_HZ),
            stft: dsp::StftConfig::survey(RATE_HZ).expect("survey geometry"),
            whiten: dsp::WhiteningConfig::default(),
            detect: DetectAllConfig {
                detector: events::DetectorConfig {
                    speeds_m_per_s: events::GridRange::new(40.0, 100.0, 20.0),
                    closest_m: events::GridRange::new(200.0, 1_200.0, 200.0),
                    fundamental_hz: events::GridRange::new(60.0, 300.0, 60.0),
                    ..events::DetectorConfig::default()
                },
                max_events: 4,
                mask_half_width_bins: 4,
            },
            fit: DirectConfig::default(),
            quality: QualityConfig::default(),
            harmonics: &[1, 2],
        },
        localize: LocalizeSpec {
            tracks,
            grid: GridSpec {
                min_lat_deg: 51.9,
                max_lat_deg: 52.1,
                min_lon_deg: 12.9,
                max_lon_deg: 13.1,
                cell_size_m: 1_000.0,
            },
            coarse_env: CoarseEnv {
                terrain,
                geoid,
                ear_height_m: core_types::Metres(1.6),
                sound_speed_m_per_s: 343.0,
                budget,
                gate_sigma: search::DEFAULT_GATE_SIGMA,
                adsb_timing_sigma_s: 0.5,
            },
            join: JoinConfig {
                cell_size_m: 500.0,
                reference_lat_deg: 52.0,
                clock_bin_s: 4.0,
            },
            error_budget: budget,
            adsb_timing_sigma_s: 0.5,
            known_time: None,
            prior,
            refine_bounds: RefineBounds {
                min_east_m: -20_000.0,
                max_east_m: 20_000.0,
                min_north_m: -20_000.0,
                max_north_m: 20_000.0,
                min_s_unix_s: 0.0,
                max_s_unix_s: 4_102_444_800.0,
            },
            work: WorkBudget::default(),
            cluster_radius_m: search::PROVISIONAL_CLUSTER_RADIUS_M,
            top_k: search::DEFAULT_TOP_K,
            verdict: VerdictThresholds::default(),
            calibration: Calibration::provisional(),
        },
        config: PipelineConfig::default(),
    }
}

#[test]
fn pip_01_noise_run_completes_with_timings_and_no_location() {
    let decoder = FakeDecoder {
        samples: noise((SECONDS * RATE_HZ) as usize),
    };
    let tracks: Vec<FlightTrack> = Vec::new();
    let terrain = FlatTerrain;
    let geoid = ZeroGeoid;
    let budget = ErrorBudget::default();
    let prior = UniformPrior;
    let spec = spec(&decoder, &tracks, &terrain, &geoid, &budget, &prior);
    let progress = RecordingProgress::default();

    let outcome = analyze(&ctx(Cancel::new(), None), &spec, &progress).expect("analyze succeeds");

    // Front-end stages always run and are timed.
    let stages: Vec<&str> = outcome
        .stage_timings
        .iter()
        .map(|timing| timing.stage.as_str())
        .collect();
    for expected in ["ingest", "decode", "detect", "fit"] {
        assert!(
            stages.contains(&expected),
            "missing stage {expected}: {stages:?}"
        );
    }
    assert!(outcome.stage_timings.iter().all(|t| t.elapsed_s >= 0.0));

    // Progress was reported for the same stages.
    let seen = progress.seen.lock().expect("progress lock");
    for expected in [Stage::Ingest, Stage::Decode, Stage::Detect, Stage::Fit] {
        assert!(
            seen.iter().any(|(stage, _)| *stage == expected),
            "no progress for {expected:?}"
        );
    }
    drop(seen);

    // Noise produces no retained events; the honest result is no location.
    if let Some(localize) = &outcome.localize {
        assert_eq!(localize.decision.verdict, Verdict::NotLocatable);
    }
    assert!(!outcome.truncated || outcome.localize.is_some());
}

#[test]
fn pip_01_cancellation_before_start_is_client_cancel() {
    let decoder = FakeDecoder {
        samples: noise(1_000),
    };
    let tracks: Vec<FlightTrack> = Vec::new();
    let terrain = FlatTerrain;
    let geoid = ZeroGeoid;
    let budget = ErrorBudget::default();
    let prior = UniformPrior;
    let spec = spec(&decoder, &tracks, &terrain, &geoid, &budget, &prior);
    let cancel = Cancel::new();
    cancel.cancel();
    let progress = RecordingProgress::default();

    let result = analyze(&ctx(cancel, None), &spec, &progress);
    assert!(matches!(
        result,
        Err(PipelineError::Cancelled(core_types::CancelReason::Client))
    ));
}

#[test]
fn pip_01_past_deadline_is_deadline_cancel() {
    let decoder = FakeDecoder {
        samples: noise(1_000),
    };
    let tracks: Vec<FlightTrack> = Vec::new();
    let terrain = FlatTerrain;
    let geoid = ZeroGeoid;
    let budget = ErrorBudget::default();
    let prior = UniformPrior;
    let spec = spec(&decoder, &tracks, &terrain, &geoid, &budget, &prior);
    let deadline = UtcMillis(1); // epoch + 1 ms: always past
    let progress = RecordingProgress::default();

    let result = analyze(&ctx(Cancel::new(), Some(deadline)), &spec, &progress);
    assert!(matches!(
        result,
        Err(PipelineError::Cancelled(core_types::CancelReason::Deadline))
    ));
}

#[test]
fn pip_01_stage_timings_match_progress_stages() {
    let decoder = FakeDecoder {
        samples: noise((SECONDS * RATE_HZ) as usize),
    };
    let tracks: Vec<FlightTrack> = Vec::new();
    let terrain = FlatTerrain;
    let geoid = ZeroGeoid;
    let budget = ErrorBudget::default();
    let prior = UniformPrior;
    let spec = spec(&decoder, &tracks, &terrain, &geoid, &budget, &prior);
    let progress = RecordingProgress::default();

    let outcome = analyze(&ctx(Cancel::new(), None), &spec, &progress).expect("analyze");
    let timed: Vec<Stage> = outcome
        .stage_timings
        .iter()
        .filter_map(|timing| {
            Stage::ALL
                .iter()
                .copied()
                .find(|stage| stage.as_str() == timing.stage)
        })
        .collect();
    let reported: Vec<Stage> = progress
        .seen
        .lock()
        .expect("progress lock")
        .iter()
        .map(|(stage, _)| *stage)
        .collect();
    for stage in timed {
        assert!(
            reported.contains(&stage),
            "timed stage {stage:?} not reported"
        );
    }
}
