//! Native ncnn worker adapter. Detection alone never grants clearance.
use crate::{Evidence, EvidenceScope, SafetyDetector, Verdict};
use camera::{Frame, Hazard};
use fly_core::Confidence;
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct NanoDetConfig {
    pub python: PathBuf,
    pub worker: PathBuf,
    pub param: PathBuf,
    pub weights: PathBuf,
    pub hazard_threshold: Confidence,
    /// Includes pipe transfer and inference. First request also includes model loading.
    pub deadline: Duration,
    pub threads: u8,
    pub scope: EvidenceScope,
}

#[derive(Debug, Error)]
pub enum NanoDetError {
    #[error("Invalid NanoDet configuration")]
    Config,
    #[error("Native worker I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Native worker returned invalid identity, scores, or protocol")]
    Protocol,
    #[error("Native worker exceeded its deadline")]
    Timeout,
    #[error("Native worker is stopped")]
    Stopped,
}

/// Scores are presence scores, not probabilities that the region is clear.
#[derive(Debug, Clone, Copy)]
pub struct HazardScores {
    pub human: Confidence,
    pub dog: Confidence,
    pub cat: Confidence,
}
impl HazardScores {
    fn verdict(self, threshold: Confidence) -> Verdict {
        // Stable priority when more than one protected class is present.
        for (score, hazard) in [
            (self.human, Hazard::Human),
            (self.dog, Hazard::Dog),
            (self.cat, Hazard::Cat),
        ] {
            if score.get() >= threshold.get() {
                return Verdict::Hazard(hazard);
            }
        }
        Verdict::Uncertain
    }
}

struct Request {
    header: [u8; 28],
    pixels: Vec<u8>,
}
type WorkerResult = Result<[u8; 32], std::io::Error>;

