use std::time::{Duration, Instant};

/// Active playback time at the last processed frame. UI idle time is not work.
pub(crate) struct ReplayClock {
    started: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
    progress: Duration,
}
impl ReplayClock {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            started: now,
            paused_at: None,
            paused_total: Duration::ZERO,
            progress: Duration::ZERO,
        }
    }
    pub(crate) fn pause(&mut self, paused: bool, now: Instant) {
        if paused {
            self.paused_at.get_or_insert(now);
        } else if let Some(start) = self.paused_at.take() {
            self.paused_total += now.saturating_duration_since(start);
        }
    }
    pub(crate) fn record_progress(&mut self, now: Instant) {
        let endpoint = self.paused_at.unwrap_or(now);
        self.progress = endpoint
            .saturating_duration_since(self.started)
            .saturating_sub(self.paused_total);
    }
    pub(crate) fn elapsed(&self) -> Duration {
        self.progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_pause_and_idle_snapshots_do_not_reduce_rate() {
        let start = Instant::now();
        let mut clock = ReplayClock::new(start);
        clock.record_progress(start + Duration::from_secs(3));
        let original = clock.elapsed();
        clock.pause(true, start + Duration::from_secs(3));
        // A repeated pause command must not reset the start of the pause.
        clock.pause(true, start + Duration::from_secs(100));
        clock.record_progress(start + Duration::from_secs(1221));
        assert_eq!(clock.elapsed(), original);
        clock.pause(false, start + Duration::from_secs(1221));
        clock.record_progress(start + Duration::from_secs(1222));
        assert_eq!(clock.elapsed(), Duration::from_secs(4));
        // Reads after EOF and repeated UI refreshes cannot change the denominator.
        assert_eq!(clock.elapsed(), Duration::from_secs(4));
    }
}
