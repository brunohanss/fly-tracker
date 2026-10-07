use aiming::calibration::Calibration;
use app::pipeline::ReplayPipeline;
use camera::{Frame, FrameSource, Hazard};
use fly_core::{
    Confidence, FrameId, FrameSize, FrameTimestamp, LockoutReason, SafetyState, TrackLifecycle,
    config::{Config, SafetyBackend},
};
use safety::{Evidence, EvidenceScope, SafetyDetector, Verdict};

struct StillFrames {
    size: FrameSize,
    next: u64,
}
impl FrameSource for StillFrames {
    fn size(&self) -> FrameSize {
        self.size
    }
    fn next_into(&mut self, output: &mut Frame) -> Result<bool, camera::CameraError> {
        if self.next == 15 {
            return Ok(false);
        }
        output.pixels.fill(160);
        for (x, y) in [(12, 12), (30, 12), (50, 20), (70, 40), (85, 52)] {
            for row in y - 1..=y + 1 {
                for col in x - 1..=x + 1 {
                    output.pixels[row * 96 + col] = 20;
                }
            }
        }
        output.id = FrameId(self.next);
        output.timestamp = FrameTimestamp(self.next * 10_000);
        self.next += 1;
        Ok(true)
    }
    fn reset(&mut self) -> Result<(), camera::CameraError> {
        self.next = 0;
        Ok(())
    }
}
struct Controlled {
    verdict: Verdict,
    identity_error: u8,
    scope: EvidenceScope,
}
impl SafetyDetector for Controlled {
    fn replay_scope(&self) -> EvidenceScope {
        EvidenceScope::ReplayImage
    }
    fn evaluate(&mut self, frame: &Frame) -> Evidence {
        // Assert that inference receives the full image, outside the insect region too.
        assert_eq!(frame.size.width(), 96);
        assert_eq!(frame.pixels.len(), 96 * 64);
        Evidence {
            frame: FrameId(frame.id.0 + u64::from(self.identity_error == 1)),
            timestamp: FrameTimestamp(frame.timestamp.0 + u64::from(self.identity_error == 2)),
            scope: self.scope,
            verdict: if self.identity_error == 3 && frame.id.0 > 0 {
                Verdict::Unavailable
            } else {
                self.verdict
            },
        }
    }
}
fn config() -> anyhow::Result<Config> {
    Ok(Config {
        frame_size: FrameSize::new(96, 64)?,
        max_frame_age_us: 1_000_000,
        max_safety_age_us: 1_000_000,
        ..Config::default()
    })
}
fn pipeline(
    mut config: Config,
    detector: Box<dyn SafetyDetector>,
) -> anyhow::Result<ReplayPipeline> {
    config.processing.virtual_aim_delay_us = 0;
    let size = config.frame_size;
    ReplayPipeline::new(
        Box::new(StillFrames { size, next: 0 }),
        detector,
        config,
        "controlled-image-safety".into(),
        Some(Calibration::virtual_plane(size)?),
    )
}
fn detector(verdict: Verdict, identity_error: u8, scope: EvidenceScope) -> Box<dyn SafetyDetector> {
    Box::new(Controlled {
        verdict,
        identity_error,
        scope,
    })
}

#[test]
fn clear_evidence_cycles_current_targets_and_dwell_is_deterministic() -> anyhow::Result<()> {
    for dwell in [0, 20_000] {
        let mut config = config()?;
        config.processing.virtual_aim_dwell_us = dwell;
        let mut run = pipeline(
            config,
            detector(
                Verdict::Clear(Confidence::new(1.0)?),
                0,
                EvidenceScope::ReplayImage,
            ),
        )?;
        let mut ids = Vec::new();
        while run.step()? {
            let s = run.snapshot();
            if let Some(aim) = s.aim {
                assert!(aim.issued);
                assert!(
                    s.predictions
                        .contains(&(aim.request.target, aim.request.predicted_pixel))
                );
                ids.push(aim.request.target);
            }
        }
        assert_eq!(ids.len(), 13);
        let cycle = if dwell == 0 {
            ids[..5].to_vec()
        } else {
            ids[..10].iter().step_by(2).copied().collect()
        };
        assert_eq!(
            cycle
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            5
        );
        for (index, id) in ids.iter().enumerate() {
            assert_eq!(*id, cycle[(index / if dwell == 0 { 1 } else { 2 }) % 5]);
        }
        run.seek(14)?;
        assert_eq!(
            run.snapshot().aim.map(|aim| aim.request.target),
            ids.last().copied()
        );
    }
    Ok(())
}

