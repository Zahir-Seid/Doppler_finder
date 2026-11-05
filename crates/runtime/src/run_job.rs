//! Run one job future under a wall-time limit.

use std::future::Future;
use std::time::Duration;

use core_types::CancelReason;

use crate::outcome::JobOutcome;

/// Run a job future with a deadline.
///
/// The future returns a stable failure code on error. On timeout the result is
/// [`JobOutcome::Cancelled`] with [`CancelReason::Deadline`]. Panic containment
/// is added with the worker loop, where the task join result is available.
pub async fn run_with_limit<F, T>(future: F, limit: Duration) -> JobOutcome
where
    F: Future<Output = Result<T, String>>,
{
    match tokio::time::timeout(limit, future).await {
        Ok(Ok(_)) => JobOutcome::Done,
        Ok(Err(code)) => JobOutcome::Failed { code },
        Err(_) => JobOutcome::Cancelled(CancelReason::Deadline),
    }
}
