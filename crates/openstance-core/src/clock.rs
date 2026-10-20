//! Injected time. Core logic never reads the system clock directly; it asks
//! a [`Clock`] so tests and `--now` are reproducible.

/// A source of "now" in unix seconds (UTC).
pub trait Clock: Send + Sync {
    /// Current time, unix seconds.
    fn now(&self) -> i64;
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

/// A clock frozen at one instant (tests, `--now`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now(&self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_is_fixed_and_system_clock_is_recent() {
        assert_eq!(FixedClock(42).now(), 42);
        assert!(SystemClock.now() > 1_700_000_000);
    }
}
