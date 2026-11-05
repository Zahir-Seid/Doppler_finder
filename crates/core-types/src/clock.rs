//! Injectable clock used by repositories and tests.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::time::UtcMillis;

/// A source of UTC time.
///
/// Tests use a fake clock so results are deterministic; production uses
/// [`SystemClock`].
pub trait Clock: std::fmt::Debug + Send + Sync {
    /// Return the current UTC time.
    fn now(&self) -> UtcMillis;
}

/// The operating system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UtcMillis {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => UtcMillis(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)),
            Err(_) => UtcMillis(0),
        }
    }
}
