//! Tokio task machinery shared by the `api` and `worker` binaries.
//!
//! Requirement FR-OP-01 and SDD section 11.6. This is the only library crate
//! allowed to use tokio. The rules are: every task belongs to a [`Scope`], the
//! owner awaits it, cancellation flows down, and blocking work polls
//! [`core_types::Cancel`].

pub mod bridge;
pub mod outcome;
pub mod run_job;
pub mod scope;

pub use bridge::spawn_cancel_bridge;
pub use outcome::JobOutcome;
pub use run_job::run_with_limit;
pub use scope::Scope;
