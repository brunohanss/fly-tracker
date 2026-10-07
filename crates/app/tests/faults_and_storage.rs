use aiming::calibration::Calibration;
use app::pipeline::ReplayPipeline;
use camera::{
    CameraError, Frame, FrameSource,
    synthetic::{Scenario, SyntheticSource},
};
use fly_core::{FrameSize, LockoutReason, SafetyState, config::Config};
use safety::FixtureDetector;

struct ModifiedSource {
    source: SyntheticSource,
    strip_truth: bool,
    fail_after: Option<u64>,
    read: u64,
}
impl FrameSource for ModifiedSource {
    fn size(&self) -> FrameSize {
        self.source.size()
    }
    fn reset(&mut self) -> Result<(), CameraError> {
        self.read = 0;
        self.source.reset()
    }
    fn next_into(&mut self, frame: &mut Frame) -> Result<bool, CameraError> {
        if self.fail_after == Some(self.read) {
            return Err(CameraError::Format("injected source fault"));
        }
        let result = self.source.next_into(frame)?;
        if result {
            self.read += 1;
            if self.strip_truth {
                frame.truth = None;
            }
        }
        Ok(result)
    }
}
fn pipeline(strip_truth: bool, fail_after: Option<u64>) -> anyhow::Result<ReplayPipeline> {
    let mut config = Config {
        frame_size: FrameSize::new(96, 64)?,
        ..Config::default()
    };
    config.processing.virtual_aim_delay_us = 0;
    let source = Box::new(ModifiedSource {
        source: SyntheticSource::new(config.frame_size, Scenario::SlowFly, 42, 20, 10_000)?,
        strip_truth,
        fail_after,
        read: 0,
    });
    let calibration = Some(Calibration::virtual_plane(config.frame_size)?);
    ReplayPipeline::new(
        source,
        Box::new(FixtureDetector),
        config,
        "fault-test".into(),
        calibration,
    )
}
#[test]
fn missing_truth_is_unavailable_and_reports_are_versioned() -> anyhow::Result<()> {
    let mut pipeline = pipeline(true, None)?;
    while pipeline.step()? {}
    let report = pipeline.report()?;
    assert_eq!(report.counters.false_positives, None);
    assert_eq!(report.counters.false_negatives, None);
    assert!(report.prediction_error_pixels.iter().all(Option::is_none));
    assert_eq!(report.counters.issued_aims, 0);
    let path = std::env::temp_dir().join(format!(
        "fly-tracker-report-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    report.save(&path)?;
    let loaded = telemetry::RunMetrics::load(&path)?;
    assert_eq!(loaded.counters, report.counters);
    assert!(report.save(&path).is_err());
    std::fs::remove_file(&path)?;
    let mut invalid = loaded;
    invalid.version = 2;
    assert!(invalid.validate().is_err());
    invalid.version = 1;
    invalid.counters.false_positives = Some(0);
    assert!(invalid.validate().is_err());
    Ok(())
}
#[test]
fn source_fault_revokes_output_and_requires_reset() -> anyhow::Result<()> {
    let mut pipeline = pipeline(false, Some(8))?;
    for _ in 0..8 {
        assert!(pipeline.step()?);
    }
    assert!(pipeline.counters().issued_aims > 0);
    assert!(pipeline.step().is_err());
    let snapshot = pipeline.snapshot();
    assert_eq!(
        snapshot.safety,
        SafetyState::SafetyLockout(LockoutReason::Shutdown)
    );
    assert!(!snapshot.camera_available);
    assert!(snapshot.aim.is_none());
    assert!(pipeline.step().is_err());
    assert!(pipeline.report().is_err());
    pipeline.reset()?;
    assert!(pipeline.step()?);
    Ok(())
}
#[test]
fn rejected_seek_preserves_known_source_state() -> anyhow::Result<()> {
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
    let mut pipeline = ReplayPipeline::new(
        source,
        Box::new(FixtureDetector),
        config,
        "seek".into(),
        calibration,
    )?;
    for _ in 0..5 {
        pipeline.step()?;
    }
    let before = pipeline.snapshot();
    assert!(pipeline.seek(20).is_err());
    let after = pipeline.snapshot();
    assert_eq!(before.counters, after.counters);
    assert_eq!(before.tracks, after.tracks);
    Ok(())
}
#[test]
fn failed_unknown_length_seek_restores_completed_run() -> anyhow::Result<()> {
    let mut pipeline = pipeline(true, None)?;
    while pipeline.step()? {}
    let before = pipeline.snapshot();
    assert!(pipeline.seek(100).is_err());
    let after = pipeline.snapshot();
    assert!(after.complete);
    assert_eq!(before.counters, after.counters);
    assert_eq!(before.tracks, after.tracks);
    Ok(())
}
