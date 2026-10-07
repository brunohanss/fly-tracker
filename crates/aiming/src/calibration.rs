use crate::AimingError;
use fly_core::{FrameSize, NormalizedAim, PixelPosition};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    pub version: u32,
    pub camera_size: FrameSize,
    pub created_utc: String,
    pub matrix: [[f64; 3]; 3],
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CalibrationPoint {
    pub pixel: PixelPosition,
    pub aim: NormalizedAim,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Residuals {
    pub rms: f64,
    pub maximum: f64,
    pub points: usize,
}
impl Calibration {
    /// Explicit simulation calibration, never a measured physical calibration.
    pub fn virtual_plane(size: FrameSize) -> Result<Self, AimingError> {
        if size.width() < 2 || size.height() < 2 {
            return Err(AimingError::Calibration("plane dimensions must be >=2"));
        }
        Ok(Self {
            version: 1,
            camera_size: size,
            created_utc: "simulation".into(),
            matrix: [
                [2.0 / f64::from(size.width() - 1), 0.0, -1.0],
                [0.0, 2.0 / f64::from(size.height() - 1), -1.0],
                [0.0, 0.0, 1.0],
            ],
        })
    }
    pub fn parse(json: &str) -> Result<Self, AimingError> {
        let calibration: Self = serde_json::from_str(json)?;
        calibration.validate()?;
        Ok(calibration)
    }
    pub fn validate(&self) -> Result<(), AimingError> {
        if self.version != 1
            || self.created_utc.is_empty()
            || self.created_utc.len() > 128
            || self
                .matrix
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
        {
            return Err(AimingError::Calibration("invalid schema or coefficients"));
        }
        inverse(self.matrix)?;
        Ok(())
    }
    pub fn map(&self, point: PixelPosition) -> Result<NormalizedAim, AimingError> {
        self.validate()?;
        if !self.camera_size.contains(point) {
            return Err(AimingError::Calibration("point outside calibrated frame"));
        }
        let [x, y] = transform(self.matrix, point.x(), point.y())?;
        if !(-1.0 - 1e-9..=1.0 + 1e-9).contains(&x) || !(-1.0 - 1e-9..=1.0 + 1e-9).contains(&y) {
            return Err(AimingError::Calibration(
                "mapped point outside normalized region",
            ));
        }
        Ok(NormalizedAim::new(x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0))?)
    }
    pub fn unmap(&self, aim: NormalizedAim) -> Result<PixelPosition, AimingError> {
        self.validate()?;
        let [x, y] = transform(inverse(self.matrix)?, aim.x(), aim.y())?;
        let point = PixelPosition::new(x.max(-1e-9).max(0.0), y.max(-1e-9).max(0.0))?;
        if x < -1e-9 || y < -1e-9 || !self.camera_size.contains(point) {
            return Err(AimingError::Calibration("inverse outside frame"));
        }
        Ok(point)
    }
    pub fn residuals(&self, points: &[CalibrationPoint]) -> Result<Residuals, AimingError> {
        if points.is_empty() || points.len() > 4096 {
            return Err(AimingError::Calibration("require 1..=4096 points"));
        }
        let mut squared = 0.0;
        let mut maximum: f64 = 0.0;
        for point in points {
            let mapped = self.map(point.pixel)?;
            let distance = (mapped.x() - point.aim.x()).hypot(mapped.y() - point.aim.y());
            squared += distance * distance;
            maximum = maximum.max(distance);
        }
        Ok(Residuals {
            rms: (squared / points.len() as f64).sqrt(),
            maximum,
            points: points.len(),
        })
    }
    /// Least-squares affine fit. Perspective fitting and physical point acquisition remain separate.
    pub fn fit_affine(
        size: FrameSize,
        points: &[CalibrationPoint],
        created_utc: String,
    ) -> Result<Self, AimingError> {
        if !(3..=4096).contains(&points.len()) {
            return Err(AimingError::Calibration("require 3..=4096 points"));
        }
        let mut normal = [[0.0; 3]; 3];
        let mut bx = [0.0; 3];
        let mut by = [0.0; 3];
        for point in points {
            if !size.contains(point.pixel) {
                return Err(AimingError::Calibration("fit point outside frame"));
            }
            let row = [point.pixel.x(), point.pixel.y(), 1.0];
            for i in 0..3 {
                bx[i] += row[i] * point.aim.x();
                by[i] += row[i] * point.aim.y();
                for j in 0..3 {
                    normal[i][j] += row[i] * row[j];
                }
            }
        }
        let inv = inverse(normal)?;
        let apply = |b: [f64; 3]| inv.map(|row| row.iter().zip(b).map(|(a, b)| a * b).sum::<f64>());
        let calibration = Self {
            version: 1,
            camera_size: size,
            created_utc,
            matrix: [apply(bx), apply(by), [0.0, 0.0, 1.0]],
        };
        calibration.validate()?;
        Ok(calibration)
    }
}
fn transform(m: [[f64; 3]; 3], x: f64, y: f64) -> Result<[f64; 2], AimingError> {
    let w = m[2][0] * x + m[2][1] * y + m[2][2];
    if !w.is_finite() || w.abs() < 1e-12 {
        return Err(AimingError::Calibration("singular mapping"));
    }
    let result = [
        (m[0][0] * x + m[0][1] * y + m[0][2]) / w,
        (m[1][0] * x + m[1][1] * y + m[1][2]) / w,
    ];
    if result.iter().any(|value| !value.is_finite()) {
        return Err(AimingError::Calibration("nonfinite mapping"));
    }
    Ok(result)
}
fn inverse(m: [[f64; 3]; 3]) -> Result<[[f64; 3]; 3], AimingError> {
    let mut a = [[0.0; 6]; 3];
    for i in 0..3 {
        a[i][..3].copy_from_slice(&m[i]);
        a[i][i + 3] = 1.0;
    }
    for column in 0..3 {
        let mut pivot = column;
        for row in column + 1..3 {
            if a[row][column].abs() > a[pivot][column].abs() {
                pivot = row;
            }
        }
        if !a[pivot][column].is_finite() || a[pivot][column].abs() < 1e-12 {
            return Err(AimingError::Calibration("degenerate fit or matrix"));
        }
        a.swap(column, pivot);
        let divisor = a[column][column];
        for value in &mut a[column] {
            *value /= divisor;
        }
        for row in 0..3 {
            if row != column {
                let factor = a[row][column];
                let pivot_values = a[column];
                for (value, pivot_value) in a[row].iter_mut().zip(pivot_values) {
                    *value -= factor * pivot_value;
                }
            }
        }
    }
    let result = a.map(|row| [row[3], row[4], row[5]]);
    if result.iter().flatten().any(|v| !v.is_finite()) {
        return Err(AimingError::Calibration("nonfinite inverse"));
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_grid_and_versioned_fit() -> Result<(), AimingError> {
        let size = FrameSize::new(101, 81)?;
        let calibration = Calibration::virtual_plane(size)?;
        let mut points = Vec::new();
        for x in (0..=100).step_by(10) {
            for y in (0..=80).step_by(10) {
                let pixel = PixelPosition::new(f64::from(x), f64::from(y))?;
                let aim = calibration.map(pixel)?;
                let restored = calibration.unmap(aim)?;
                assert!((restored.x() - pixel.x()).abs() < 1e-8);
                assert!((restored.y() - pixel.y()).abs() < 1e-8);
                points.push(CalibrationPoint { pixel, aim });
            }
        }
        let fit = Calibration::fit_affine(size, &points, "test".into())?;
        assert!(fit.residuals(&points)?.maximum < 1e-8);
        Calibration::parse(&serde_json::to_string(&fit)?)?;
        assert!(Calibration::fit_affine(size, &[points[0]; 3], "test".into()).is_err());
        let mut invalid = calibration;
        invalid.version = 2;
        assert!(invalid.validate().is_err());
        Ok(())
    }
}
