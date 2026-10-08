use crate::{Counters, Distribution, MetricsError};
use aiming::{AimRecord, calibration::Residuals};
use fly_core::{FrameId, FrameTimestamp, PixelPosition, SafetyState, Track};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

#[derive(Debug, Clone)]
pub struct Preview {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    pub pan_tilt: Option<aiming::pan_tilt::PanTiltSnapshot>,
    pub publication_time: std::time::Instant,
    pub system: fly_core::SystemState,
    pub aim_status: aiming::AimStatus,
    pub calibration: Option<aiming::calibration::Calibration>,
    pub time_windows: Vec<crate::windows::WindowDistribution>,
    pub identity: crate::RunIdentity,
    pub camera_size: fly_core::FrameSize,
    pub total_frames: Option<u64>,
    pub paused: bool,
    pub replay_speed: f64,
    pub detection_error_pixels: Option<Distribution>,
    pub prediction_error_pixels: Vec<Option<Distribution>>,
    pub virtual_aim_error_pixels: Option<Distribution>,
    pub frame: FrameId,
    pub timestamp: FrameTimestamp,
    pub source: String,
    pub recorded_safety: bool,
    pub safety_backend: String,
    pub safety: SafetyState,
    pub camera_available: bool,
    pub tracks: Vec<Track>,
    pub recent_tracks: Vec<Track>,
    pub predictions: Vec<(fly_core::TargetId, PixelPosition)>,
    pub aim: Option<AimRecord>,
    pub preview: Preview,
    pub foreground_preview: Preview,
    pub counters: Counters,
    pub latency: Vec<Option<Distribution>>,
    pub calibration_residuals: Option<Residuals>,
    pub calibration_version: Option<u32>,
    pub sample_capacity: usize,
    pub complete: bool,
    pub fps: Option<f64>,
    pub elapsed_seconds: f64,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReplayCommand {
    TogglePause,
    Step(i64),
    Seek(u64),
    Speed(f64),
    Stop,
}
pub struct Publisher<T> {
    sender: SyncSender<T>,
    pub dropped: u64,
}
impl<T> Publisher<T> {
    /// Full/stopped reader never blocks processing. Full drops this publication.
    pub fn publish(&mut self, value: T) -> bool {
        match self.sender.try_send(value) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped = self.dropped.saturating_add(1);
                false
            }
        }
    }
}
pub fn bounded<T>(capacity: usize) -> Result<(Publisher<T>, Receiver<T>), MetricsError> {
    if !(1..=65536).contains(&capacity) {
        return Err(MetricsError::Invalid("channel capacity must be 1..=65536"));
    }
    let (sender, receiver) = sync_channel(capacity);
    Ok((Publisher { sender, dropped: 0 }, receiver))
}
pub fn drain_latest<T>(receiver: &Receiver<T>) -> Option<T> {
    let mut latest = None;
    while let Ok(value) = receiver.try_recv() {
        latest = Some(value);
    }
    latest
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_stalled_and_disconnected_readers_do_not_block() -> Result<(), MetricsError> {
        let (mut publisher, receiver) = bounded(2)?;
        assert!(publisher.publish(1));
        assert!(publisher.publish(2));
        assert!(!publisher.publish(3));
        assert_eq!(drain_latest(&receiver), Some(2));
        assert!(publisher.publish(4));
        drop(receiver);
        assert!(!publisher.publish(5));
        assert_eq!(publisher.dropped, 2);
        Ok(())
    }
}
