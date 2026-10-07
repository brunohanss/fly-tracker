//! Disk-image regressions. Fixture labels test the command interlock, not recognition.
use aiming::calibration::Calibration;
use app::pipeline::ReplayPipeline;
use camera::{FrameSource, replay::ImageSequence};
use fly_core::{LockoutReason, SafetyState, TargetId, TrackLifecycle, config::Config};
use safety::{FixtureDetector, SafetyDetector};
use std::path::PathBuf;

fn directory(fixture: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(fixture)
}
fn pipeline(
    fixture: &str,
    name: &str,
    detector: Box<dyn SafetyDetector>,
) -> anyhow::Result<ReplayPipeline> {
    let mut config = Config::load(&directory(fixture).join("config.json"))?;
    // These historical control tests isolate immediate interlock behavior.
    config.processing.virtual_aim_delay_us = 0;
    let source = ImageSequence::open(&directory(fixture).join(name))?;
    let calibration = Some(Calibration::virtual_plane(source.size())?);
    ReplayPipeline::new(Box::new(source), detector, config, name.into(), calibration)
}

pub fn check_sequence(
    fixture: &str,
    target_count: usize,
    reason: LockoutReason,
    name: &str,
    dogs: &[bool],
    expected_issued: u64,
) -> anyhow::Result<()> {
    let mut run = pipeline(fixture, name, Box::new(FixtureDetector))?;
    let mut index = 0;
    let mut ids: Option<Vec<TargetId>> = None;
    let mut accepted_targets = Vec::new();
    while run.step()? {
        let snapshot = run.snapshot();
        assert_eq!(snapshot.frame.0, index as u64);
        assert_eq!(
            snapshot.tracks.len(),
            target_count,
            "{name} frame {}: fixed flies must remain tracked",
            index + 1
        );
        let current_ids: Vec<_> = snapshot.tracks.iter().map(|track| track.id).collect();
        if let Some(previous) = &ids {
            assert_eq!(&current_ids, previous, "track IDs changed at transition");
        } else {
            ids = Some(current_ids);
        }
        let truth = run
            .frame()
            .truth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing fixture truth"))?;
        assert_eq!(truth.targets.len(), target_count);
        for target in &truth.targets {
            assert!(target.visible);
            assert!(
                snapshot
                    .tracks
                    .iter()
                    .any(|track| (track.position.x() - target.position.x())
                        .hypot(track.position.y() - target.position.y())
                        <= 6.0),
                "{name} frame {}: fixed target {} missing",
                index + 1,
                target.id.0
            );
        }
        if index >= 2 {
            assert!(
                snapshot
                    .tracks
                    .iter()
                    .all(|track| track.lifecycle == TrackLifecycle::Confirmed)
            );
        }
        if dogs[index] {
            assert_eq!(snapshot.safety, SafetyState::SafetyLockout(reason));
            assert!(
                snapshot.aim.is_none_or(|record| !record.issued),
                "{name}: dog frame issued a command"
            );
            if index >= 2 {
                assert_eq!(
                    snapshot.aim.and_then(|record| record.suppressed_by),
                    Some(reason)
                );
            }
        } else {
            assert_eq!(snapshot.safety, SafetyState::Clear);
            if index >= 2 {
                let record = snapshot
                    .aim
                    .ok_or_else(|| anyhow::anyhow!("clear confirmed frame has no request"))?;
                assert!(
                    record.issued,
                    "{name}: aiming did not start/resume on fresh clear frame"
                );
                accepted_targets.push(record.request.target);
                assert!(
                    snapshot
                        .predictions
                        .contains(&(record.request.target, record.request.predicted_pixel))
                );
            }
        }
        index += 1;
    }
    assert_eq!(index, dogs.len());
    let report = run.report()?;
    assert_eq!(report.counters.issued_aims, expected_issued);
    assert_eq!(report.counters.unsafe_commands, 0);
    assert_eq!(
        report.counters.lockout_frames,
        dogs.iter().filter(|dog| **dog).count() as u64
    );
    if name.ends_with("-exit.json") {
        accepted_targets.sort();
        accepted_targets.dedup();
        assert_eq!(
            accepted_targets.len(),
            target_count.min(5),
            "five distinct targets must receive a request after the hazard leaves"
        );
    }
    // Seeking must reproduce the complete interlock and target-selection state.
    run.seek(0)?;
    while run.step()? {}
    assert_eq!(run.report()?.counters, report.counters);
    Ok(())
}

#[cfg(feature = "nanodet-model-tests")]
pub fn check_model_sequences(
    fixture: &str,
    names: &[&str],
    reason: LockoutReason,
) -> anyhow::Result<()> {
    use fly_core::config::SafetyBackend;
    // This test fails if prerequisites are absent. It never falls back to labels.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut config = Config::load(&directory(fixture).join("config.json"))?;
    config.max_frame_age_us = 1_000_000;
    config.max_safety_age_us = 1_000_000;
    config.safety_backend = SafetyBackend::Nanodet {
        python: std::env::var_os("FLY_TRACKER_TEST_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join(".venv-safety/Scripts/python.exe")),
        worker: root.join("tools/safety/nanodet_worker.py"),
        param: root.join("models/nanodet-reference/coco.torchscript.ncnn.param"),
        weights: root.join("models/nanodet-reference/coco.torchscript.ncnn.bin"),
        hazard_threshold: fly_core::Confidence::new(0.4)?,
        timeout_us: 1_000_000,
        threads: 2,
    };
    for &name in names {
        let source = ImageSequence::open(&directory(fixture).join(name))?;
        let calibration = Some(Calibration::virtual_plane(source.size())?);
        let mut run = ReplayPipeline::new(
            Box::new(source),
            app::safety_backend::build(&config.safety_backend),
            config.clone(),
            name.into(),
            calibration,
        )?;
        while run.step()? {
            let dog = run
                .frame()
                .truth
                .as_ref()
                .is_some_and(|truth| truth.hazard.is_some());
            let s = run.snapshot();
            assert!(
                s.safety_backend.contains("ready; clearance unvalidated"),
                "inference unavailable or failed: {}",
                s.safety_backend
            );
            assert!(s.aim.is_none_or(|aim| !aim.issued));
            if dog {
                assert_eq!(
                    s.safety,
                    SafetyState::SafetyLockout(reason),
                    "protected hazard was not detected: {}",
                    s.safety_backend
                );
            } else {
                assert_eq!(
                    s.safety,
                    SafetyState::SafetyLockout(LockoutReason::InsufficientConfidence)
                );
            }
        }
        assert_eq!(run.counters().issued_aims, 0);
    }
    Ok(())
}
