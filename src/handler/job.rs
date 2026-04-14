use std::task::Context;
use std::time::Duration;

use tokio::time::{Interval, MissedTickBehavior};

use crate::handler::REQUEST_TIMEOUT;

/// A background job run periodically.
///
/// Uses `tokio::time::Interval` instead of a boxed `Sleep` to avoid
/// heap indirection and manual deadline resets.
#[derive(Debug)]
pub(crate) struct PeriodicJob {
    /// The interval timer that fires periodically.
    interval: Interval,
}

impl PeriodicJob {
    /// Returns `true` if the job is currently not running but ready
    /// to be run, `false` otherwise.
    pub fn poll_ready(&mut self, cx: &mut Context<'_>) -> bool {
        self.interval.poll_tick(cx).is_ready()
    }

    pub fn new(period: Duration) -> Self {
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        // If ticks are missed (e.g. handler was busy), skip them instead of
        // firing a burst of catch-up ticks.
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self { interval }
    }
}

impl Default for PeriodicJob {
    fn default() -> Self {
        Self::new(Duration::from_millis(REQUEST_TIMEOUT))
    }
}