#[test]
fn every_failed_or_hazard_frame_suppresses_output_while_tracking_continues() -> anyhow::Result<()> {
    let low = Confidence::new(0.1)?;
    for verdict in [
        Verdict::Hazard(Hazard::Human),
        Verdict::Hazard(Hazard::Dog),
        Verdict::Hazard(Hazard::Cat),
        Verdict::Uncertain,
        Verdict::Clear(low),
        Verdict::Unavailable,
        Verdict::Error,
        Verdict::Timeout,
        Verdict::Stale,
    ] {
        let mut run = pipeline(config()?, detector(verdict, 0, EvidenceScope::ReplayImage))?;
        while run.step()? {
            let s = run.snapshot();
            assert!(matches!(s.safety, SafetyState::SafetyLockout(_)));
            assert!(s.aim.is_none_or(|aim| !aim.issued));
            if s.frame.0 >= 2 {
                assert_eq!(
                    s.tracks
                        .iter()
                        .filter(|t| t.lifecycle == TrackLifecycle::Confirmed)
                        .count(),
                    5
                );
            }
        }
        assert_eq!(run.counters().issued_aims, 0);
        assert_eq!(run.counters().suppressed_aims, 13);
    }
    Ok(())
}

#[test]
fn identity_scope_and_previous_clear_results_cannot_authorize_output() -> anyhow::Result<()> {
    for (identity, scope) in [
        (1, EvidenceScope::ReplayImage),
        (2, EvidenceScope::ReplayImage),
        (3, EvidenceScope::ReplayImage),
        (0, EvidenceScope::Live),
        (0, EvidenceScope::ReplayFixture),
    ] {
        let mut run = pipeline(
            config()?,
            detector(Verdict::Clear(Confidence::new(1.0)?), identity, scope),
        )?;
        while run.step()? {
            assert!(run.snapshot().aim.is_none_or(|aim| !aim.issued));
        }
        assert_eq!(run.counters().issued_aims, 0);
    }
    Ok(())
}

#[test]
fn missing_runtime_or_models_remains_unavailable() -> anyhow::Result<()> {
    let mut config = config()?;
    config.safety_backend = SafetyBackend::Nanodet {
        python: "missing-python.exe".into(),
        worker: "missing-worker.py".into(),
        param: "missing.param".into(),
        weights: "missing.bin".into(),
        hazard_threshold: Confidence::new(0.4)?,
        timeout_us: 1_000_000,
        threads: 2,
    };
    config.validate()?;
    let backend = app::safety_backend::build(&config.safety_backend);
    let mut run = pipeline(config, backend)?;
    while run.step()? {
        let s = run.snapshot();
        assert_eq!(
            s.safety,
            SafetyState::SafetyLockout(LockoutReason::DetectorUnavailable)
        );
        assert!(s.safety_backend.contains("initialization failed"));
    }
    assert_eq!(run.counters().issued_aims, 0);
    Ok(())
}

#[test]
fn slow_clear_inference_expires_even_when_replay_source_time_is_fixed() -> anyhow::Result<()> {
    struct DelayedClear;
    impl SafetyDetector for DelayedClear {
        fn replay_scope(&self) -> EvidenceScope {
            EvidenceScope::ReplayImage
        }
        fn evaluate(&mut self, frame: &Frame) -> Evidence {
            std::thread::sleep(std::time::Duration::from_millis(5));
            Evidence {
                frame: frame.id,
                timestamp: frame.timestamp,
                scope: EvidenceScope::ReplayImage,
                verdict: Verdict::Clear(Confidence::new(1.0).unwrap_or_else(|_| unreachable!())),
            }
        }
    }
    for reason in [LockoutReason::StaleSafety, LockoutReason::StaleCamera] {
        let mut config = config()?;
        if reason == LockoutReason::StaleSafety {
            config.max_safety_age_us = 1000;
        } else {
            config.max_frame_age_us = 1000;
        }
        let mut run = pipeline(config, Box::new(DelayedClear))?;
        while run.step()? {
            assert_eq!(run.snapshot().safety, SafetyState::SafetyLockout(reason));
        }
        assert_eq!(run.counters().issued_aims, 0);
    }
    Ok(())
}
