use aiming::{AimStatus, calibration::Calibration};
use app::pipeline::ReplayPipeline;
use camera::{Frame, FrameSource};
use fly_core::{
    Confidence, FrameId, FrameSize, FrameTimestamp, LockoutReason, SafetyState, config::Config,
};
use safety::{Evidence, EvidenceScope, SafetyDetector, Verdict};

struct Frames {
    index: u64,
    missing: bool,
}
impl FrameSource for Frames {
    fn size(&self) -> FrameSize {
        FrameSize::new(96, 64).unwrap_or_else(|_| unreachable!())
    }
    fn next_into(&mut self, frame: &mut Frame) -> Result<bool, camera::CameraError> {
        if self.index == 22 {
            return Ok(false);
        }
        frame.pixels.fill(160);
        let x = if self.missing {
            12
        } else {
            12 + self.index as usize
        };
        if self.index > 0 && !(self.missing && (4..12).contains(&self.index)) {
            for row in 11..=13 {
                for col in x - 1..=x + 1 {
                    frame.pixels[row * 96 + col] = 20;
                }
            }
        }
        if !self.missing && self.index > 0 {
            for row in 39..=41 {
                for col in 59..=61 {
                    frame.pixels[row * 96 + col] = 20;
                }
            }
        }
        frame.id = FrameId(self.index);
        frame.timestamp = FrameTimestamp(self.index * 100_000);
        self.index += 1;
        Ok(true)
    }
    fn reset(&mut self) -> Result<(), camera::CameraError> {
        self.index = 0;
        Ok(())
    }
}
struct Detector {
    failure: Option<Verdict>,
}
impl SafetyDetector for Detector {
    fn evaluate(&mut self, frame: &Frame) -> Evidence {
        let verdict = if (3..9).contains(&frame.id.0) {
            self.failure
        } else {
            None
        }
        .unwrap_or(Verdict::Clear(
            Confidence::new(1.0).unwrap_or_else(|_| unreachable!()),
        ));
        Evidence {
            frame: frame.id,
            timestamp: frame.timestamp,
            scope: EvidenceScope::ReplayFixture,
            verdict,
        }
    }
}
fn run(failure: Option<Verdict>, missing: bool) -> anyhow::Result<ReplayPipeline> {
    let source = Frames { index: 0, missing };
    let config = Config {
        frame_size: source.size(),
        ..Config::default()
    };
    assert_eq!(config.processing.virtual_aim_delay_us, 500_000);
    let calibration = Some(Calibration::virtual_plane(config.frame_size)?);
    ReplayPipeline::new(
        Box::new(source),
        Box::new(Detector { failure }),
        config,
        "500ms-delay".into(),
        calibration,
    )
}

#[test]
fn default_delay_waits_500ms_uses_current_position_and_cycles_targets() -> anyhow::Result<()> {
    let mut pipeline = run(None, false)?;
    let mut issued = Vec::new();
    while pipeline.step()? {
        let s = pipeline.snapshot();
        if let Some(aim) = s.aim.filter(|aim| aim.issued) {
            assert!(
                s.predictions
                    .contains(&(aim.request.target, aim.request.predicted_pixel))
            );
            if issued.is_empty() {
                assert_eq!(s.timestamp.0, 800_000);
                assert!(
                    aim.request.predicted_pixel.x() > 18.0,
                    "used the old position instead of the current target"
                );
            }
            issued.push((s.frame.0, aim.request.target));
        } else if s.frame.0 >= 3 {
            assert!(matches!(s.aim_status, AimStatus::WaitingVirtual { .. }));
            assert!(s.aim.is_none());
        }
    }
    assert_eq!(
        issued.iter().map(|(frame, _)| *frame).collect::<Vec<_>>(),
        vec![8, 14, 20]
    );
    assert_ne!(issued[0].1, issued[1].1);
    assert_eq!(issued[0].1, issued[2].1);
    let before = pipeline.counters().issued_aims;
    for _ in 0..3 {
        assert!(!pipeline.step()?);
    }
    assert_eq!(pipeline.counters().issued_aims, before);
    pipeline.seek(8)?;
    assert!(pipeline.snapshot().aim.is_some_and(|aim| aim.issued));
    pipeline.seek(4)?;
    assert!(matches!(
        pipeline.snapshot().aim_status,
        AimStatus::WaitingVirtual {
            remaining_us: 400_000,
            ..
        }
    ));
    Ok(())
}

