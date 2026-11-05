//! Cancellation and per-job context.
//!
//! `JobCtx` is deliberately free of tokio types so synchronous library crates
//! can accept it (SDD section 11.6). The async bridge that turns a tokio
//! `CancellationToken` into `Cancel` lives in the `runtime` crate.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::ids::JobId;
use crate::time::UtcMillis;

/// Why a job was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CancelReason {
    /// The client deleted the job.
    Client,
    /// The job's wall-time limit expired.
    Deadline,
    /// The process is shutting down.
    Shutdown,
}

/// A cloneable cancellation flag polled by synchronous code.
///
/// The flag is set by the runtime's bridge task. Polling is cheap and lock-free,
/// so stages may check it at every boundary and inside long loops.
#[derive(Debug, Clone, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
}

impl Cancel {
    /// Create a new, uncancelled flag.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Idempotent.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Return whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// Read-only context handed to every pipeline stage.
///
/// The context carries the job identity, the cooperative cancellation flag,
/// an optional deadline and a private scratch directory. It owns no tokio
/// handles.
#[derive(Debug, Clone)]
pub struct JobCtx {
    /// Identity of the job.
    pub job_id: JobId,
    /// Cooperative cancellation flag.
    pub cancel: Cancel,
    /// Optional wall-clock deadline for the job.
    pub deadline: Option<UtcMillis>,
    /// Directory the job may write temporary files to.
    pub scratch_dir: PathBuf,
}
