//! Relational repositories, service limits and retention policy.
//!
//! Requirements FR-AC-04 and FR-OP-02. The sqlx implementation and migrations
//! arrive with the persistence task; this crate fixes the records, repository
//! boundary and the initial limits from SDD sections 9 and 11.3.

pub mod error;
pub mod jobs;
pub mod limits;
pub mod retention;

pub use error::StoreError;
pub use jobs::{JobRecord, JobRepository, JobStatus};
pub use limits::ServiceLimits;
pub use retention::RetentionPolicy;
