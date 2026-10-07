#![forbid(unsafe_code)]
use fly_core::{
    Detection, DomainError, FrameSize, FrameTimestamp, PixelPosition, PixelVelocity, TargetId,
    Track, TrackLifecycle,
};
use std::collections::VecDeque;
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct TrackerConfig {
    pub max_tracks: usize,
    pub max_detections: usize,
    pub association_radius: f64,
    pub confirmation_hits: u32,
    pub remove_after_misses: u32,
    pub velocity_time_constant_s: f64,
}
impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            max_tracks: 32,
            max_detections: 128,
            association_radius: 20.0,
            confirmation_hits: 3,
            remove_after_misses: 8,
            velocity_time_constant_s: 0.03,
        }
    }
}
#[derive(Debug, Error)]
pub enum TrackingError {
    #[error("Invalid tracker configuration or detections")]
    Invalid,
    #[error("Frame timestamp must strictly increase")]
    Timestamp,
    #[error("Numerical failure: {0}")]
    Domain(#[from] DomainError),
    #[error("Target ID exhausted")]
    IdExhausted,
}
pub struct Tracker {
    size: FrameSize,
    config: TrackerConfig,
    tracks: Vec<Track>,
    recent: VecDeque<Track>,
    used: Vec<bool>,
    next_id: u64,
    last_timestamp: Option<FrameTimestamp>,
    dropped: u64,
}
impl Tracker {
    pub fn new(size: FrameSize, config: TrackerConfig) -> Result<Self, TrackingError> {
        if !(1..=65536).contains(&config.max_tracks)
            || !(1..=65536).contains(&config.max_detections)
            || !config.association_radius.is_finite()
            || config.association_radius <= 0.0
            || config.association_radius > 1_000_000.0
            || config.confirmation_hits == 0
            || config.remove_after_misses == 0
            || !config.velocity_time_constant_s.is_finite()
            || config.velocity_time_constant_s <= 0.0
            || !(1e-6..=10.0).contains(&config.velocity_time_constant_s)
        {
            return Err(TrackingError::Invalid);
        }
        Ok(Self {
            size,
            config,
            tracks: Vec::with_capacity(config.max_tracks),
            recent: VecDeque::with_capacity(config.max_tracks),
            used: vec![false; config.max_detections],
            next_id: 1,
            last_timestamp: None,
            dropped: 0,
        })
    }
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }
    pub fn recent(&self) -> &VecDeque<Track> {
        &self.recent
    }
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
    pub fn reset(&mut self) {
        self.tracks.clear();
        self.recent.clear();
        self.next_id = 1;
        self.last_timestamp = None;
        self.dropped = 0;
    }
    pub fn update(
        &mut self,
        detections: &[Detection],
        timestamp: FrameTimestamp,
    ) -> Result<&[Track], TrackingError> {
        if self.last_timestamp.is_some_and(|last| timestamp <= last) {
            return Err(TrackingError::Timestamp);
        }
        if detections.len() > self.config.max_detections
            || detections
                .iter()
                .any(|d| !self.size.contains(d.position) || d.area == 0)
        {
            return Err(TrackingError::Invalid);
        }
        self.last_timestamp = Some(timestamp);
        self.used.fill(false);
        // Stable track order and lowest-index ties give deterministic greedy association.
        // Crossings can exchange IDs; the regression report exposes this limit.
        for track in &mut self.tracks {
            let predicted =
                predict(track, timestamp, self.size, 1_000_000)?.unwrap_or(track.position);
            let mut best = None;
            let mut best_distance = self.config.association_radius.powi(2);
            for (index, detection) in detections.iter().enumerate() {
                if self.used[index] {
                    continue;
                }
                let distance = (predicted.x() - detection.position.x()).powi(2)
                    + (predicted.y() - detection.position.y()).powi(2);
                if distance < best_distance {
                    best_distance = distance;
                    best = Some(index);
                }
            }
            if let Some(index) = best {
                self.used[index] = true;
                let detection = detections[index];
                let dt = timestamp.elapsed_since(track.updated_at)?.as_secs_f64();
                let blend = if track.hits == 1 {
                    1.0
                } else {
                    1.0 - (-dt / self.config.velocity_time_constant_s).exp()
                };
                let vx = (detection.position.x() - track.position.x()) / dt;
                let vy = (detection.position.y() - track.position.y()) / dt;
                track.velocity = PixelVelocity::new(
                    track.velocity.x() + blend * (vx - track.velocity.x()),
                    track.velocity.y() + blend * (vy - track.velocity.y()),
                )?;
                track.position = detection.position;
                track.confidence = detection.confidence;
                track.updated_at = timestamp;
                track.hits = track.hits.saturating_add(1);
                track.lost_count = 0;
                track.lifecycle = if track.hits >= self.config.confirmation_hits {
                    TrackLifecycle::Confirmed
                } else {
                    TrackLifecycle::Candidate
                };
            } else {
                track.lost_count = track.lost_count.saturating_add(1);
                track.lifecycle = if track.lost_count >= self.config.remove_after_misses {
                    TrackLifecycle::Removed
                } else {
                    TrackLifecycle::TemporarilyLost
                };
            }
        }
        for track in self
            .tracks
            .iter()
            .filter(|track| track.lifecycle == TrackLifecycle::Removed)
        {
            if self.recent.len() == self.config.max_tracks {
                self.recent.pop_front();
            }
            self.recent.push_back(track.clone());
        }
        self.tracks
            .retain(|track| track.lifecycle != TrackLifecycle::Removed);
        for (index, detection) in detections.iter().enumerate() {
            if self.used[index] {
                continue;
            }
            if self.tracks.len() == self.config.max_tracks {
                self.dropped = self.dropped.saturating_add(1);
                continue;
            }
            let id = TargetId(self.next_id);
            self.next_id = self
                .next_id
                .checked_add(1)
                .ok_or(TrackingError::IdExhausted)?;
            self.tracks.push(Track {
                id,
                position: detection.position,
                velocity: PixelVelocity::new(0.0, 0.0)?,
                confidence: detection.confidence,
                lifecycle: if self.config.confirmation_hits == 1 {
                    TrackLifecycle::Confirmed
                } else {
                    TrackLifecycle::Candidate
                },
                first_seen: timestamp,
                updated_at: timestamp,
                hits: 1,
                lost_count: 0,
            });
        }
        Ok(&self.tracks)
    }
}
/// Constant-velocity prediction. None means outside the frame or allowed horizon.
pub fn predict(
    track: &Track,
    at: FrameTimestamp,
    size: FrameSize,
    max_horizon_us: u64,
) -> Result<Option<PixelPosition>, DomainError> {
    let duration = at.elapsed_since(track.updated_at)?;
    if duration.as_micros() > u128::from(max_horizon_us)
        || track.lifecycle == TrackLifecycle::Removed
    {
        return Ok(None);
    }
    let dt = duration.as_secs_f64();
    let x = track.position.x() + track.velocity.x() * dt;
    let y = track.position.y() + track.velocity.y() * dt;
    if !x.is_finite() || !y.is_finite() {
        return Err(DomainError::Range("finite prediction"));
    }
    if x < 0.0 || y < 0.0 || x >= f64::from(size.width()) || y >= f64::from(size.height()) {
        return Ok(None);
    }
    Ok(Some(PixelPosition::new(x, y)?))
}
#[cfg(test)]
mod tests {
    use super::*;
    use fly_core::Confidence;
    fn detection(x: f64, y: f64) -> Result<Detection, DomainError> {
        Ok(Detection {
            position: PixelPosition::new(x, y)?,
            area: 9,
            confidence: Confidence::new(1.0)?,
        })
    }
    #[test]
    fn irregular_timestamps_preserve_velocity_and_lifecycle()
    -> Result<(), Box<dyn std::error::Error>> {
        let size = FrameSize::new(100, 100)?;
        let mut tracker = Tracker::new(size, TrackerConfig::default())?;
        for micros in [0, 10_000, 25_000] {
            tracker.update(
                &[detection(10.0 + micros as f64 / 10_000.0, 20.0)?],
                FrameTimestamp(micros),
            )?;
        }
        let track = &tracker.tracks()[0];
        assert_eq!(track.id, TargetId(1));
        assert_eq!(track.lifecycle, TrackLifecycle::Confirmed);
        assert!((track.velocity.x() - 100.0).abs() < 1e-9);
        for horizon in [0, 5000, 10000, 20000] {
            let prediction = predict(track, FrameTimestamp(25_000 + horizon), size, 20_000)?
                .ok_or("missing prediction")?;
            assert!((prediction.x() - (12.5 + horizon as f64 / 10_000.0)).abs() < 1e-9);
        }
        assert!(tracker.update(&[], FrameTimestamp(25_000)).is_err());
        tracker.update(&[], FrameTimestamp(30_000))?;
        assert_eq!(
            tracker.tracks()[0].lifecycle,
            TrackLifecycle::TemporarilyLost
        );
        tracker.update(&[detection(14.0, 20.0)?], FrameTimestamp(40_000))?;
        assert_eq!(tracker.tracks()[0].id, TargetId(1));
        assert_eq!(tracker.tracks()[0].lifecycle, TrackLifecycle::Confirmed);
        for index in 1..=8 {
            tracker.update(&[], FrameTimestamp(40_000 + index * 10_000))?;
        }
        assert!(tracker.tracks().is_empty());
        assert_eq!(tracker.recent()[0].lifecycle, TrackLifecycle::Removed);
        Ok(())
    }
    #[test]
    fn one_detection_cannot_update_two_tracks_and_storage_is_bounded()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = TrackerConfig {
            max_tracks: 2,
            ..TrackerConfig::default()
        };
        let mut tracker = Tracker::new(FrameSize::new(100, 100)?, config)?;
        tracker.update(
            &[
                detection(10.0, 10.0)?,
                detection(20.0, 20.0)?,
                detection(80.0, 80.0)?,
            ],
            FrameTimestamp(0),
        )?;
        assert_eq!(tracker.tracks().len(), 2);
        assert_eq!(tracker.dropped(), 1);
        tracker.update(&[detection(15.0, 15.0)?], FrameTimestamp(10_000))?;
        assert_eq!(
            tracker
                .tracks()
                .iter()
                .filter(|track| track.lost_count == 0)
                .count(),
            1
        );
        Ok(())
    }
}
