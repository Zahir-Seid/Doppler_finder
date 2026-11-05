//! Injectable clock used by repositories and tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::time::UtcMillis;

/// A source of UTC time.
///
/// Tests use a fake clock so results are deterministic; production uses
/// [`SystemClock`]. Every component that needs "now" takes a `&dyn Clock` so
/// time can be frozen in a test.
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

/// A clock that stands still until advanced by the test.
///
/// Clones share the same instant, so passing a clone to the code under test
/// still lets the test move time.
#[derive(Debug, Clone)]
pub struct FakeClock {
    now: Arc<AtomicI64>,
}

impl FakeClock {
    /// Create a clock frozen at `start`.
    pub fn new(start: UtcMillis) -> Self {
        Self {
            now: Arc::new(AtomicI64::new(start.0)),
        }
    }

    /// Move the clock to `instant`.
    pub fn set(&self, instant: UtcMillis) {
        self.now.store(instant.0, Ordering::SeqCst);
    }

    /// Advance the clock by `millis`, saturating at the `i64` bounds.
    pub fn advance_millis(&self, millis: i64) {
        let previous = self.now.fetch_add(millis, Ordering::SeqCst);
        let _ = previous.saturating_add(millis);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> UtcMillis {
        UtcMillis(self.now.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_returns_its_start() {
        let clock = FakeClock::new(UtcMillis(1_700_000_000_000));
        assert_eq!(clock.now(), UtcMillis(1_700_000_000_000));
    }

    #[test]
    fn fake_clock_advances_and_clones_share_the_instant() {
        let clock = FakeClock::new(UtcMillis(0));
        let alias = clock.clone();
        clock.advance_millis(1_500);
        assert_eq!(alias.now(), UtcMillis(1_500));
        alias.set(UtcMillis(9_000));
        assert_eq!(clock.now(), UtcMillis(9_000));
    }

    #[test]
    fn system_clock_is_after_the_epoch() {
        let now = SystemClock.now();
        assert!(now.0 > 0, "system clock should be past 1970: {now:?}");
    }

    #[test]
    fn clock_trait_object_is_usable() {
        let clock: Box<dyn Clock> = Box::new(FakeClock::new(UtcMillis(42)));
        assert_eq!(clock.now().0, 42);
    }
}
