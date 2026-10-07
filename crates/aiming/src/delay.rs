//! Bounded replay-time aiming simulation. Stores one target ID, never coordinates or permission.
use fly_core::{FrameTimestamp, TargetId};
use std::time::Duration;

pub struct VirtualAimDelay {
    delay: Duration,
    pending: Option<(TargetId, FrameTimestamp)>,
}
impl VirtualAimDelay {
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            pending: None,
        }
    }
    pub fn target(&self) -> Option<TargetId> {
        self.pending.map(|(target, _)| target)
    }
    pub fn cancel(&mut self) {
        self.pending = None;
    }
    /// Call only while current-frame evidence and the target are valid.
    /// Zero remaining time means due, not authorized. The interlock must still run.
    pub fn remaining_us(
        &mut self,
        target: TargetId,
        now: FrameTimestamp,
    ) -> Result<u64, fly_core::DomainError> {
        if self.pending.is_none_or(|(pending, _)| pending != target) {
            self.pending = Some((target, now.advance(self.delay)?));
        }
        Ok(self
            .pending
            .map_or(0, |(_, due)| due.0.saturating_sub(now.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadline_boundary_replacement_and_cancel_are_explicit() -> Result<(), fly_core::DomainError>
    {
        let mut timer = VirtualAimDelay::new(Duration::from_millis(500));
        let first = TargetId(1);
        assert_eq!(timer.remaining_us(first, FrameTimestamp(1000))?, 500_000);
        assert_eq!(timer.remaining_us(first, FrameTimestamp(500_999))?, 1);
        assert_eq!(timer.remaining_us(first, FrameTimestamp(501_000))?, 0);
        assert_eq!(
            timer.remaining_us(TargetId(2), FrameTimestamp(501_000))?,
            500_000
        );
        timer.cancel();
        assert_eq!(timer.target(), None);
        assert_eq!(timer.remaining_us(first, FrameTimestamp(600_000))?, 500_000);
        Ok(())
    }
    #[test]
    fn zero_delay_and_timestamp_overflow() -> Result<(), fly_core::DomainError> {
        let mut zero = VirtualAimDelay::new(Duration::ZERO);
        assert_eq!(zero.remaining_us(TargetId(1), FrameTimestamp(0))?, 0);
        let mut timer = VirtualAimDelay::new(Duration::from_millis(500));
        assert!(
            timer
                .remaining_us(TargetId(1), FrameTimestamp(u64::MAX))
                .is_err()
        );
        assert_eq!(timer.target(), None);
        Ok(())
    }
}
