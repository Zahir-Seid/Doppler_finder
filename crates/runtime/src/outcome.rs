//! Terminal outcome of one job (SDD section 11.6).

use core_types::CancelReason;

/// How a job ended.
///
/// The worker maps this value to the `jobs.status` column in one transaction
/// after the job scope has drained.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum JobOutcome {
    /// The job completed and its result was stored.
    Done,
    /// The job failed with a stable error code from SDD section 10.3.
    Failed {
        /// Stable error code, for example `internal`.
        code: String,
    },
    /// The job was cancelled for the stated reason.
    Cancelled(CancelReason),
    /// A task panicked; the worker records the failure and stays alive.
    Panicked,
}
