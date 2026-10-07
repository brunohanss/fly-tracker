use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockoutReason {
    Human,
    Dog,
    Cat,
    DetectorUnavailable,
    DetectorError,
    DetectorTimeout,
    StaleSafety,
    StaleCamera,
    InsufficientConfidence,
    InvalidState,
    Shutdown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafetyState {
    Clear,
    SafetyLockout(LockoutReason),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemState {
    Idle,
    Tracking,
    TemporarilyLost,
    SafetyLockout(LockoutReason),
    Stopped,
}
/// View reducer only. The safety owner must gate commands separately.
pub fn system_state(safety: SafetyState, has_confirmed: bool, has_lost: bool) -> SystemState {
    match safety {
        SafetyState::SafetyLockout(reason) => SystemState::SafetyLockout(reason),
        SafetyState::Clear if has_confirmed => SystemState::Tracking,
        SafetyState::Clear if has_lost => SystemState::TemporarilyLost,
        SafetyState::Clear => SystemState::Idle,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lockout_overrides_every_tracking_state() {
        for reason in [
            LockoutReason::Human,
            LockoutReason::Dog,
            LockoutReason::Cat,
            LockoutReason::DetectorUnavailable,
            LockoutReason::DetectorError,
            LockoutReason::DetectorTimeout,
            LockoutReason::StaleSafety,
            LockoutReason::StaleCamera,
            LockoutReason::InsufficientConfidence,
            LockoutReason::InvalidState,
            LockoutReason::Shutdown,
        ] {
            for confirmed in [false, true] {
                for lost in [false, true] {
                    assert_eq!(
                        system_state(SafetyState::SafetyLockout(reason), confirmed, lost),
                        SystemState::SafetyLockout(reason)
                    );
                }
            }
        }
    }
}
