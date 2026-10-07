use camera::{Frame, GroundTruth, Hazard};
use fly_core::{Confidence, FrameId, FrameSize, FrameTimestamp, SafetyState, config::Config};
use safety::{
    EvidenceScope, SafetyAuthority, SafetyDetector, Verdict,
    nanodet::{NanoDetConfig, NanoDetDetector},
};
use std::{path::PathBuf, time::Duration};

// Explicit opt-in: CI remains independent from Python installation.
#[test]
#[ignore = "set FLY_TRACKER_TEST_PYTHON and run --ignored to test real worker processes"]
fn native_worker_protocol_faults_and_hazards() -> anyhow::Result<()> {
    let python = PathBuf::from(std::env::var("FLY_TRACKER_TEST_PYTHON")?);
    let worker =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/safety/test_worker.py");
    let mut frame = Frame::new(FrameSize::new(16, 16)?);
    frame.id = FrameId(7);
    frame.timestamp = FrameTimestamp(100);
    frame.truth = Some(GroundTruth {
        targets: vec![],
        hazard: Some(Hazard::Human),
    });
    for mode in [
        "human", "dog", "cat", "clear", "timeout", "crash", "mismatch",
    ] {
        let mut detector = NanoDetDetector::start(NanoDetConfig {
            python: python.clone(),
            worker: worker.clone(),
            param: mode.into(),
            weights: "unused".into(),
            hazard_threshold: Confidence::new(0.4)?,
            deadline: Duration::from_millis(500),
            threads: 1,
            scope: EvidenceScope::ReplayFixture,
        })?;
        let evidence = detector.evaluate(&frame);
        match mode {
            "human" => assert!(matches!(evidence.verdict, Verdict::Hazard(Hazard::Human))),
            "dog" => assert!(matches!(evidence.verdict, Verdict::Hazard(Hazard::Dog))),
            "cat" => assert!(matches!(evidence.verdict, Verdict::Hazard(Hazard::Cat))),
            "clear" => assert!(matches!(evidence.verdict, Verdict::Uncertain)),
            "timeout" => assert!(matches!(evidence.verdict, Verdict::Timeout)),
            _ => assert!(matches!(evidence.verdict, Verdict::Error)),
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
                || panic!("output escaped lockout")
            )
            .is_err()
        );
        if matches!(mode, "timeout" | "crash" | "mismatch") {
            let later = detector.evaluate(&frame).verdict;
            if mode == "timeout" {
                assert!(matches!(later, Verdict::Timeout));
            } else {
                assert!(matches!(later, Verdict::Error));
            }
        } else {
            // Persistent worker serves another frame without model/process restart.
            frame.id.0 += 1;
            frame.timestamp.0 += 1;
            assert!(!matches!(detector.evaluate(&frame).verdict, Verdict::Error));
        }
    }
    Ok(())
}
