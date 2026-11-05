//! Progress reporting interface.

use crate::stages::Stage;

/// Receives stage progress from the pipeline.
///
/// The API implements this by writing `job_events` rows and publishing to an
/// SSE stream; the CLI prints to stderr. Implementations must be cheap and
/// must not block the compute thread.
pub trait ProgressSink: Send + Sync {
    /// Report completion of a stage, where `fraction` is in `[0, 1]`.
    fn progress(&self, stage: Stage, fraction: f64);
}
