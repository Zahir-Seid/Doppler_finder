//! Stage orchestration, progress and run manifests.
//!
//! Requirements FR-OUT-01 to FR-OUT-04. The pipeline is synchronous; the
//! binaries hand it a [`core_types::JobCtx`] and poll cancellation at stage
//! boundaries. Stage implementations arrive with their crate-level tasks.

pub mod budget;
pub mod error;
pub mod manifest;
pub mod progress;
pub mod run;
pub mod stages;

pub use budget::{PipelineConfig, RunMode};
pub use error::PipelineError;
pub use manifest::RunManifest;
pub use progress::ProgressSink;
pub use run::{
    AnalyzeOutcome, AnalyzeSpec, FittedEvent, FrontEndSpec, LocalizeOutcome, LocalizeSpec, analyze,
};
pub use stages::Stage;