#[test]
fn every_lockout_cancels_the_wait_and_clear_recovery_restarts_500ms() -> anyhow::Result<()> {
    for failure in [
        Verdict::Hazard(camera::Hazard::Human),
        Verdict::Hazard(camera::Hazard::Dog),
        Verdict::Hazard(camera::Hazard::Cat),
        Verdict::Unavailable,
        Verdict::Error,
        Verdict::Timeout,
        Verdict::Stale,
        Verdict::Invalid,
        Verdict::Uncertain,
    ] {
        let mut pipeline = run(Some(failure), false)?;
        while pipeline.step()? {
            let s = pipeline.snapshot();
            if (3..9).contains(&s.frame.0) {
                assert!(matches!(s.safety, SafetyState::SafetyLockout(_)));
                assert!(s.aim.is_some_and(|aim| !aim.issued));
            }
            if s.frame.0 == 9 {
                assert!(matches!(
                    s.aim_status,
                    AimStatus::WaitingVirtual {
                        remaining_us: 500_000,
                        ..
                    }
                ));
            }
            if s.frame.0 < 14 {
                assert!(s.aim.is_none_or(|aim| !aim.issued));
            }
            if s.frame.0 == 14 {
                assert!(s.aim.is_some_and(|aim| aim.issued));
            }
        }
    }
    Ok(())
}

#[test]
fn lost_target_cannot_complete_an_old_wait_and_shutdown_cancels() -> anyhow::Result<()> {
    let mut pipeline = run(None, true)?;
    for _ in 0..9 {
        assert!(pipeline.step()?);
    }
    let s = pipeline.snapshot();
    assert_eq!(s.frame.0, 8);
    assert!(
        s.aim.is_none(),
        "lost target completed its previous deadline"
    );
    pipeline.shutdown();
    pipeline.step()?;
    let s = pipeline.snapshot();
    assert_eq!(
        s.safety,
        SafetyState::SafetyLockout(LockoutReason::Shutdown)
    );
    assert!(s.aim.is_none_or(|aim| !aim.issued));
    Ok(())
}

#[test]
fn delay_applies_to_dog_human_and_hand_disk_replays() -> anyhow::Result<()> {
    use camera::replay::ImageSequence;
    for kind in ["dog", "human", "hand"] {
        let folder = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(format!("{kind}-transition"));
        let config = Config::load(&folder.join("config.json"))?;
        assert_eq!(config.processing.virtual_aim_delay_us, 500_000);
        for (phase, expected) in [("entry", 0), ("exit", 1), ("cycle", 1)] {
            let source = ImageSequence::open(&folder.join(format!("{kind}-{phase}.json")))?;
            let calibration = Some(Calibration::virtual_plane(source.size())?);
            let mut pipeline = ReplayPipeline::new(
                Box::new(source),
                Box::new(safety::FixtureDetector),
                config.clone(),
                format!("{kind}-{phase}-delay"),
                calibration,
            )?;
            while pipeline.step()? {
                let hazard = pipeline
                    .frame()
                    .truth
                    .as_ref()
                    .is_some_and(|truth| truth.hazard.is_some());
                let s = pipeline.snapshot();
                if hazard {
                    assert!(s.aim.is_none_or(|aim| !aim.issued));
                }
                if s.aim.is_some_and(|aim| aim.issued) {
                    assert_eq!(s.frame.0, if phase == "exit" { 8 } else { 13 });
                }
            }
            assert_eq!(pipeline.counters().issued_aims, expected, "{kind}-{phase}");
        }
    }
    Ok(())
}
