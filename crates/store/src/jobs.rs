//! Job records and the repository boundary (SDD sections 9.3 and 10.1).

use core_types::{JobId, UtcMillis};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Lifecycle state of an analysis job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum JobStatus {
    /// Waiting for a worker.
    Queued,
    /// Claimed by a worker.
    Running,
    /// Finished successfully.
    Done,
    /// Finished with a stable error code.
    Failed,
    /// Deleted or expired before completion.
    Expired,
}

impl JobStatus {
    /// Stable status name used by the database and API.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }
}

/// One row of the `jobs` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    /// Job identifier.
    pub id: JobId,
    /// Current lifecycle state.
    pub status: JobStatus,
    /// Creation time in UTC milliseconds.
    pub created_at: UtcMillis,
    /// Current stage name while running.
    pub stage: Option<String>,
    /// Stable error code when failed.
    pub error_code: Option<String>,
}

/// Synchronous repository boundary for jobs.
///
/// The sqlx implementation lives behind this trait so the pipeline and tests
/// do not depend on a database driver.
pub trait JobRepository: Send + Sync {
    /// Insert a new job.
    fn insert(&self, job: &JobRecord) -> Result<(), StoreError>;

    /// Fetch a job by identifier.
    fn get(&self, id: JobId) -> Result<Option<JobRecord>, StoreError>;

    /// Move a job to a new lifecycle state.
    ///
    /// Implementations must reject transitions that violate SDD section 10.1
    /// with [`StoreError::Conflict`].
    fn update_status(&self, id: JobId, status: JobStatus) -> Result<(), StoreError>;
}
