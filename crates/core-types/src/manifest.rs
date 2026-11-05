//! Run-manifest types (SDD section 9.4, NFR-04).
//!
//! A result is reproducible if and only if it can be regenerated from its
//! manifest (NFR-04). The types live here so that every crate can describe a
//! run without depending on `pipeline`; the pipeline owns writing and hashing.

use serde::{Deserialize, Serialize};

use crate::ids::JobId;

/// Current manifest schema version.
pub const MANIFEST_VERSION: u32 = 1;

/// One data source used by a run, with its identity and measured statistics.
///
/// The measured sampling statistics are what the search needs in order to
/// reproduce its uncertainty budget (SDD section 9.3, `ingest_runs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataSource {
    /// Source name, for example `adsb.lol` or `opensky`.
    pub source: String,
    /// Source or archive version, as the provider reports it.
    pub version: String,
    /// SHA-256 of the consumed file, hex encoded.
    pub file_sha256: String,
    /// Median interval between samples, in seconds.
    pub sampling_median_s: f64,
    /// 90th-percentile interval between samples, in seconds.
    pub sampling_p90_s: f64,
}

/// How the atmosphere was obtained (SDD T-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtmosphereSource {
    /// A named reanalysis or observation source.
    Measured,
    /// The standard atmosphere with its wide model error.
    StandardFallback,
}

/// Model and calibration versions that affect the result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelVersions {
    /// Doppler and atmospheric model version.
    pub model: String,
    /// Calibration version, including the factor `kappa`.
    pub calibration: String,
    /// The calibration factor `kappa` itself (SDD section 6.4).
    pub kappa: f64,
}

/// Wall-clock duration of one pipeline stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageTiming {
    /// Stage name, matching the stage enum.
    pub stage: String,
    /// Elapsed wall time in seconds.
    pub elapsed_s: f64,
}

/// Everything needed to regenerate one result (SDD section 9.4).
///
/// Fields grow as data sources and calibration are added. A result is
/// reproducible only when re-running from this manifest reproduces its hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunManifest {
    /// Manifest schema version.
    pub manifest_version: u32,
    /// Job this manifest belongs to.
    pub job_id: JobId,
    /// Git commit of the running binary, when known.
    pub git_commit: Option<String>,
    /// Workspace version from `Cargo.toml`.
    pub crate_version: String,
    /// Hash of the parameter set.
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
    /// Per-stage wall times.
    pub stage_timings: Vec<StageTiming>,
    /// Whether any input or search budget was truncated.
    pub truncated: bool,
}

impl RunManifest {
    /// Serialize to canonical JSON and return it with its SHA-256 hash.
    ///
    /// The manifest is stored alongside its hash so a result can be tied to
    /// the exact inputs that produced it (SDD section 9.4, NFR-04).
    ///
    /// # Errors
    ///
    /// Returns a [`serde_json::Error`] if serialization fails, which for this
    /// type cannot happen with finite `f64` fields.
    pub fn to_json_and_hash(&self) -> Result<ManifestJson, serde_json::Error> {
        let json = serde_json::to_string(self)?;
        let sha256 = sha256_hex(json.as_bytes());
        Ok(ManifestJson { json, sha256 })
    }
}

/// A serialized manifest together with the hash of its exact bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestJson {
    /// The manifest as JSON text.
    pub json: String,
    /// SHA-256 of `json`'s UTF-8 bytes, lowercase hex.
    pub sha256: String,
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Writing to a String cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RunManifest {
        RunManifest {
            manifest_version: MANIFEST_VERSION,
            job_id: JobId::new_v7(),
            git_commit: Some("deadbeef".to_owned()),
            crate_version: "0.0.0".to_owned(),
            parameter_hash: "abc123".to_owned(),
            rng_seed: 42,
            data_sources: vec![DataSource {
                source: "adsb.lol".to_owned(),
                version: "2026-11-04".to_owned(),
                file_sha256: "00".repeat(32),
                sampling_median_s: 1.0,
                sampling_p90_s: 2.5,
            }],
            atmosphere: AtmosphereSource::StandardFallback,
            models: ModelVersions {
                model: "v1".to_owned(),
                calibration: "c1".to_owned(),
                kappa: 1.1,
            },
            thread_count: 8,
            stage_timings: vec![StageTiming {
                stage: "detect".to_owned(),
                elapsed_s: 1.25,
            }],
            truncated: false,
        }
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let manifest = sample();
        let json = serde_json::to_string(&manifest).expect("serialize");
        let back: RunManifest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, manifest);
    }

    #[test]
    fn manifest_serializes_the_schema_version() {
        let json = serde_json::to_value(sample()).expect("serialize");
        assert_eq!(
            json.get("manifest_version")
                .and_then(serde_json::Value::as_u64),
            Some(u64::from(MANIFEST_VERSION))
        );
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn manifest_hash_is_stable_and_matches_its_json() {
        let manifest = sample();
        let first = manifest.to_json_and_hash().expect("serialize");
        let second = manifest.to_json_and_hash().expect("serialize");
        assert_eq!(first, second);
        assert_eq!(first.sha256, sha256_hex(first.json.as_bytes()));
        assert_eq!(first.sha256.len(), 64);
    }

    #[test]
    fn different_manifests_hash_differently() {
        let a = sample();
        let mut b = sample();
        b.rng_seed = a.rng_seed + 1;
        assert_ne!(
            a.to_json_and_hash().expect("serialize").sha256,
            b.to_json_and_hash().expect("serialize").sha256
        );
    }

    #[test]
    fn atmosphere_uses_snake_case_tags() {
        let json = serde_json::to_value(AtmosphereSource::StandardFallback).expect("serialize");
        assert_eq!(json, serde_json::json!("standard_fallback"));
    }
}
