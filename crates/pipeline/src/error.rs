//! Pipeline failures.

use core_types::CancelReason;

use crate::stages::Stage;

/// A failure before or during a pipeline stage.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PipelineError {
    /// The job was cancelled cooperatively.
    #[error("job cancelled: {0:?}")]
    Cancelled(CancelReason),
    /// A stage returned a typed failure.
    #[error("stage {stage:?} failed: {message}")]
    StageFailed {
        /// Stage that failed.
        stage: Stage,
        /// Human-readable detail for the job event.
        message: String,
    },
    /// The configuration or manifest was inconsistent.
    #[error("invalid pipeline configuration: {0}")]
    InvalidConfig(String),
}
