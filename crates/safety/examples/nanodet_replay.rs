//! Offline model evaluation only. No output device or clearance policy.
use camera::{Frame, FrameSource, replay::ImageSequence};
use fly_core::Confidence;
use safety::{
    EvidenceScope, SafetyDetector,
    nanodet::{NanoDetConfig, NanoDetDetector},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 5,
        "Usage: nanodet_replay PYTHON WORKER PARAM WEIGHTS SEQUENCE_JSON"
    );
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let mut source = ImageSequence::open(Path::new(&args[4]))?;
    let mut frame = Frame::new(source.size());
    let mut detector = NanoDetDetector::start(NanoDetConfig {
        python: PathBuf::from(&args[0]),
        worker: PathBuf::from(&args[1]),
        param: PathBuf::from(&args[2]),
        weights: PathBuf::from(&args[3]),
        hazard_threshold: Confidence::new(0.4)?,
        deadline: Duration::from_secs(10),
        threads: 2,
        scope: EvidenceScope::ReplayFixture,
    })?;
    let mut latencies = Vec::new();
    let mut cold_start_us = None;
    let mut truth_samples = 0;
    let mut missed = 0;
    let mut class_mismatches = 0;
    let mut faults = 0;
    while source.next_into(&mut frame)? {
        let evidence = detector.evaluate(&frame);
        let elapsed_us = detector.last_elapsed.as_micros();
        if cold_start_us.is_none() {
            cold_start_us = Some(elapsed_us);
        } else {
            latencies.push(elapsed_us);
        }
        if let Some(error) = &detector.last_error {
            faults += 1;
            tracing::error!(frame = frame.id.0, %error, "Detector fault");
        }
        if let Some(hazard) = frame.truth.as_ref().and_then(|truth| truth.hazard) {
            truth_samples += 1;
            match evidence.verdict {
                safety::Verdict::Hazard(found) if found != hazard => class_mismatches += 1,
                safety::Verdict::Hazard(_) => {}
                _ => missed += 1,
            }
        }
        tracing::info!(frame = frame.id.0, ?evidence.verdict, ?detector.last_scores, elapsed_us, "Safety inference; no output connected");
        if detector.last_error.is_some() {
            break;
        }
    }
    latencies.sort_unstable();
    let percentile = |percent: usize| -> Option<u128> {
        if latencies.is_empty() {
            None
        } else {
            latencies
                .get((latencies.len() * percent).div_ceil(100).saturating_sub(1))
                .copied()
        }
    };
    tracing::info!(?cold_start_us, warm_samples = latencies.len(), p50_us = ?percentile(50), p95_us = ?percentile(95), p99_us = ?percentile(99), max_us = ?latencies.last(), truth_samples, missed, class_mismatches, faults, "Offline evaluation summary; timings include IPC");
    anyhow::ensure!(faults == 0, "Detector evaluation stopped after a fault");
    Ok(())
}
