//! Run manifest (SDD section 9.4, NFR-04).
//!
//! The types live in `core-types` so every crate can describe a run without
//! depending on this one. This module re-exports them and is where writing,
//! hashing and persisting will be added.

pub use core_types::manifest::{
    AtmosphereSource, DataSource, MANIFEST_VERSION, ManifestJson, ModelVersions, RunManifest,
    StageTiming, sha256_hex,
};
