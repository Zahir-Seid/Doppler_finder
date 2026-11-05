//! Run manifest (SDD section 9.4, NFR-04).

use serde::{Deserialize, Serialize};

/// Everything needed to regenerate one result.
///
/// Fields grow as data sources and calibration are added. A result is
/// reproducible only when re-running from this manifest reproduces its hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunManifest {
    /// Manifest schema version.
    pub manifest_version: u32,
    /// Git commit of the running binary.
    pub git_commit: Option<String>,
    /// Workspace version from `Cargo.toml`.
    pub crate_version: String,
    /// Hash of the parameter set.
    pub parameter_hash: String,
    /// Seed for every stochastic stage.
    pub rng_seed: u64,
    /// Calibration version, including the factor `kappa`.
    pub calibration_version: String,
    /// Whether any input or search budget was truncated.
    pub truncated: bool,
}
