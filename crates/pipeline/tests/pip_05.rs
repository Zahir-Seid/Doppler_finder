//! PIP-05: report figures render off the compute path as SVG files — the
//! spectrogram with fitted curves, the evidence panel, and the calibrated
//! uncertainty ellipse when a refined solution exists.

#![allow(clippy::expect_used)]

use std::path::PathBuf;

use core_types::EventId;
use events::{EventFit, QualityAssessment};
use pipeline::run::LocalizeOutcome;
use pipeline::{FittedEvent, render_figures};
use search::{Verdict, VerdictDecision};

fn tiny_zmap() -> dsp::ZMap {
    let config = dsp::StftConfig::survey(4_000.0).expect("stft");
    let frames = 8;
    dsp::ZMap::from_parts(
        config,
        (0..frames).map(|i| i as f64 * 0.25).collect(),
        vec![0.0; frames * config.bin_count()],
    )
    .expect("zmap")
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pip05-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn event() -> FittedEvent {
    FittedEvent {
        fit: EventFit {
            event_id: EventId::new_v7(),
            fundamental_hz: core_types::Hertz(85.0),
            speed_m_per_s: 40.0,
            closest_m: core_types::Metres(2_000.0),
            heard_at: core_types::AudioSeconds(1.0),
            covariance: [[0.0; 4]; 4],
        },
        assessment: QualityAssessment {
            quality: events::EventQuality::from_score(0.9).expect("score"),
            retained: true,
            reasons: Vec::new(),
            curve_snr_db: 14.0,
            residual_autocorrelation: 0.0,
            covariance_inflation: 1.0,
            harmonic_count: 2,
        },
    }
}

fn outcome(refined: bool) -> pipeline::AnalyzeOutcome {
    pipeline::AnalyzeOutcome {
        detections: Vec::new(),
        events: vec![event()],
        localize: Some(LocalizeOutcome {
            scored: Vec::new(),
            join_candidates: Vec::new(),
            clusters: Vec::new(),
            ranked: Vec::new(),
            refined: refined.then(|| search::RefinedSolution {
                observer: core_types::LatLon::new(52.0, 13.0),
                s_unix_s: 1_700_000_000.0,
                covariance: [[1.0, 0.0, 0.0], [0.0, 4.0, 0.0], [0.0, 0.0, 1.0]],
                log_posterior: -1.0,
                iterations: 3,
                converged: true,
            }),
            ellipse_95: refined.then_some([[100.0, 0.0], [0.0, 50.0]]),
            decision: VerdictDecision {
                verdict: Verdict::Locatable,
                reason_codes: Vec::new(),
            },
            matched_track_indices: Vec::new(),
            truncated: false,
        }),
        stage_timings: Vec::new(),
        zmap: tiny_zmap(),
        truncated: false,
    }
}

#[test]
fn pip_05_renders_spectrogram_and_evidence_without_refinement() {
    let dir = scratch_dir("basic");
    let names = render_figures(&outcome(false), 343.0, &dir).expect("render");
    assert_eq!(names, vec!["spectrogram.svg", "evidence.svg"]);
    for name in &names {
        let path = dir.join(name);
        let svg = std::fs::read_to_string(&path).expect("read svg");
        assert!(
            svg.starts_with("<?xml") || svg.contains("<svg"),
            "{name} not svg"
        );
        assert!(svg.len() > 500, "{name} suspiciously small");
    }
    let evidence = std::fs::read_to_string(dir.join("evidence.svg")).expect("evidence");
    assert!(evidence.contains("Locatable"));
    assert!(evidence.contains("quality 0.90"));
}

#[test]
fn pip_05_renders_uncertainty_ellipse_when_refined() {
    let dir = scratch_dir("ellipse");
    let names = render_figures(&outcome(true), 343.0, &dir).expect("render");
    assert_eq!(
        names,
        vec!["spectrogram.svg", "uncertainty.svg", "evidence.svg"]
    );
    let svg = std::fs::read_to_string(dir.join("uncertainty.svg")).expect("uncertainty");
    assert!(svg.contains("semi-major"));
}

#[test]
fn pip_05_rejects_empty_map() {
    let mut outcome = outcome(false);
    outcome.zmap = dsp::ZMap::from_parts(
        dsp::StftConfig::survey(4_000.0).expect("stft"),
        Vec::new(),
        Vec::new(),
    )
    .expect("empty zmap");
    let dir = scratch_dir("empty");
    let result = render_figures(&outcome, 343.0, &dir);
    assert!(result.is_err());
}
