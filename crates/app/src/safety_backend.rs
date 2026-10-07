//! Replay-only detector composition. Initialization faults stay visible.
use camera::Frame;
use fly_core::config::SafetyBackend;
use safety::{Evidence, EvidenceScope, SafetyDetector, UnavailableDetector};
use std::time::Duration;

struct ReplayNanoDet {
    detector: safety::nanodet::NanoDetDetector,
    ready: bool,
    initialization_failed: bool,
}
impl SafetyDetector for ReplayNanoDet {
    fn replay_scope(&self) -> EvidenceScope {
        EvidenceScope::ReplayImage
    }
    fn diagnostics(&self) -> String {
        if self.initialization_failed {
            format!(
                "NanoDet unavailable: initialization failed | {}",
                self.detector.diagnostics()
            )
        } else {
            self.detector.diagnostics()
        }
    }
    fn evaluate(&mut self, frame: &Frame) -> Evidence {
        if self.initialization_failed {
            return UnavailableDetector.evaluate(frame);
        }
        let mut evidence = self.detector.evaluate(frame);
        if !self.ready && matches!(evidence.verdict, safety::Verdict::Error) {
            self.initialization_failed = true;
            evidence.verdict = safety::Verdict::Unavailable;
        }
        self.ready |= self.detector.last_scores.is_some();
        evidence
    }
}

struct InitializationFailed(String);
impl SafetyDetector for InitializationFailed {
    fn evaluate(&mut self, frame: &Frame) -> Evidence {
        UnavailableDetector.evaluate(frame)
    }
    fn diagnostics(&self) -> String {
        format!("NanoDet unavailable: initialization failed: {}", self.0)
    }
}

pub fn build(backend: &SafetyBackend) -> Box<dyn SafetyDetector> {
    match backend {
        SafetyBackend::Unavailable => Box::new(UnavailableDetector),
        SafetyBackend::Fixture => Box::new(safety::FixtureDetector),
        SafetyBackend::Nanodet {
            python,
            worker,
            param,
            weights,
            hazard_threshold,
            timeout_us,
            threads,
        } => {
            // Missing dependencies/models must leave a running, locked-out dashboard.
            for path in [python, worker, param, weights] {
                if !path.is_file() {
                    return Box::new(InitializationFailed(format!(
                        "missing file {}",
                        path.display()
                    )));
                }
            }
            match safety::nanodet::NanoDetDetector::start(safety::nanodet::NanoDetConfig {
                python: python.clone(),
                worker: worker.clone(),
                param: param.clone(),
                weights: weights.clone(),
                hazard_threshold: *hazard_threshold,
                deadline: Duration::from_micros(*timeout_us),
                threads: *threads,
                scope: EvidenceScope::ReplayImage,
            }) {
                Ok(detector) => Box::new(ReplayNanoDet {
                    detector,
                    ready: false,
                    initialization_failed: false,
                }),
                Err(error) => Box::new(InitializationFailed(error.to_string())),
            }
        }
    }
}
