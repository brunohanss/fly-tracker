use aiming::calibration::Calibration;
use app::pipeline::ReplayPipeline;
use camera::{
    FrameSource,
    synthetic::{Scenario, SyntheticSource},
};
use fly_core::{FrameSize, config::Config};
use safety::{FixtureDetector, UnavailableDetector};

fn pipeline(scenario: Scenario, seed: u64) -> anyhow::Result<ReplayPipeline> {
    let mut config = Config {
        frame_size: FrameSize::new(96, 64)?,
        ..Config::default()
    };
    config.processing.virtual_aim_delay_us = 0;
    let source: Box<dyn FrameSource> = Box::new(SyntheticSource::new(
        config.frame_size,
        scenario,
        seed,
        60,
        10_000,
    )?);
    let calibration = Some(Calibration::virtual_plane(config.frame_size)?);
    ReplayPipeline::new(
        source,
        Box::new(FixtureDetector),
        config,
        format!("{scenario:?}:{seed}:60:10000"),
        calibration,
    )
}
#[test]
fn complete_seeded_suite_has_no_unsafe_commands_and_repeats() -> anyhow::Result<()> {
    for seed in [0, 42] {
        for scenario in Scenario::ALL {
            let mut run = pipeline(scenario, seed)?;
            while run.step()? {}
            let first = run.report()?;
            first.validate()?;
            assert_eq!(
                first.counters.unsafe_commands, 0,
                "seed={seed}, scenario={scenario:?}"
            );
            if scenario.hazard().is_some() {
                assert_eq!(first.counters.lockout_frames, 60);
                assert_eq!(first.counters.issued_aims, 0);
            }
            run.reset()?;
            while run.step()? {}
            let second = run.report()?;
            assert_eq!(
                first.counters, second.counters,
                "seed={seed}, scenario={scenario:?}"
            );
            assert_eq!(
                first.prediction_error_pixels,
                second.prediction_error_pixels
            );
        }
    }
    Ok(())
}
#[test]
fn seek_restores_complete_downstream_state() -> anyhow::Result<()> {
    let mut run = pipeline(Scenario::Occlusion, 13)?;
    for _ in 0..31 {
        assert!(run.step()?);
    }
    let expected = run.snapshot();
    run.seek(30)?;
    let actual = run.snapshot();
    assert_eq!(expected.frame, actual.frame);
    assert_eq!(expected.tracks, actual.tracks);
    assert_eq!(expected.counters, actual.counters);
    assert_eq!(expected.aim, actual.aim);
    assert_eq!(expected.preview.pixels, actual.preview.pixels);
    Ok(())
}
#[test]
fn missing_detector_keeps_tracking_but_suppresses_aiming() -> anyhow::Result<()> {
    let mut config = Config {
        frame_size: FrameSize::new(96, 64)?,
        ..Config::default()
    };
    config.processing.virtual_aim_delay_us = 0;
    let source = Box::new(SyntheticSource::new(
        config.frame_size,
        Scenario::SlowFly,
        42,
        20,
        10_000,
    )?);
    let calibration = Some(Calibration::virtual_plane(config.frame_size)?);
    let mut run = ReplayPipeline::new(
        source,
        Box::new(UnavailableDetector),
        config,
        "test".into(),
        calibration,
    )?;
    while run.step()? {}
    let report = run.report()?;
    assert!(report.counters.acquired_targets > 0);
    assert!(report.counters.suppressed_aims > 0);
    assert_eq!(report.counters.issued_aims, 0);
    Ok(())
}

#[test]
fn idle_and_completed_snapshots_preserve_fps_and_active_time() -> anyhow::Result<()> {
    let mut run = pipeline(Scenario::SlowFly, 42)?;
    for _ in 0..10 {
        assert!(run.step()?);
    }
    run.set_paused(true);
    let paused = run.snapshot();
    for _ in 0..20 {
        let idle = run.snapshot();
        assert_eq!(idle.fps, paused.fps);
        assert_eq!(idle.elapsed_seconds, paused.elapsed_seconds);
    }
    run.set_paused(false);
    while run.step()? {}
    let done = run.snapshot();
    assert!(done.complete);
    assert!(done.fps.is_some_and(|fps| fps > 0.0));
    for _ in 0..20 {
        assert!(!run.step()?);
        let idle = run.snapshot();
        assert_eq!(idle.fps, done.fps);
        assert_eq!(idle.elapsed_seconds, done.elapsed_seconds);
    }
    Ok(())
}

#[test]
fn stationary_initial_targets_confirm_with_zero_velocity_and_safety_still_overrides()
-> anyhow::Result<()> {
    struct StillSource {
        frame: camera::Frame,
        next: u64,
    }
    impl FrameSource for StillSource {
        fn size(&self) -> FrameSize {
            self.frame.size
        }
        fn next_into(&mut self, output: &mut camera::Frame) -> Result<bool, camera::CameraError> {
            if self.next == 5 {
                return Ok(false);
            }
            output.pixels.copy_from_slice(&self.frame.pixels);
            output.truth.clone_from(&self.frame.truth);
            output.id = fly_core::FrameId(self.next);
            output.timestamp = fly_core::FrameTimestamp(self.next * 10_000);
            self.next += 1;
            Ok(true)
        }
        fn reset(&mut self) -> Result<(), camera::CameraError> {
            self.next = 0;
            Ok(())
        }
    }
    let mut config = Config {
        frame_size: FrameSize::new(96, 64)?,
        ..Config::default()
    };
    config.processing.virtual_aim_delay_us = 0;
    for hazard in [None, Some(camera::Hazard::Human)] {
        let mut frame = camera::Frame::new(config.frame_size);
        frame.pixels.fill(160);
        let mut targets = Vec::new();
        for (i, (x, y)) in [(12, 12), (30, 12), (50, 20), (70, 40), (85, 52)]
            .into_iter()
            .enumerate()
        {
            for row in y - 1..=y + 1 {
                for col in x - 1..=x + 1 {
                    frame.pixels[row * 96 + col] = 20;
                }
            }
            targets.push(camera::TruthTarget {
                id: fly_core::TargetId(i as u64 + 1),
                position: fly_core::ScenePosition::new(x as f64, y as f64)?,
                visible: true,
            });
        }
        frame.truth = Some(camera::GroundTruth { targets, hazard });
        let calibration = Some(Calibration::virtual_plane(config.frame_size)?);
        let mut run = ReplayPipeline::new(
            Box::new(StillSource { frame, next: 0 }),
            Box::new(FixtureDetector),
            config.clone(),
            "stationary-initial".into(),
            calibration,
        )?;
        while run.step()? {
            let snapshot = run.snapshot();
            assert_eq!(snapshot.tracks.len(), 5);
            for track in &snapshot.tracks {
                assert_eq!(track.velocity.x(), 0.0);
                assert_eq!(track.velocity.y(), 0.0);
            }
        }
        let report = run.report()?;
        assert_eq!(report.counters.detections, 25);
        assert_eq!(report.counters.acquired_targets, 5);
        assert_eq!(report.counters.unsafe_commands, 0);
        if hazard.is_some() {
            assert_eq!(report.counters.issued_aims, 0);
            assert_eq!(report.counters.lockout_frames, 5);
        } else {
            assert_eq!(report.counters.issued_aims, 3);
        }
    }
    Ok(())
}