pub struct NanoDetDetector {
    config: NanoDetConfig,
    child: Child,
    requests: Option<SyncSender<Request>>,
    responses: Receiver<WorkerResult>,
    worker_thread: Option<JoinHandle<()>>,
    pub last_scores: Option<HazardScores>,
    pub last_elapsed: Duration,
    pub last_error: Option<NanoDetError>,
    stopped: bool,
}
impl NanoDetDetector {
    pub fn start(config: NanoDetConfig) -> Result<Self, NanoDetError> {
        if config.deadline.is_zero()
            || config.deadline > Duration::from_secs(60)
            || config.hazard_threshold.get() <= 0.0
            || !(1..=4).contains(&config.threads)
        {
            return Err(NanoDetError::Config);
        }
        let mut child = Command::new(&config.python)
            .arg("-u")
            .arg(&config.worker)
            .arg("--param")
            .arg(&config.param)
            .arg("--weights")
            .arg(&config.weights)
            .arg("--threads")
            .arg(config.threads.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Never allow stderr to fill an unread pipe or corrupt protocol stdout.
            .stderr(Stdio::inherit())
            .spawn()?;
        let Some(mut input) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(NanoDetError::Protocol);
        };
        let Some(mut output) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(NanoDetError::Protocol);
        };
        let (requests, incoming) = mpsc::sync_channel::<Request>(1);
        let (outgoing, responses) = mpsc::sync_channel(1);
        let worker_thread =
            match thread::Builder::new()
                .name("nanodet-pipes".into())
                .spawn(move || {
                    while let Ok(request) = incoming.recv() {
                        let result = (|| {
                            input.write_all(&request.header)?;
                            input.write_all(&request.pixels)?;
                            input.flush()?;
                            let mut response = [0; 32];
                            output.read_exact(&mut response)?;
                            Ok(response)
                        })();
                        let failed = result.is_err();
                        if outgoing.send(result).is_err() || failed {
                            break;
                        }
                    }
                }) {
                Ok(handle) => handle,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.into());
                }
            };
        Ok(Self {
            config,
            child,
            requests: Some(requests),
            responses,
            worker_thread: Some(worker_thread),
            last_scores: None,
            last_elapsed: Duration::ZERO,
            last_error: None,
            stopped: false,
        })
    }
    fn stop(&mut self) {
        self.stopped = true;
        self.requests.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.worker_thread.take() {
            let _ = handle.join();
        }
    }
    fn infer(&mut self, frame: &Frame) -> Result<HazardScores, NanoDetError> {
        let started = Instant::now();
        if self.stopped {
            return Err(NanoDetError::Stopped);
        }
        // Ground-truth labels are neither consumed nor validated by this image backend.
        if frame.pixels.len() != frame.size.pixels() {
            return Err(NanoDetError::Protocol);
        }
        let mut header = [0; 28];
        header[..4].copy_from_slice(b"ND01");
        header[4..8].copy_from_slice(&frame.size.width().to_le_bytes());
        header[8..12].copy_from_slice(&frame.size.height().to_le_bytes());
        header[12..20].copy_from_slice(&frame.id.0.to_le_bytes());
        header[20..28].copy_from_slice(&frame.timestamp.0.to_le_bytes());
        self.requests
            .as_ref()
            .ok_or(NanoDetError::Stopped)?
            .try_send(Request {
                header,
                pixels: frame.pixels.clone(),
            })
            .map_err(|_| NanoDetError::Stopped)?;
        let response = self
            .responses
            .recv_timeout(
                self.config
                    .deadline
                    .checked_sub(started.elapsed())
                    .ok_or(NanoDetError::Timeout)?,
            )
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => NanoDetError::Timeout,
                mpsc::RecvTimeoutError::Disconnected => NanoDetError::Stopped,
            })??;
        if started.elapsed() > self.config.deadline {
            return Err(NanoDetError::Timeout);
        }
        decode(&response, frame)
    }
}
fn decode(response: &[u8; 32], frame: &Frame) -> Result<HazardScores, NanoDetError> {
    if &response[..4] != b"ND01"
        || response[4..12] != frame.id.0.to_le_bytes()
        || response[12..20] != frame.timestamp.0.to_le_bytes()
    {
        return Err(NanoDetError::Protocol);
    }
    let score = |offset| {
        let bytes = [
            response[offset],
            response[offset + 1],
            response[offset + 2],
            response[offset + 3],
        ];
        Confidence::new(f64::from(f32::from_le_bytes(bytes))).map_err(|_| NanoDetError::Protocol)
    };
    Ok(HazardScores {
        human: score(20)?,
        dog: score(24)?,
        cat: score(28)?,
    })
}
impl SafetyDetector for NanoDetDetector {
    fn replay_scope(&self) -> EvidenceScope {
        // Live evidence must not be relabelled to authorize replay output.
        if self.config.scope == EvidenceScope::ReplayImage {
            EvidenceScope::ReplayImage
        } else {
            EvidenceScope::ReplayFixture
        }
    }
    fn diagnostics(&self) -> String {
        format!(
            "NanoDet-Plus / native ncnn: {} | human/dog/cat: {:?} | inference age: {}us | error: {:?}",
            if self.last_error.is_some() {
                "error or timeout"
            } else if self.last_scores.is_some() {
                "ready; clearance unvalidated"
            } else {
                "initializing"
            },
            self.last_scores,
            self.last_elapsed.as_micros(),
            self.last_error
        )
    }
    fn evaluate(&mut self, frame: &Frame) -> Evidence {
        if self.stopped {
            return Evidence {
                frame: frame.id,
                timestamp: frame.timestamp,
                scope: self.config.scope,
                verdict: if matches!(self.last_error, Some(NanoDetError::Timeout)) {
                    Verdict::Timeout
                } else {
                    Verdict::Error
                },
            };
        }
        let start = Instant::now();
        self.last_scores = None;
        self.last_error = None;
        let verdict = match self.infer(frame) {
            Ok(scores) => {
                self.last_scores = Some(scores);
                scores.verdict(self.config.hazard_threshold)
            }
            Err(error) => {
                let verdict = if matches!(error, NanoDetError::Timeout) {
                    Verdict::Timeout
                } else {
                    Verdict::Error
                };
                self.last_error = Some(error);
                // Late responses cannot be reused for a subsequent frame.
                self.stop();
                verdict
            }
        };
        self.last_elapsed = start.elapsed();
        Evidence {
            frame: frame.id,
            timestamp: frame.timestamp,
            scope: self.config.scope,
            verdict,
        }
    }
}
impl Drop for NanoDetDetector {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scores_cannot_grant_clearance() -> Result<(), Box<dyn std::error::Error>> {
        let threshold = Confidence::new(0.4)?;
        for (values, hazard) in [
            ([0.5, 0.1, 0.1], Hazard::Human),
            ([0.1, 0.5, 0.1], Hazard::Dog),
            ([0.1, 0.1, 0.5], Hazard::Cat),
        ] {
            let scores = HazardScores {
                human: Confidence::new(values[0])?,
                dog: Confidence::new(values[1])?,
                cat: Confidence::new(values[2])?,
            };
            assert!(matches!(scores.verdict(threshold), Verdict::Hazard(found) if found == hazard));
        }
        let zero = Confidence::new(0.0)?;
        assert!(matches!(
            HazardScores {
                human: zero,
                dog: zero,
                cat: zero
            }
            .verdict(threshold),
            Verdict::Uncertain
        ));
        Ok(())
    }
    #[test]
    fn identity_and_invalid_scores_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let frame = Frame::new(fly_core::FrameSize::new(8, 8)?);
        let mut reply = [0; 32];
        reply[..4].copy_from_slice(b"ND01");
        assert!(decode(&reply, &frame).is_ok());
        for score in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            reply[20..24].copy_from_slice(&score.to_le_bytes());
            assert!(decode(&reply, &frame).is_err());
        }
        reply[20..24].copy_from_slice(&0_f32.to_le_bytes());
        reply[4] = 1;
        assert!(decode(&reply, &frame).is_err());
        reply[4] = 0;
        reply[12] = 1;
        assert!(decode(&reply, &frame).is_err());
        Ok(())
    }
}
