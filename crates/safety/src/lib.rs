#![forbid(unsafe_code)]
pub mod nanodet;
use camera::{Frame, Hazard};
use fly_core::{Confidence, FrameId, FrameTimestamp, LockoutReason, SafetyState, config::Config};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceScope {
    ReplayFixture,
    ReplayImage,
    Live,
}
#[derive(Debug, Clone, Copy)]
pub enum Verdict {
    Clear(Confidence),
    Hazard(Hazard),
    Uncertain,
    Unavailable,
    Error,
    Timeout,
    Stale,
    Invalid,
}
#[derive(Debug, Clone, Copy)]
pub struct Evidence {
    pub frame: FrameId,
    pub timestamp: FrameTimestamp,
    pub scope: EvidenceScope,
    pub verdict: Verdict,
}
pub trait SafetyDetector: Send {
    fn evaluate(&mut self, frame: &Frame) -> Evidence;
    fn diagnostics(&self) -> String {
        "Controlled backend; health and class scores unavailable".into()
    }
    fn replay_scope(&self) -> EvidenceScope {
        EvidenceScope::ReplayFixture
    }
}

/// Test-only semantics. No image recognition and no live output authority.
pub struct FixtureDetector;
impl SafetyDetector for FixtureDetector {
    fn diagnostics(&self) -> String {
        "Fixture labels (explicit test mode); no image recognition".into()
    }
    fn evaluate(&mut self, frame: &Frame) -> Evidence {
        let verdict = match &frame.truth {
            Some(truth) => match truth.hazard {
                Some(hazard) => Verdict::Hazard(hazard),
                None => Verdict::Clear(
                    Confidence::new(1.0)
                        .unwrap_or_else(|_| unreachable!("constant valid confidence")),
                ),
            },
            None => Verdict::Unavailable,
        };
        Evidence {
            frame: frame.id,
            timestamp: frame.timestamp,
            scope: EvidenceScope::ReplayFixture,
            verdict,
        }
    }
}
/// Default production behavior until a validated detector is installed.
pub struct UnavailableDetector;
impl SafetyDetector for UnavailableDetector {
    fn diagnostics(&self) -> String {
        "Unavailable; no safety detector initialized".into()
    }
    fn evaluate(&mut self, frame: &Frame) -> Evidence {
        Evidence {
            frame: frame.id,
            timestamp: frame.timestamp,
            scope: EvidenceScope::Live,
            verdict: Verdict::Unavailable,
        }
    }
}
#[derive(Debug, Error, PartialEq, Eq)]
#[error("Output denied: {0:?}")]
pub struct OutputDenied(pub LockoutReason);

