//! Offline commissioning. No physical transport or live authorization exists here.
use crate::{
    AimRequest, AimingDevice, AimingError,
    calibration::{CalibrationPoint, Residuals},
    fit::{CalibrationSamples, FitReport},
    pan_tilt::{MotionState, PanTiltSnapshot, VirtualPanTilt},
};
use fly_core::{
    FrameId, FrameSize, FrameTimestamp, PixelPosition, TargetId,
    servo::{PanTiltConfig, PanTiltPosition, ServoAngle},
};
use safety::{EvidenceScope, SafetyAuthority};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Provenance {
    SimulationOnly,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Axis {
    Pan,
    Tilt,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServoSample {
    pub frame: FrameId,
    pub timestamp: FrameTimestamp,
    pub pixel: PixelPosition,
    pub position: PanTiltPosition,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServoSamples {
    pub version: u32,
    pub provenance: Provenance,
    pub assembly_id: String,
    pub camera_size: FrameSize,
    pub profile: PanTiltConfig,
    pub fit_points: Vec<ServoSample>,
    pub validation_points: Vec<ServoSample>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServoFitReport {
    pub samples: ServoSamples,
    pub fit: FitReport,
    pub validation_pixels: Residuals,
    pub max_validation_pixels: f64,
}
impl ServoSamples {
    fn validate(&self) -> Result<(), AimingError> {
        self.profile.validate()?;
        if self.version != 1
            || self.assembly_id.is_empty()
            || self.assembly_id.len() > 128
            || !(4..=256).contains(&self.fit_points.len())
            || !(1..=256).contains(&self.validation_points.len())
        {
            return Err(AimingError::Calibration(
                "invalid servo sample schema or count",
            ));
        }
        let mut seen: Vec<ServoSample> = Vec::with_capacity(512);
        for point in self.fit_points.iter().chain(&self.validation_points) {
            self.profile.pulses(point.position)?;
            if !self.camera_size.contains(point.pixel)
                || seen.iter().any(|previous| {
                    previous.pixel == point.pixel
                        || previous.position == point.position
                        || previous.frame == point.frame
                })
            {
                return Err(AimingError::Calibration(
                    "duplicate observation or point outside frame",
                ));
            }
            seen.push(*point);
        }
        Ok(())
    }
    pub fn fit(&self, created_utc: String, max_pixels: f64) -> Result<ServoFitReport, AimingError> {
        self.validate()?;
        if !max_pixels.is_finite() || !(0.0..=100.0).contains(&max_pixels) {
            return Err(AimingError::Calibration(
                "pixel error limit must be 0..=100",
            ));
        }
        let convert = |points: &[ServoSample]| -> Result<Vec<CalibrationPoint>, AimingError> {
            points
                .iter()
                .map(|p| {
                    Ok(CalibrationPoint {
                        pixel: p.pixel,
                        aim: self.profile.normalized(p.position)?,
                    })
                })
                .collect()
        };
        let fit = CalibrationSamples {
            version: 1,
            camera_size: self.camera_size,
            points: convert(&self.fit_points)?,
            validation_points: convert(&self.validation_points)?,
        }
        .fit(created_utc, 1.0)?;
        let mut squared = 0.0;
        let mut maximum: f64 = 0.0;
        for point in &self.validation_points {
            let predicted = fit
                .calibration
                .unmap(self.profile.normalized(point.position)?)?;
            let distance = (predicted.x() - point.pixel.x()).hypot(predicted.y() - point.pixel.y());
            squared += distance * distance;
            maximum = maximum.max(distance);
        }
        if maximum > max_pixels {
            return Err(AimingError::Calibration(
                "held-out pixel error exceeds configured limit",
            ));
        }
        Ok(ServoFitReport {
            samples: self.clone(),
            fit,
            validation_pixels: Residuals {
                rms: (squared / self.validation_points.len() as f64).sqrt(),
                maximum,
                points: self.validation_points.len(),
            },
            max_validation_pixels: max_pixels,
        })
    }
    pub fn load(path: &Path) -> Result<Self, AimingError> {
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > 1024 * 1024 {
            return Err(AimingError::Calibration("servo samples exceed 1MiB"));
        }
        let samples: Self = serde_json::from_reader(file)?;
        samples.validate()?;
        Ok(samples)
    }
}
impl ServoFitReport {
    /// Recompute instead of accepting saved coefficients or a saved pass flag.
    pub fn load(path: &Path) -> Result<Self, AimingError> {
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > 1024 * 1024 {
            return Err(AimingError::Calibration("servo report exceeds 1MiB"));
        }
        let saved: Self = serde_json::from_reader(file)?;
        saved.samples.fit(
            saved.fit.calibration.created_utc,
            saved.max_validation_pixels,
        )
    }
    pub fn save_new(&self, path: &Path) -> Result<(), AimingError> {
        let checked = self.samples.fit(
            self.fit.calibration.created_utc.clone(),
            self.max_validation_pixels,
        )?;
        let bytes = serde_json::to_vec_pretty(&checked)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    }
    pub fn map(
        &self,
        pixel: PixelPosition,
        profile: PanTiltConfig,
        size: FrameSize,
        assembly_id: &str,
    ) -> Result<PanTiltPosition, AimingError> {
        if profile != self.samples.profile
            || size != self.samples.camera_size
            || assembly_id != self.samples.assembly_id
        {
            return Err(AimingError::Calibration(
                "servo calibration binding mismatch",
            ));
        }
        // Never extrapolate beyond the convex hull of the observed fit points.
        let p = &self.samples.fit_points;
        let cross = |a: PixelPosition, b: PixelPosition, c: PixelPosition| {
            (b.x() - a.x()) * (c.y() - a.y()) - (b.y() - a.y()) * (c.x() - a.x())
        };
        let mut inside = false;
        'triangles: for i in 0..p.len() {
            for j in i + 1..p.len() {
                for k in j + 1..p.len() {
                    let area = cross(p[i].pixel, p[j].pixel, p[k].pixel);
                    if area.abs() < 1e-9 {
                        continue;
                    }
                    let signs = [
                        cross(p[i].pixel, p[j].pixel, pixel),
                        cross(p[j].pixel, p[k].pixel, pixel),
                        cross(p[k].pixel, p[i].pixel, pixel),
                    ];
                    if signs.iter().all(|v| *v >= -1e-9) || signs.iter().all(|v| *v <= 1e-9) {
                        inside = true;
                        break 'triangles;
                    }
                }
            }
        }
        if !inside {
            return Err(AimingError::Calibration("point outside sampled region"));
        }
        Ok(profile.map(self.fit.calibration.map(pixel)?)?)
    }
}

/// One owner, no movement queue. Each jog changes one axis by at most five degrees.
pub struct CommissioningSession {
    profile: PanTiltConfig,
    simulator: VirtualPanTilt,
}
impl CommissioningSession {
    pub fn new(profile: PanTiltConfig) -> Result<Self, AimingError> {
        Ok(Self {
            profile,
            simulator: VirtualPanTilt::new(profile, EvidenceScope::ReplayFixture)?,
        })
    }
    pub fn snapshot(&self) -> PanTiltSnapshot {
        self.simulator.snapshot()
    }
    pub fn stop(&mut self) {
        self.simulator.stop();
    }
    pub fn advance(
        &mut self,
        gate: &mut SafetyAuthority,
        now: FrameTimestamp,
        frame: FrameId,
    ) -> Result<(), AimingError> {
        self.simulator.advance(gate, now, frame)
    }
    pub fn jog(
        &mut self,
        gate: &mut SafetyAuthority,
        now: FrameTimestamp,
        frame: FrameId,
        axis: Axis,
        delta: f64,
    ) -> Result<bool, AimingError> {
        self.advance(gate, now, frame)?;
        if !delta.is_finite()
            || delta == 0.0
            || delta.abs() > 5.0
            || matches!(
                self.snapshot().state,
                MotionState::Moving | MotionState::Settling
            )
        {
            return Err(AimingError::Calibration(
                "jog requires an idle axis and a nonzero step up to five degrees",
            ));
        }
        let mut position = self.snapshot().simulated;
        let angle = match axis {
            Axis::Pan => &mut position.pan,
            Axis::Tilt => &mut position.tilt,
        };
        *angle = ServoAngle::new(angle.degrees() + delta)?;
        let aim = self.profile.normalized(position)?;
        Ok(self
            .simulator
            .aim(
                gate,
                now,
                AimRequest {
                    frame,
                    target: TargetId(1),
                    predicted_pixel: PixelPosition::new(0.0, 0.0)?,
                    aim,
                },
            )?
            .issued)
    }
    pub fn sample(
        &mut self,
        gate: &mut SafetyAuthority,
        now: FrameTimestamp,
        frame: FrameId,
        pixel: PixelPosition,
        size: FrameSize,
    ) -> Result<ServoSample, AimingError> {
        self.advance(gate, now, frame)?;
        if self.snapshot().state != MotionState::Settled || !size.contains(pixel) {
            return Err(AimingError::Calibration(
                "sample requires settled motion and an in-frame observation",
            ));
        }
        let position = self.snapshot().simulated;
        gate.execute(now, frame, EvidenceScope::ReplayFixture, || ServoSample {
            frame,
            timestamp: now,
            pixel,
            position,
        })
        .map_err(|_| AimingError::Calibration("safety denied sample"))
    }
}
