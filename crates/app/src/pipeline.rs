use aiming::{AimRecord, AimRequest, AimingDevice, VirtualAimer, calibration::Calibration};
use anyhow::{Context, Result};
use camera::{Frame, FrameSource, GroundTruth};
use fly_core::{
    FrameTimestamp, PixelPosition, SafetyState, TargetId, TrackLifecycle, config::Config,
};
use safety::{EvidenceScope, SafetyAuthority, SafetyDetector};
use std::{
    collections::VecDeque,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use telemetry::{
    Counters, DashboardSnapshot, Latencies, Preview, RunIdentity, RunMetrics, Series, Stage,
    fingerprint,
};
use tracking::{Tracker, TrackerConfig, predict};
use vision::{Detector, DetectorConfig};

struct Pending {
    truth_id: TargetId,
    point: PixelPosition,
    due: FrameTimestamp,
    metric: usize,
}
pub struct ReplayPipeline {
    source: Box<dyn FrameSource>,
    detector: Box<dyn SafetyDetector>,
    config: Config,
    frame: Frame,
    vision: Detector,
    tracker: Tracker,
    authority: SafetyAuthority,
    aimer: VirtualAimer,
    calibration: Option<Calibration>,
    counters: Counters,
    latencies: Latencies,
    detection_errors: Series,
    prediction_errors: Vec<Series>,
    aim_errors: Series,
    pending: VecDeque<Pending>,
    pending_scratch: Vec<Pending>,
    previous_truth: Option<GroundTruth>,
    previous_timestamp: Option<FrameTimestamp>,
    truth_matches: Vec<(TargetId, TargetId)>,
    used_detections: Vec<bool>,
    used_tracks: Vec<bool>,
    predictions: Vec<(TargetId, PixelPosition)>,
    identity: RunIdentity,
    last_safety: SafetyState,
    last_aim: Option<AimRecord>,
    aim_status: aiming::AimStatus,
    complete: bool,
    faulted: bool,
    clock: crate::replay_clock::ReplayClock,
    previous_tracks: Vec<(TargetId, TrackLifecycle)>,
    last_issued_target: Option<TargetId>,
    evidence_age_us: u64,
    target_selected_at: FrameTimestamp,
    virtual_delay: aiming::delay::VirtualAimDelay,
}
impl ReplayPipeline {
    pub fn new(
        source: Box<dyn FrameSource>,
        detector: Box<dyn SafetyDetector>,
        config: Config,
        dataset: String,
        calibration: Option<Calibration>,
    ) -> Result<Self> {
        config.validate()?;
        let size = source.size();
        anyhow::ensure!(
            size == config.frame_size,
            "Source dimensions differ from configured frame size"
        );
        if let Some(calibration) = &calibration {
            calibration.validate()?;
            anyhow::ensure!(
                calibration.camera_size == size,
                "Calibration dimensions differ from source"
            );
        }
        let p = &config.processing;
        let vision = Detector::new(
            size,
            DetectorConfig {
                stationary_detection: p.stationary_detection,
                region: p.detection_region,
                threshold: p.detection_threshold,
                min_neighbors: p.morphology_min_neighbors,
                min_area: p.min_area,
                max_area: p.max_area,
                capacity: config.max_detections,
                ..DetectorConfig::default()
            },
        )?;
        let tracker = Tracker::new(
            size,
            TrackerConfig {
                max_tracks: config.max_tracks,
                max_detections: config.max_detections,
                association_radius: p.association_radius,
                confirmation_hits: p.confirmation_hits,
                remove_after_misses: p.remove_after_misses,
                velocity_time_constant_s: p.velocity_time_constant_s,
            },
        )?;
        let identity = RunIdentity {
            dataset,
            config_fingerprint: fingerprint(&serde_json::to_vec(&(
                &config,
                &calibration,
                algorithm_fingerprint(),
            ))?),
            hardware: std::env::var("FLY_TRACKER_HARDWARE_ID")
                .ok()
                .filter(|id| !id.is_empty() && id.len() <= 128)
                .unwrap_or_else(|| {
                    format!(
                        "unverified:{}-{}",
                        std::env::consts::ARCH,
                        std::env::consts::OS
                    )
                }),
            build: format!(
                "{}-{}-{}-{}",
                env!("CARGO_PKG_VERSION"),
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                },
                env!("FLY_TRACKER_COMPILER"),
                env!("FLY_TRACKER_TARGET")
            ),
            metric_version: 1,
            git_commit: None,
        };
        let capacity = config.history_capacity;
        let track_capacity = config.max_tracks;
        let scope = detector.replay_scope();
        let virtual_delay = aiming::delay::VirtualAimDelay::new(Duration::from_micros(
            config.processing.virtual_aim_delay_us,
        ));
        Ok(Self {
            source,
            detector,
            frame: Frame::new(size),
            vision,
            tracker,
            authority: SafetyAuthority::new(&config)?,
            aimer: VirtualAimer::new(scope),
            calibration,
            counters: Counters::default(),
            latencies: Latencies::new(capacity)?,
            detection_errors: Series::new(capacity)?,
            prediction_errors: (0..4)
                .map(|_| Series::new(capacity))
                .collect::<Result<_, _>>()?,
            aim_errors: Series::new(capacity)?,
            pending: VecDeque::with_capacity(capacity),
            pending_scratch: Vec::with_capacity(track_capacity * 3),
            previous_truth: None,
            previous_timestamp: None,
            truth_matches: Vec::with_capacity(track_capacity),
            used_detections: vec![false; config.max_detections],
            used_tracks: vec![false; track_capacity],
            predictions: Vec::with_capacity(track_capacity),
            identity,
            last_safety: SafetyState::SafetyLockout(fly_core::LockoutReason::DetectorUnavailable),
            last_aim: None,
            aim_status: aiming::AimStatus::NoTarget,
            complete: false,
            faulted: false,
            clock: crate::replay_clock::ReplayClock::new(Instant::now()),
            previous_tracks: Vec::with_capacity(track_capacity),
            last_issued_target: None,
            evidence_age_us: 0,
            target_selected_at: FrameTimestamp(0),
            virtual_delay,
            config,
        })
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub fn set_paused(&mut self, paused: bool) {
        self.clock.pause(paused, Instant::now());
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    pub fn next_timestamp(&self) -> Option<FrameTimestamp> {
        self.source.next_timestamp()
    }
    pub fn counters(&self) -> &Counters {
        &self.counters
    }
    pub fn seek(&mut self, frame_index: u64) -> Result<()> {
        anyhow::ensure!(
            frame_index < 1_000_000,
            "Seek exceeds supported replay limit"
        );
        if let Some(total) = self.source.total_frames() {
            anyhow::ensure!(frame_index < total, "Seek is past end of source");
        }
        let previous_count = self.counters.processed_frames;
        let was_complete = self.complete;
        self.reset()?;
        for _ in 0..=frame_index {
            if !self.step()? {
                self.reset()?;
                for _ in 0..previous_count {
                    self.step()?;
                }
                if was_complete {
                    self.step()?;
                }
                anyhow::bail!("Seek is past end of source");
            }
        }
        Ok(())
    }
    pub fn reset(&mut self) -> Result<()> {
        self.source.reset()?;
        self.vision.reset();
        self.tracker.reset();
        self.authority = SafetyAuthority::new(&self.config)?;
        self.aimer.stop();
        self.counters = Counters::default();
        self.latencies = Latencies::new(self.config.history_capacity)?;
        self.detection_errors = Series::new(self.config.history_capacity)?;
        self.prediction_errors = (0..4)
            .map(|_| Series::new(self.config.history_capacity))
            .collect::<Result<_, _>>()?;
        self.aim_errors = Series::new(self.config.history_capacity)?;
        self.pending.clear();
        self.previous_truth = None;
        self.previous_timestamp = None;
        self.truth_matches.clear();
        self.predictions.clear();
        self.last_aim = None;
        self.aim_status = aiming::AimStatus::NoTarget;
        self.complete = false;
        self.faulted = false;
        self.clock = crate::replay_clock::ReplayClock::new(Instant::now());
        self.previous_tracks.clear();
        self.last_safety = SafetyState::SafetyLockout(fly_core::LockoutReason::DetectorUnavailable);
        self.last_issued_target = None;
        self.evidence_age_us = 0;
        self.target_selected_at = FrameTimestamp(0);
        self.virtual_delay.cancel();
        Ok(())
    }
    pub fn shutdown(&mut self) {
        self.virtual_delay.cancel();
        self.authority.shutdown();
        self.aimer.stop();
    }
    pub fn step(&mut self) -> Result<bool> {
        anyhow::ensure!(!self.faulted, "Pipeline is faulted; reset is required");
        let result = self.step_inner();
        if result.is_err() {
            self.faulted = true;
            self.shutdown();
            self.last_aim = None;
            self.last_safety = SafetyState::SafetyLockout(fly_core::LockoutReason::Shutdown);
            self.aim_status = aiming::AimStatus::Suppressed(fly_core::LockoutReason::Shutdown);
        }
        result
    }
    fn step_inner(&mut self) -> Result<bool> {
        if self.complete {
            return Ok(false);
        }
        let start = Instant::now();
        if !self
            .source
            .next_into(&mut self.frame)
            .context("Frame acquisition failed")?
        {
            self.complete = true;
            self.virtual_delay.cancel();
            self.last_aim = None;
            self.aim_status = aiming::AimStatus::NoTarget;
            self.aimer.stop();
            return Ok(false);
        }
        self.latencies.set_timestamp(self.frame.timestamp);
        self.latencies.record(Stage::Capture, start.elapsed())?;
        self.frame.validate()?;
        let stage = Instant::now();
        self.authority
            .record_camera(self.frame.id, self.frame.timestamp);
        let mut evidence = self.detector.evaluate(&self.frame);
        if evidence.scope != self.detector.replay_scope()
            && matches!(evidence.verdict, safety::Verdict::Clear(_))
        {
            evidence.verdict = safety::Verdict::Invalid;
        }
        self.evidence_age_us = stage.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        // Source time does not advance during synchronous replay inference.
        // Check real inference age separately. Do not replace errors or hazards.
        if self.detector.replay_scope() == EvidenceScope::ReplayImage
            && self.evidence_age_us > self.config.max_safety_age_us
            && matches!(
                evidence.verdict,
                safety::Verdict::Clear(_) | safety::Verdict::Uncertain
            )
        {
            evidence.verdict = safety::Verdict::Stale;
        }
        self.authority.observe(evidence);
        if self.detector.replay_scope() == EvidenceScope::ReplayImage {
            self.authority.record_processing_age(start.elapsed());
        }
        let safety = self.authority.state_at(self.frame.timestamp);
        if matches!(safety, SafetyState::SafetyLockout(_)) {
            self.counters.lockout_frames += 1;
            if !matches!(self.last_safety, SafetyState::SafetyLockout(_))
                || self.counters.processed_frames == 0
            {
                self.counters.lockouts += 1;
            }
        }
        self.last_safety = safety;
        self.latencies.record(Stage::Safety, stage.elapsed())?;
        let stage = Instant::now();
        let detections = self.vision.detect(&self.frame)?;
        self.counters.detections += detections.len() as u64;
        self.latencies.record(Stage::Detection, stage.elapsed())?;
        self.previous_tracks.clear();
        self.previous_tracks.extend(
            self.tracker
                .tracks()
                .iter()
                .map(|track| (track.id, track.lifecycle)),
        );
        let stage = Instant::now();
        self.tracker.update(detections, self.frame.timestamp)?;
        self.latencies.record(Stage::Tracking, stage.elapsed())?;
        if self.vision.truncated() {
            self.counters.detection_capacity_frames += 1;
        }
        for track in self.tracker.tracks() {
            let previous = self
                .previous_tracks
                .iter()
                .find(|(id, _)| *id == track.id)
                .map(|(_, state)| *state);
            if previous.is_none() {
                self.counters.acquired_targets += 1;
            }
            if previous == Some(TrackLifecycle::TemporarilyLost) && track.lost_count == 0 {
                self.counters.reacquisitions += 1;
            }
            if track.lifecycle == TrackLifecycle::TemporarilyLost
                && previous != Some(TrackLifecycle::TemporarilyLost)
            {
                self.counters.lost_targets += 1;
            }
        }
        self.evaluate_truth()?;
        let stage = Instant::now();
        self.predictions.clear();
        let at = self.frame.timestamp.advance(Duration::from_micros(
            self.config.processing.prediction_horizon_us,
        ))?;
        for track in self
            .tracker
            .tracks()
            .iter()
            .filter(|track| track.lifecycle == TrackLifecycle::Confirmed)
        {
            if let Some(point) = predict(track, at, self.frame.size, 20_000)? {
                self.predictions.push((track.id, point));
            }
        }
        self.latencies.record(Stage::Prediction, stage.elapsed())?;
        self.last_aim = None;
        let stage = Instant::now();
        self.aim_status = if self.calibration.is_none() {
            aiming::AimStatus::NoCalibration
        } else {
            aiming::AimStatus::NoTarget
        };
        // Cycle through current confirmed IDs. Never queue predicted coordinates.
        // Advance only after the virtual command is accepted by the interlock.
        if self.detector.replay_scope() == EvidenceScope::ReplayImage {
            self.authority.record_processing_age(start.elapsed());
        }
        let current_safety = self.authority.state_at(self.frame.timestamp);
        if matches!(current_safety, SafetyState::SafetyLockout(_)) {
            self.virtual_delay.cancel();
            self.aimer.stop();
        }
        let pending_target = self.virtual_delay.target();
        let pending_point = self
            .predictions
            .iter()
            .find(|(id, _)| Some(*id) == pending_target);
        if pending_target.is_some() && pending_point.is_none() {
            self.virtual_delay.cancel();
        }
        let held = self.predictions.iter().find(|(id, _)| {
            Some(*id) == self.last_issued_target
                && self
                    .frame
                    .timestamp
                    .0
                    .saturating_sub(self.target_selected_at.0)
                    < self.config.processing.virtual_aim_dwell_us
        });
        let selected = pending_point.or(held).or_else(|| {
            self.predictions
                .iter()
                .filter(|(id, _)| self.last_issued_target.is_none_or(|last| *id > last))
                .min_by_key(|(id, _)| *id)
                .or_else(|| self.predictions.iter().min_by_key(|(id, _)| *id))
        });
        if let (Some(calibration), Some((target, pixel))) = (&self.calibration, selected) {
            // A mapping failure suppresses this request; no uncalibrated fallback.
            if let Ok(aim) = calibration.map(*pixel) {
                let request = AimRequest {
                    frame: self.frame.id,
                    target: *target,
                    predicted_pixel: *pixel,
                    aim,
                };
                if self.detector.replay_scope() == EvidenceScope::ReplayImage {
                    self.authority.record_processing_age(start.elapsed());
                }
                let remaining_us = if current_safety == SafetyState::Clear {
                    if self.virtual_delay.target() != Some(*target)
                        && self.last_issued_target != Some(*target)
                    {
                        self.target_selected_at = self.frame.timestamp;
                    }
                    self.virtual_delay
                        .remaining_us(*target, self.frame.timestamp)?
                } else {
                    0
                };
                if remaining_us > 0 {
                    self.aim_status = aiming::AimStatus::WaitingVirtual {
                        target: *target,
                        remaining_us,
                    };
                } else {
                    let record =
                        self.aimer
                            .aim(&mut self.authority, self.frame.timestamp, request)?;
                    self.last_aim = Some(record);
                    self.virtual_delay.cancel();
                    self.aim_status = record.suppressed_by.map_or(
                        aiming::AimStatus::IssuedVirtual,
                        aiming::AimStatus::Suppressed,
                    );
                    if record.issued {
                        if self.config.processing.virtual_aim_delay_us == 0
                            && self.last_issued_target != Some(*target)
                        {
                            self.target_selected_at = self.frame.timestamp;
                        }
                        self.last_issued_target = Some(*target);
                        self.counters.issued_aims += 1;
                        self.latencies
                            .record(Stage::FrameToCommand, start.elapsed())?;
                        if let Some(truth_id) = self
                            .truth_matches
                            .iter()
                            .find(|(_, id)| id == target)
                            .map(|(id, _)| *id)
                        {
                            self.enqueue(Pending {
                                truth_id,
                                point: *pixel,
                                due: at,
                                metric: 4,
                            });
                        }
                    } else {
                        self.counters.suppressed_aims += 1;
                    }
                }
            } else {
                self.virtual_delay.cancel();
                self.counters.calibration_rejections += 1;
                self.aim_status = aiming::AimStatus::RejectedCalibration;
            }
        } else {
            self.virtual_delay.cancel();
        }
        self.latencies.record(Stage::Aiming, stage.elapsed())?;
        // Replay uses source time. Recorded clearance never grants live permission.
        if self.detector.replay_scope() == EvidenceScope::ReplayImage {
            self.authority.record_processing_age(start.elapsed());
        }
        self.evidence_age_us = start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        self.last_safety = self.authority.state_at(self.frame.timestamp);
        if self
            .frame
            .truth
            .as_ref()
            .is_some_and(|truth| truth.hazard.is_some())
        {
            self.counters.safety_positive_frames += 1;
            if self.last_aim.is_some_and(|record| record.issued) {
                self.counters.unsafe_commands += 1;
            }
        }
        self.previous_truth.clone_from(&self.frame.truth);
        self.previous_timestamp = Some(self.frame.timestamp);
        self.counters.processed_frames += 1;
        self.latencies
            .record(Stage::FrameProcessing, start.elapsed())?;
        self.clock.record_progress(Instant::now());
        Ok(true)
    }
    fn enqueue(&mut self, value: Pending) {
        if self.pending.len() == self.config.history_capacity {
            self.pending.pop_front();
            self.counters.prediction_samples_dropped += 1;
        }
        self.pending.push_back(value);
    }
    fn evaluate_truth(&mut self) -> Result<()> {
        // Evaluate due predictions against truth at their timestamp, not today's position.
        let now = self.frame.timestamp;
        let count = self.pending.len();
        for _ in 0..count {
            let Some(pending) = self.pending.pop_front() else {
                break;
            };
            if pending.due > now {
                self.pending.push_back(pending);
                continue;
            }
            let current = self
                .frame
                .truth
                .as_ref()
                .and_then(|truth| {
                    truth
                        .targets
                        .iter()
                        .find(|target| target.id == pending.truth_id)
                })
                .map(|target| target.position);
            let previous = self
                .previous_truth
                .as_ref()
                .and_then(|truth| {
                    truth
                        .targets
                        .iter()
                        .find(|target| target.id == pending.truth_id)
                })
                .map(|target| target.position);
            let point = if pending.due == now {
                current
            } else {
                match (previous, current, self.previous_timestamp) {
                    (Some(a), Some(b), Some(t)) if pending.due >= t && now > t => {
                        let fraction = (pending.due.0 - t.0) as f64 / (now.0 - t.0) as f64;
                        Some(fly_core::ScenePosition::new(
                            a.x() + (b.x() - a.x()) * fraction,
                            a.y() + (b.y() - a.y()) * fraction,
                        )?)
                    }
                    _ => None,
                }
            };
            if let Some(point) = point {
                let error = (pending.point.x() - point.x()).hypot(pending.point.y() - point.y());
                if pending.metric == 4 {
                    self.aim_errors.push(error)?;
                } else {
                    self.prediction_errors[pending.metric].push(error)?;
                }
            }
        }
        let Some(truth) = &self.frame.truth else {
            return Ok(());
        };
        self.counters.truth_frames += 1;
        self.counters.false_positives.get_or_insert(0);
        self.counters.false_negatives.get_or_insert(0);
        self.counters.truth_target_observations +=
            truth.targets.iter().filter(|target| target.visible).count() as u64;
        self.used_detections.fill(false);
        self.used_tracks.fill(false);
        let detections = self.vision.detected();
        // Ground-truth IDs are used only for measurements, never for detection/tracking.
        self.pending_scratch.clear();
        for target in truth.targets.iter().filter(|target| target.visible) {
            let detection = detections
                .iter()
                .enumerate()
                .filter(|(index, _)| !self.used_detections[*index])
                .map(|(index, d)| {
                    (
                        index,
                        (d.position.x() - target.position.x())
                            .hypot(d.position.y() - target.position.y()),
                    )
                })
                .filter(|(_, error)| *error <= 5.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((index, error)) = detection {
                self.used_detections[index] = true;
                self.detection_errors.push(error)?;
            } else {
                *self.counters.false_negatives.get_or_insert(0) += 1;
            }
            let matched = self
                .tracker
                .tracks()
                .iter()
                .enumerate()
                .filter(|(index, track)| {
                    !self.used_tracks[*index] && track.lifecycle == TrackLifecycle::Confirmed
                })
                .map(|(index, track)| {
                    (
                        index,
                        (track.position.x() - target.position.x())
                            .hypot(track.position.y() - target.position.y()),
                    )
                })
                .filter(|(_, error)| *error <= 20.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((index, error)) = matched {
                self.used_tracks[index] = true;
                self.counters.tracked_truth_observations += 1;
                let track = &self.tracker.tracks()[index];
                self.prediction_errors[0].push(error)?;
                if let Some((_, id)) = self
                    .truth_matches
                    .iter_mut()
                    .find(|(id, _)| *id == target.id)
                {
                    if *id != track.id {
                        self.counters.id_switches += 1;
                        *id = track.id;
                    }
                } else if self.truth_matches.len() < self.config.max_tracks {
                    self.truth_matches.push((target.id, track.id));
                }
                for (metric, horizon) in [(1, 5000), (2, 10000), (3, 20000)] {
                    let due = now.advance(Duration::from_micros(horizon))?;
                    if let Some(point) = predict(track, due, self.frame.size, 20_000)? {
                        self.pending_scratch.push(Pending {
                            truth_id: target.id,
                            point,
                            due,
                            metric,
                        });
                    }
                }
            }
        }
        *self.counters.false_positives.get_or_insert(0) += detections
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.used_detections[*index])
            .count() as u64;
        for prediction in self.pending_scratch.drain(..) {
            if self.pending.len() == self.config.history_capacity {
                self.pending.pop_front();
                self.counters.prediction_samples_dropped += 1;
            }
            self.pending.push_back(prediction);
        }
        Ok(())
    }
    pub fn snapshot(&mut self) -> DashboardSnapshot {
        let width = self.frame.size.width().min(160) as u16;
        let height = self.frame.size.height().min(120) as u16;
        let mut pixels = Vec::with_capacity(usize::from(width) * usize::from(height));
        let mut foreground = Vec::with_capacity(usize::from(width) * usize::from(height));
        for y in 0..u32::from(height) {
            for x in 0..u32::from(width) {
                let row = y * self.frame.size.height() / u32::from(height);
                let column = x * self.frame.size.width() / u32::from(width);
                pixels.push(
                    self.frame.pixels
                        [row as usize * self.frame.size.width() as usize + column as usize],
                );
                foreground.push(
                    self.vision.mask()
                        [row as usize * self.frame.size.width() as usize + column as usize],
                );
            }
        }
        let elapsed = self.clock.elapsed().as_secs_f64();
        DashboardSnapshot {
            publication_time: Instant::now(),
            system: if self.complete {
                fly_core::SystemState::Stopped
            } else {
                fly_core::system_state(
                    self.last_safety,
                    self.tracker
                        .tracks()
                        .iter()
                        .any(|track| track.lifecycle == TrackLifecycle::Confirmed),
                    self.tracker
                        .tracks()
                        .iter()
                        .any(|track| track.lifecycle == TrackLifecycle::TemporarilyLost),
                )
            },
            aim_status: self.aim_status,
            calibration: self.calibration.clone(),
            time_windows: self.latencies.time_windows(),
            identity: self.identity.clone(),
            camera_size: self.frame.size,
            total_frames: self.source.total_frames(),
            paused: false,
            replay_speed: 1.0,
            detection_error_pixels: self.detection_errors.distribution(),
            prediction_error_pixels: self
                .prediction_errors
                .iter_mut()
                .map(Series::distribution)
                .collect(),
            virtual_aim_error_pixels: self.aim_errors.distribution(),
            frame: self.frame.id,
            timestamp: self.frame.timestamp,
            source: self.identity.dataset.clone(),
            recorded_safety: true,
            safety_backend: format!(
                "{} | recorded replay only | result age {}us",
                self.detector.diagnostics(),
                self.evidence_age_us
            ),
            safety: self.last_safety,
            camera_available: !self.complete && !self.faulted,
            tracks: self.tracker.tracks().to_vec(),
            recent_tracks: self.tracker.recent().iter().cloned().collect(),
            predictions: self.predictions.clone(),
            aim: self.last_aim,
            preview: Preview {
                width,
                height,
                pixels,
            },
            foreground_preview: Preview {
                width,
                height,
                pixels: foreground,
            },
            counters: self.counters.clone(),
            latency: self.latencies.distributions(),
            calibration_residuals: None,
            calibration_version: self.calibration.as_ref().map(|c| c.version),
            sample_capacity: self.config.history_capacity,
            complete: self.complete,
            fps: if elapsed > 0.0 {
                Some(self.counters.processed_frames as f64 / elapsed)
            } else {
                None
            },
            elapsed_seconds: elapsed,
        }
    }
    pub fn report(&mut self) -> Result<RunMetrics> {
        anyhow::ensure!(self.complete, "Run report requires completed replay");
        Ok(RunMetrics {
            time_windows: self.latencies.time_windows(),
            version: 1,
            completed_unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            identity: self.identity.clone(),
            sample_capacity: self.config.history_capacity,
            window: "last N samples per metric; exact nearest-rank percentiles".into(),
            latency_unit: "microseconds".into(),
            counters: self.counters.clone(),
            latency: self.latencies.distributions(),
            detection_error_pixels: self.detection_errors.distribution(),
            prediction_error_pixels: self
                .prediction_errors
                .iter_mut()
                .map(Series::distribution)
                .collect(),
            virtual_aim_error_pixels: self.aim_errors.distribution(),
        })
    }
}
fn algorithm_fingerprint() -> String {
    fingerprint(
        concat!(
            include_str!("../../vision/src/lib.rs"),
            include_str!("../../tracking/src/lib.rs"),
            include_str!("../../aiming/src/calibration.rs"),
            include_str!("../../aiming/src/lib.rs"),
            include_str!("../../camera/src/synthetic.rs"),
            include_str!("../../core/src/domain.rs"),
            include_str!("pipeline.rs"),
            include_str!("../../telemetry/src/metrics.rs"),
            include_str!("../../telemetry/src/windows.rs")
        )
        .as_bytes(),
    )
}
