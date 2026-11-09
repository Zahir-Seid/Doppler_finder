//! Run manifest (SDD section 9.4, NFR-04).
//!
//! The types live in `core-types` so every crate can describe a run without
//! depending on this one. This module builds the manifest from a finished
//! [`AnalyzeOutcome`] plus the run-level inputs the outcome cannot know
//! (data sources, atmosphere provenance, versions, seed, thread count).

use core_types::ids::JobId;
use serde::Serialize;

pub use core_types::manifest::{
    AtmosphereSource, DataSource, MANIFEST_VERSION, ManifestJson, ModelVersions, RunManifest,
    StageTiming, sha256_hex,
};

use crate::run::AnalyzeOutcome;

/// Run-level inputs the manifest needs that [`AnalyzeOutcome`] does not carry.
///
/// The caller (CLI, worker) fills these from job configuration and the data
/// layer: which archives were read, how the atmosphere was obtained, which
/// binary and parameter set produced the run.
#[derive(Debug, Clone)]
pub struct ManifestInputs {
    /// Job this manifest belongs to.
    pub job_id: JobId,
    /// Git commit of the running binary, when known.
    pub git_commit: Option<String>,
    /// Workspace version from `Cargo.toml`.
    pub crate_version: String,
    /// Hash of the parameter set (see [`parameter_hash`]).
    pub parameter_hash: String,
    /// Seed for every stochastic stage.
    pub rng_seed: u64,
    /// Data sources used, with versions, hashes and measured statistics.
    pub data_sources: Vec<DataSource>,
    /// Atmosphere source, or the standard-atmosphere fallback.
    pub atmosphere: AtmosphereSource,
    /// Model and calibration versions, including `kappa`.
    pub models: ModelVersions,
    /// Number of worker threads used.
    pub thread_count: usize,
}

/// Build the run manifest for a finished analysis (SDD 9.4).
///
/// Stage timings and the truncation flag come straight from the outcome, so
/// the manifest can never disagree with what the run actually did.
#[must_use]
pub fn build_manifest(inputs: ManifestInputs, outcome: &AnalyzeOutcome) -> RunManifest {
    RunManifest {
        manifest_version: MANIFEST_VERSION,
        job_id: inputs.job_id,
        git_commit: inputs.git_commit,
        crate_version: inputs.crate_version,
        parameter_hash: inputs.parameter_hash,
        rng_seed: inputs.rng_seed,
        data_sources: inputs.data_sources,
        atmosphere: inputs.atmosphere,
        models: inputs.models,
        thread_count: inputs.thread_count,
        stage_timings: outcome.stage_timings.clone(),
        truncated: outcome.truncated,
    }
}

/// Hash a serializable parameter set into the manifest's `parameter_hash`.
///
/// The hash covers the exact JSON encoding, so two parameter sets that differ
/// in any recorded field hash differently. Serialization failure returns the
/// empty-string hash of no input rather than panicking; callers pass plain
/// data structs so this cannot happen in practice.
#[must_use]
pub fn parameter_hash<T: Serialize + ?Sized>(params: &T) -> String {
    match serde_json::to_vec(params) {
        Ok(bytes) => sha256_hex(&bytes),
        Err(_) => sha256_hex(b""),
    }
}
