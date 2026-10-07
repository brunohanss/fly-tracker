#![forbid(unsafe_code)]
pub mod calibration;
pub mod delay;
pub mod fit;
use fly_core::{FrameId, FrameTimestamp, LockoutReason, NormalizedAim, PixelPosition, TargetId};
use safety::{EvidenceScope, SafetyAuthority};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AimingError {
    #[error("Calibration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid calibration: {0}")]
    Calibration(&'static str),
    #[error("Coordinate failure: {0}")]
    Domain(#[from] fly_core::DomainError),
    #[error("Calibration JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Physical output unavailable: {0}")]
    DeviceUnavailable(&'static str),
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AimRequest {
    pub frame: FrameId,
    pub target: TargetId,
    pub predicted_pixel: PixelPosition,
    pub aim: NormalizedAim,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AimRecord {
    pub request: AimRequest,
    pub issued: bool,
    pub suppressed_by: Option<LockoutReason>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AimStatus {
    NoTarget,
    NoCalibration,
    RejectedCalibration,
    IssuedVirtual,
    WaitingVirtual { target: TargetId, remaining_us: u64 },
    Suppressed(LockoutReason),
}
pub trait AimingDevice: Send {
    fn aim(
        &mut self,
        authority: &mut SafetyAuthority,
        now: FrameTimestamp,
        request: AimRequest,
    ) -> Result<AimRecord, AimingError>;
    fn stop(&mut self);
}
pub struct VirtualAimer {
    scope: EvidenceScope,
    last: Option<AimRecord>,
    issued_count: u64,
}
impl VirtualAimer {
    pub fn new(scope: EvidenceScope) -> Self {
        Self {
            scope,
            last: None,
            issued_count: 0,
        }
    }
    pub fn last(&self) -> Option<AimRecord> {
        self.last
    }
    pub fn issued_count(&self) -> u64 {
        self.issued_count
    }
    /// Overlay only issued output. A suppressed request must not appear as a command.
    pub fn render_overlay(
        &self,
        pixels: &mut [u8],
        size: fly_core::FrameSize,
    ) -> Result<(), AimingError> {
        if pixels.len() != size.pixels() {
            return Err(AimingError::Calibration("overlay dimensions mismatch"));
        }
        if let Some(record) = self.last.filter(|record| record.issued) {
            let point = record.request.predicted_pixel;
            if !size.contains(point) {
                return Err(AimingError::Calibration("overlay point outside frame"));
            }
            let x = point.x().round() as usize;
            let y = point.y().round() as usize;
            for delta in -3_i32..=3 {
                for (column, row) in [(x as i32 + delta, y as i32), (x as i32, y as i32 + delta)] {
                    if column >= 0
                        && row >= 0
                        && column < size.width() as i32
                        && row < size.height() as i32
                    {
                        pixels[row as usize * size.width() as usize + column as usize] = 255;
                    }
                }
            }
        }
        Ok(())
    }
}
impl AimingDevice for VirtualAimer {
    fn aim(
        &mut self,
        authority: &mut SafetyAuthority,
        now: FrameTimestamp,
        request: AimRequest,
    ) -> Result<AimRecord, AimingError> {
        let suppressed_by = authority
            .execute(now, request.frame, self.scope, || {
                self.issued_count = self.issued_count.saturating_add(1);
            })
            .err()
            .map(|denied| denied.0);
        let record = AimRecord {
            request,
            issued: suppressed_by.is_none(),
            suppressed_by,
        };
        self.last = Some(record);
        Ok(record)
    }
    fn stop(&mut self) {
        self.last = None;
    }
}
