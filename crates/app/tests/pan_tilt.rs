use aiming::{
    calibration::Calibration,
    pan_tilt::{MotionState, PanTiltSnapshot},
};
use app::pipeline::ReplayPipeline;
use camera::{
    CameraError, Frame, FrameSource,
    synthetic::{Scenario, SyntheticSource},
};
use fly_core::{
    Confidence, FrameId, FrameSize, FrameTimestamp, SafetyState, config::Config,
    servo::PanTiltConfig,
};
use ratatui::{Terminal, backend::TestBackend};
use safety::{Evidence, EvidenceScope, SafetyDetector, Verdict};
use tui::{app::Dashboard, views::ConnectedView};

struct Frames {
    index: u64,
    missing: bool,
    fault: bool,
}
impl FrameSource for Frames {
    fn size(&self) -> FrameSize {
        FrameSize::new(96, 64).unwrap_or_else(|_| unreachable!())
    }
    fn reset(&mut self) -> Result<(), CameraError> {
        self.index = 0;
        Ok(())
    }
    fn next_into(&mut self, frame: &mut Frame) -> Result<bool, CameraError> {
        if self.fault && self.index == 10 {
            return Err(CameraError::Format("injected servo replay fault"));
        }
        if self.index == 25 {
            return Ok(false);
        }
        frame.pixels.fill(160);
        if self.index > 0 && !(self.missing && self.index >= 10) {
            for row in 11..=13 {
                for col in 11..=13 {
                    frame.pixels[row * 96 + col] = 20;
                }
            }
        }
        frame.id = FrameId(self.index);
        frame.timestamp = FrameTimestamp(self.index * 100_000);
        self.index += 1;
        Ok(true)
    }
}
struct Detector {
    hazard: bool,
}
impl SafetyDetector for Detector {
    fn evaluate(&mut self, frame: &Frame) -> Evidence {
        Evidence {
            frame: frame.id,
            timestamp: frame.timestamp,
            scope: EvidenceScope::ReplayFixture,
            verdict: if self.hazard && (10..13).contains(&frame.id.0) {
                Verdict::Hazard(camera::Hazard::Human)
            } else {
                Verdict::Clear(Confidence::new(1.0).unwrap_or_else(|_| unreachable!()))
            },
        }
    }
}
fn pipeline(hazard: bool, missing: bool, fault: bool) -> anyhow::Result<ReplayPipeline> {
    let source = Frames {
        index: 0,
        missing,
        fault,
    };
    let mut profile = PanTiltConfig::simulation_example();
    profile.pan.max_speed_degrees_per_s = 10.0;
    profile.tilt.max_speed_degrees_per_s = 10.0;
    let config = Config {
        frame_size: source.size(),
        pan_tilt_simulation: Some(profile),
        ..Config::default()
    };
    let calibration = Some(Calibration::virtual_plane(config.frame_size)?);
    ReplayPipeline::new(
        Box::new(source),
        Box::new(Detector { hazard }),
        config,
        "servo-test".into(),
        calibration,
    )
}
fn servos(pipeline: &mut ReplayPipeline) -> anyhow::Result<PanTiltSnapshot> {
    pipeline
        .snapshot()
        .pan_tilt
        .ok_or_else(|| anyhow::anyhow!("missing simulator"))
}
#[test]
fn simulation_is_deterministic_across_reset_seek_and_pause() -> anyhow::Result<()> {
    let mut run = pipeline(false, false, false)?;
    let mut snapshots = Vec::new();
    while run.step()? {
        snapshots.push(servos(&mut run)?);
    }
    assert!(snapshots.iter().any(|s| s.state == MotionState::Moving));
    assert_eq!(servos(&mut run)?.state, MotionState::Cancelled);
    for index in [15, 4, 9, 8] {
        run.seek(index)?;
        assert_eq!(servos(&mut run)?, snapshots[index as usize]);
    }
    run.set_paused(true);
    let paused = servos(&mut run)?;
    assert_eq!(servos(&mut run)?, paused);
    run.reset()?;
    assert_eq!(servos(&mut run)?.state, MotionState::Idle);
    assert_eq!(servos(&mut run)?.simulated.pan.degrees(), 0.0);
    for expected in snapshots {
        assert!(run.step()?);
        assert_eq!(servos(&mut run)?, expected);
    }
    Ok(())
}
#[test]
fn lockout_freezes_motion_and_recovery_requires_a_new_full_wait() -> anyhow::Result<()> {
    let mut run = pipeline(true, false, false)?;
    let mut frozen = None;
    while run.step()? {
        let snapshot = run.snapshot();
        let servo = snapshot
            .pan_tilt
            .ok_or_else(|| anyhow::anyhow!("missing servos"))?;
        if snapshot.frame.0 == 9 {
            frozen = Some(servo.simulated);
            assert_eq!(servo.state, MotionState::Moving);
        }
        if (10..18).contains(&snapshot.frame.0) {
            assert_eq!(
                servo.simulated,
                frozen.ok_or_else(|| anyhow::anyhow!("missing position"))?
            );
            assert!(servo.requested.is_none());
            assert_eq!(servo.state, MotionState::Cancelled);
        }
        if snapshot.frame.0 == 18 {
            assert!(snapshot.aim.is_some_and(|a| a.issued));
            assert!(servo.requested.is_some());
        }
    }
    Ok(())
}
#[test]
fn target_loss_fault_shutdown_and_end_of_source_remove_destinations() -> anyhow::Result<()> {
    let mut lost = pipeline(false, true, false)?;
    while lost.step()? {
        if lost.frame().id.0 >= 10 {
            assert!(servos(&mut lost)?.requested.is_none());
        }
    }
    let mut fault = pipeline(false, false, true)?;
    for _ in 0..10 {
        assert!(fault.step()?);
    }
    let before = servos(&mut fault)?.simulated;
    assert!(fault.step().is_err());
    assert_eq!(servos(&mut fault)?.simulated, before);
    assert_eq!(servos(&mut fault)?.state, MotionState::Cancelled);
    assert!(!fault.snapshot().camera_available);
    let mut stopped = pipeline(false, false, false)?;
    for _ in 0..10 {
        stopped.step()?;
    }
    stopped.shutdown();
    assert_eq!(servos(&mut stopped)?.state, MotionState::Cancelled);
    stopped.step()?;
    assert!(servos(&mut stopped)?.requested.is_none());
    Ok(())
}
#[test]
fn safety_positive_synthetic_replays_never_move_or_request_servos() -> anyhow::Result<()> {
    for scenario in Scenario::ALL.into_iter().filter(|s| s.hazard().is_some()) {
        for seed in [42, 73] {
            let config = Config {
                frame_size: FrameSize::new(96, 64)?,
                pan_tilt_simulation: Some(PanTiltConfig::simulation_example()),
                ..Config::default()
            };
            let source = SyntheticSource::new(config.frame_size, scenario, seed, 20, 100_000)?;
            let calibration = Some(Calibration::virtual_plane(config.frame_size)?);
            let mut run = ReplayPipeline::new(
                Box::new(source),
                Box::new(safety::FixtureDetector),
                config,
                "servo-safety".into(),
                calibration,
            )?;
            while run.step()? {
                let snapshot = run.snapshot();
                assert!(
                    matches!(snapshot.safety, SafetyState::SafetyLockout(_)),
                    "{scenario:?} seed={seed}"
                );
                let s = servos(&mut run)?;
                assert!(s.requested.is_none(), "{scenario:?} seed={seed}");
                assert_eq!(s.simulated.pan.degrees(), 0.0, "{scenario:?} seed={seed}");
                assert_eq!(s.simulated.tilt.degrees(), 0.0, "{scenario:?} seed={seed}");
            }
            assert_eq!(run.counters().issued_aims, 0, "{scenario:?} seed={seed}");
        }
    }
    Ok(())
}
#[test]
fn dashboard_distinguishes_requested_simulated_and_physical_output() -> anyhow::Result<()> {
    let mut run = pipeline(false, false, false)?;
    for _ in 0..10 {
        run.step()?;
    }
    let dashboard = Dashboard {
        connected: Some(ConnectedView::new(run.snapshot())),
        ..Dashboard::default()
    };
    let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
    terminal.draw(|frame| dashboard.render(frame))?;
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    for label in [
        "SIMULATION",
        "Simulated pan/tilt",
        "Requested pan/tilt",
        "not sent",
        "OUTPUT DISABLED",
    ] {
        assert!(text.contains(label), "missing {label}");
    }
    Ok(())
}
