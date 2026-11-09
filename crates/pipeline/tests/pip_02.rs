//! PIP-02: the run manifest is assembled from run inputs plus the finished
//! outcome — stage timings and the truncation flag can never disagree with
//! what the run did, and `parameter_hash` covers the exact parameter set.

#![allow(clippy::expect_used)]

use core_types::ids::JobId;
use pipeline::manifest::{ManifestInputs, build_manifest, parameter_hash};
use pipeline::{AtmosphereSource, DataSource, ModelVersions};
use serde::Serialize;

fn inputs(job_id: JobId) -> ManifestInputs {
    ManifestInputs {
        job_id,
        git_commit: Some("8f3ca43".to_owned()),
        crate_version: "0.0.0".to_owned(),
        parameter_hash: parameter_hash(&Params { gate: 4.0, seed: 7 }),
        rng_seed: 7,
        data_sources: vec![DataSource {
            source: "adsb.lol".to_owned(),
            version: "2026-11-04".to_owned(),
            file_sha256: "ab".repeat(32),
            sampling_median_s: 1.0,
            sampling_p90_s: 2.0,
            latency_median_s: Some(0.4),
            latency_p90_s: Some(1.1),
        }],
        atmosphere: AtmosphereSource::StandardFallback,
        models: ModelVersions {
            model: "m1".to_owned(),
            calibration: "c1".to_owned(),
            kappa: 1.0,
        },
        thread_count: 4,
    }
}

#[derive(Serialize)]
struct Params {
    gate: f64,
    seed: u64,
}

#[test]
fn pip_02_parameter_hash_is_stable_and_sensitive() {
    let a = parameter_hash(&Params { gate: 4.0, seed: 7 });
    let b = parameter_hash(&Params { gate: 4.0, seed: 7 });
    let c = parameter_hash(&Params { gate: 5.0, seed: 7 });
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.len(), 64);
}

#[test]
fn pip_02_manifest_carries_every_sdd_9_4_field() {
    let job_id = JobId::new_v7();
    let inputs = inputs(job_id);
    // A minimal outcome: no detections, timings and truncation as reported.
    let outcome = pipeline::AnalyzeOutcome {
        detections: Vec::new(),
        events: Vec::new(),
        localize: None,
        stage_timings: vec![pipeline::StageTiming {
            stage: "ingest".to_owned(),
            elapsed_s: 0.01,
        }],
        truncated: true,
    };
    let manifest = build_manifest(inputs.clone(), &outcome);

    assert_eq!(manifest.manifest_version, pipeline::MANIFEST_VERSION);
    assert_eq!(manifest.job_id, job_id);
    assert_eq!(manifest.git_commit.as_deref(), Some("8f3ca43"));
    assert_eq!(manifest.crate_version, "0.0.0");
    assert_eq!(manifest.rng_seed, 7);
    assert_eq!(manifest.data_sources.len(), 1);
    assert_eq!(manifest.atmosphere, AtmosphereSource::StandardFallback);
    assert_eq!(manifest.models.kappa, 1.0);
    assert_eq!(manifest.thread_count, 4);
    assert_eq!(manifest.stage_timings.len(), 1);
    assert!(manifest.truncated, "outcome truncation must propagate");

    // The manifest serializes and hashes deterministically.
    let first = manifest.to_json_and_hash().expect("serialize");
    let second = manifest.to_json_and_hash().expect("serialize");
    assert_eq!(first, second);
    assert_eq!(first.sha256.len(), 64);
}

#[test]
fn pip_02_manifest_hash_changes_with_inputs() {
    let outcome = pipeline::AnalyzeOutcome {
        detections: Vec::new(),
        events: Vec::new(),
        localize: None,
        stage_timings: Vec::new(),
        truncated: false,
    };
    let a = build_manifest(inputs(JobId::new_v7()), &outcome)
        .to_json_and_hash()
        .expect("serialize")
        .sha256;
    let mut changed = inputs(JobId::new_v7());
    changed.rng_seed += 1;
    let b = build_manifest(changed, &outcome)
        .to_json_and_hash()
        .expect("serialize")
        .sha256;
    assert_ne!(a, b);
}
