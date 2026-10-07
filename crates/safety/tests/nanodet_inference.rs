//! Opt-in real-model tests. No synthetic inference results are used.
use camera::{Frame, FrameSource, Hazard, replay::ImageSequence};
use fly_core::{Confidence, FrameId, FrameSize, FrameTimestamp, SafetyState, config::Config};
use safety::{
    EvidenceScope, SafetyAuthority, SafetyDetector, Verdict,
    nanodet::{NanoDetConfig, NanoDetDetector},
};
use std::{path::PathBuf, time::Duration};

fn configuration() -> anyhow::Result<NanoDetConfig> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Ok(NanoDetConfig {
        python: std::env::var("FLY_TRACKER_TEST_PYTHON")?.into(),
        worker: root.join("tools/safety/nanodet_worker.py"),
        param: root.join("models/nanodet-reference/coco.torchscript.ncnn.param"),
        weights: root.join("models/nanodet-reference/coco.torchscript.ncnn.bin"),
        hazard_threshold: Confidence::new(0.4)?,
        deadline: Duration::from_secs(10),
        threads: 2,
        scope: EvidenceScope::ReplayFixture,
    })
}

#[test]
#[ignore = "requires local ncnn environment and pinned model weights"]
fn rust_calls_real_model_repeatedly_and_fails_closed() -> anyhow::Result<()> {
    let mut detector = NanoDetDetector::start(configuration()?)?;
    for (index, (width, height)) in [(640, 480), (480, 640), (416, 416), (320, 192)]
        .into_iter()
        .enumerate()
    {
        let mut frame = Frame::new(FrameSize::new(width, height)?);
        frame.id = FrameId(index as u64);
        frame.timestamp = FrameTimestamp(index as u64 * 10_000);
        let evidence = detector.evaluate(&frame);
        anyhow::ensure!(
            detector.last_error.is_none(),
            "inference failed: {:?}",
            detector.last_error
        );
        anyhow::ensure!(detector.last_scores.is_some(), "no native model scores");
        assert!(matches!(
            evidence.verdict,
            Verdict::Uncertain | Verdict::Hazard(_)
        ));
        let mut gate = SafetyAuthority::new(&Config::default())?;
        gate.record_camera(frame.id, frame.timestamp);
        gate.observe(evidence);
        assert!(matches!(
            gate.state_at(frame.timestamp),
            SafetyState::SafetyLockout(_)
        ));
        assert!(
            gate.execute(
                frame.timestamp,
                frame.id,
                EvidenceScope::ReplayFixture,
                || panic!("unexpected output")
            )
            .is_err()
        );
    }
    // A genuine model load failure must become an error verdict through Rust.
    let mut config = configuration()?;
    config.param = "missing-model.param".into();
    let mut broken = NanoDetDetector::start(config)?;
    let frame = Frame::new(FrameSize::new(16, 16)?);
    assert!(matches!(broken.evaluate(&frame).verdict, Verdict::Error));
    assert!(broken.last_error.is_some());
    Ok(())
}

#[test]
#[ignore = "requires downloaded real-image test sequence"]
fn rust_real_monochrome_photographs_trigger_hazard_lockout() -> anyhow::Result<()> {
    let mut detector = NanoDetDetector::start(configuration()?)?;
    let manifest = PathBuf::from(std::env::var("FLY_TRACKER_NANODET_IMAGES")?);
    let mut sequence = ImageSequence::open(&manifest)?;
    let mut frame = Frame::new(sequence.size());
    let mut count = 0;
    while sequence.next_into(&mut frame)? {
        let expected = frame
            .truth
            .as_ref()
            .and_then(|truth| truth.hazard)
            .ok_or_else(|| anyhow::anyhow!("unlabelled test frame"))?;
        anyhow::ensure!(
            matches!(expected, Hazard::Human | Hazard::Dog),
            "unexpected fixture"
        );
        let evidence = detector.evaluate(&frame);
        anyhow::ensure!(
            detector.last_error.is_none(),
            "inference fault: {:?}",
            detector.last_error
        );
        anyhow::ensure!(
            matches!(evidence.verdict, Verdict::Hazard(_)),
            "frame {} expected protected {:?}, got {:?}, scores {:?}",
            frame.id.0,
            expected,
            evidence.verdict,
            detector.last_scores
        );
        // The grayscale dog is labelled cat by this model. Both are protected hazards.
        // This checks safety suppression; exact classification is reported separately.
        if expected == Hazard::Human {
            assert!(matches!(evidence.verdict, Verdict::Hazard(Hazard::Human)));
        }
        let mut gate = SafetyAuthority::new(&Config::default())?;
        gate.record_camera(frame.id, frame.timestamp);
        gate.observe(evidence);
        assert!(matches!(
            gate.state_at(frame.timestamp),
            SafetyState::SafetyLockout(_)
        ));
        assert!(
            gate.execute(
                frame.timestamp,
                frame.id,
                EvidenceScope::ReplayFixture,
                || panic!("output escaped image hazard")
            )
            .is_err()
        );
        count += 1;
    }
    anyhow::ensure!(count >= 2, "require at least a human and a dog fixture");
    Ok(())
}