pub struct SafetyAuthority {
    frame_age_us: u64,
    safety_age_us: u64,
    minimum_confidence: Confidence,
    camera: Option<(FrameId, FrameTimestamp)>,
    evidence: Option<Evidence>,
    last_check: Option<FrameTimestamp>,
    latched: Option<LockoutReason>,
    processing_age_us: u64,
}
impl SafetyAuthority {
    pub fn new(config: &Config) -> Result<Self, fly_core::config::ConfigError> {
        config.validate()?;
        Ok(Self {
            frame_age_us: config.max_frame_age_us,
            safety_age_us: config.max_safety_age_us,
            minimum_confidence: config.minimum_clear_confidence,
            camera: None,
            evidence: None,
            last_check: None,
            latched: None,
            processing_age_us: 0,
        })
    }
    pub fn record_camera(&mut self, id: FrameId, captured: FrameTimestamp) {
        if self.camera.is_some_and(|(previous_id, previous_time)| {
            id <= previous_id || captured <= previous_time
        }) {
            self.latched = Some(LockoutReason::InvalidState);
        }
        self.camera = Some((id, captured));
        self.evidence = None;
        self.processing_age_us = 0;
    }
    /// Additional real elapsed time for synchronous image replay processing.
    pub fn record_processing_age(&mut self, elapsed: std::time::Duration) {
        self.processing_age_us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
    }
    pub fn observe(&mut self, evidence: Evidence) {
        self.evidence = Some(evidence);
    }
    pub fn shutdown(&mut self) {
        self.latched = Some(LockoutReason::Shutdown);
    }
    /// Call on an independent live watchdog tick as well as before every command.
    /// Does not require a new frame to revoke permission.
    pub fn state_at(&mut self, now: FrameTimestamp) -> SafetyState {
        if self.last_check.is_some_and(|last| now < last) {
            self.latched = Some(LockoutReason::InvalidState);
        }
        self.last_check = Some(now);
        let reason = self.lockout_reason(now);
        reason.map_or(SafetyState::Clear, SafetyState::SafetyLockout)
    }
    fn lockout_reason(&self, now: FrameTimestamp) -> Option<LockoutReason> {
        if let Some(reason) = self.latched {
            return Some(reason);
        }
        let Some((id, captured)) = self.camera else {
            return Some(LockoutReason::StaleCamera);
        };
        let Some(camera_age) = now.0.checked_sub(captured.0) else {
            return Some(LockoutReason::InvalidState);
        };
        if camera_age.max(self.processing_age_us) > self.frame_age_us {
            return Some(LockoutReason::StaleCamera);
        }
        let Some(evidence) = self.evidence else {
            return Some(LockoutReason::DetectorUnavailable);
        };
        if evidence.frame != id || evidence.timestamp != captured || evidence.timestamp > now {
            return Some(LockoutReason::InvalidState);
        }
        if (now.0 - evidence.timestamp.0).max(self.processing_age_us) > self.safety_age_us {
            return Some(LockoutReason::StaleSafety);
        }
        match evidence.verdict {
            Verdict::Clear(confidence) if confidence.get() >= self.minimum_confidence.get() => None,
            Verdict::Clear(_) | Verdict::Uncertain => Some(LockoutReason::InsufficientConfidence),
            Verdict::Hazard(Hazard::Human) => Some(LockoutReason::Human),
            Verdict::Hazard(Hazard::Dog) => Some(LockoutReason::Dog),
            Verdict::Hazard(Hazard::Cat) => Some(LockoutReason::Cat),
            Verdict::Unavailable => Some(LockoutReason::DetectorUnavailable),
            Verdict::Error => Some(LockoutReason::DetectorError),
            Verdict::Timeout => Some(LockoutReason::DetectorTimeout),
            Verdict::Stale => Some(LockoutReason::StaleSafety),
            Verdict::Invalid => Some(LockoutReason::InvalidState),
        }
    }
    /// Runs the command only after fresh evidence is checked. No cacheable permit.
    pub fn execute<R>(
        &mut self,
        now: FrameTimestamp,
        frame: FrameId,
        scope: EvidenceScope,
        command: impl FnOnce() -> R,
    ) -> Result<R, OutputDenied> {
        if let SafetyState::SafetyLockout(reason) = self.state_at(now) {
            return Err(OutputDenied(reason));
        }
        if self.camera.is_none_or(|(id, _)| id != frame)
            || self.evidence.is_none_or(|evidence| evidence.scope != scope)
        {
            return Err(OutputDenied(LockoutReason::InvalidState));
        }
        Ok(command())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use camera::{
        FrameSource,
        synthetic::{Scenario, SyntheticSource},
    };
    #[test]
    fn all_safety_fixtures_deny_output() -> Result<(), Box<dyn std::error::Error>> {
        let config = Config::default();
        for scenario in Scenario::ALL
            .into_iter()
            .filter(|scenario| scenario.hazard().is_some())
        {
            let mut source = SyntheticSource::new(config.frame_size, scenario, 42, 10, 10_000)?;
            let mut frame = Frame::new(config.frame_size);
            let mut authority = SafetyAuthority::new(&config)?;
            let mut detector = FixtureDetector;
            let mut commands = 0;
            while source.next_into(&mut frame)? {
                authority.record_camera(frame.id, frame.timestamp);
                authority.observe(detector.evaluate(&frame));
                assert!(
                    matches!(
                        authority.state_at(frame.timestamp),
                        SafetyState::SafetyLockout(_)
                    ),
                    "scenario={scenario:?}"
                );
                assert!(
                    authority
                        .execute(
                            frame.timestamp,
                            frame.id,
                            EvidenceScope::ReplayFixture,
                            || commands += 1
                        )
                        .is_err()
                );
            }
            assert_eq!(commands, 0);
        }
        Ok(())
    }
    #[test]
    fn expiry_faults_and_scope_are_checked_at_execution() -> Result<(), Box<dyn std::error::Error>>
    {
        let config = Config {
            max_safety_age_us: 10,
            max_frame_age_us: 20,
            ..Config::default()
        };
        let mut gate = SafetyAuthority::new(&config)?;
        gate.record_camera(FrameId(1), FrameTimestamp(100));
        let clear = Evidence {
            frame: FrameId(1),
            timestamp: FrameTimestamp(100),
            scope: EvidenceScope::ReplayFixture,
            verdict: Verdict::Clear(Confidence::new(1.0)?),
        };
        gate.observe(clear);
        assert!(
            gate.execute(FrameTimestamp(100), FrameId(1), EvidenceScope::Live, || ())
                .is_err()
        );
        assert!(
            gate.execute(
                FrameTimestamp(110),
                FrameId(1),
                EvidenceScope::ReplayFixture,
                || ()
            )
            .is_ok()
        );
        assert_eq!(
            gate.state_at(FrameTimestamp(111)),
            SafetyState::SafetyLockout(LockoutReason::StaleSafety)
        );
        assert_eq!(
            gate.state_at(FrameTimestamp(121)),
            SafetyState::SafetyLockout(LockoutReason::StaleCamera)
        );
        for (verdict, reason) in [
            (Verdict::Error, LockoutReason::DetectorError),
            (Verdict::Unavailable, LockoutReason::DetectorUnavailable),
            (Verdict::Timeout, LockoutReason::DetectorTimeout),
            (Verdict::Uncertain, LockoutReason::InsufficientConfidence),
            (
                Verdict::Clear(Confidence::new(0.1)?),
                LockoutReason::InsufficientConfidence,
            ),
        ] {
            let mut gate = SafetyAuthority::new(&config)?;
            gate.record_camera(FrameId(1), FrameTimestamp(100));
            gate.observe(Evidence { verdict, ..clear });
            assert_eq!(
                gate.state_at(FrameTimestamp(100)),
                SafetyState::SafetyLockout(reason)
            );
        }
        Ok(())
    }
    #[test]
    fn invalid_identity_time_and_shutdown_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
        let mut gate = SafetyAuthority::new(&Config::default())?;
        gate.record_camera(FrameId(2), FrameTimestamp(100));
        gate.observe(Evidence {
            frame: FrameId(1),
            timestamp: FrameTimestamp(100),
            scope: EvidenceScope::Live,
            verdict: Verdict::Clear(Confidence::new(1.0)?),
        });
        assert_eq!(
            gate.state_at(FrameTimestamp(100)),
            SafetyState::SafetyLockout(LockoutReason::InvalidState)
        );
        gate.record_camera(FrameId(1), FrameTimestamp(99));
        assert_eq!(
            gate.state_at(FrameTimestamp(101)),
            SafetyState::SafetyLockout(LockoutReason::InvalidState)
        );
        gate.shutdown();
        assert_eq!(
            gate.state_at(FrameTimestamp(102)),
            SafetyState::SafetyLockout(LockoutReason::Shutdown)
        );
        Ok(())
    }
}
