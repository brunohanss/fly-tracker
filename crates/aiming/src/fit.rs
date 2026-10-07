use crate::{
    AimingError,
    calibration::{Calibration, CalibrationPoint, Residuals},
};
use fly_core::FrameSize;
use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, io::Write, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationSamples {
    pub version: u32,
    pub camera_size: FrameSize,
    pub points: Vec<CalibrationPoint>,
    pub validation_points: Vec<CalibrationPoint>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitReport {
    pub calibration: Calibration,
    pub fit_residuals: Residuals,
    pub validation_residuals: Residuals,
}
impl CalibrationSamples {
    pub fn load(path: &Path) -> Result<Self, AimingError> {
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > 1024 * 1024 {
            return Err(AimingError::Calibration("sample file exceeds 1MiB"));
        }
        Ok(serde_json::from_reader(file)?)
    }
    pub fn fit(
        &self,
        created_utc: String,
        max_validation_error: f64,
    ) -> Result<FitReport, AimingError> {
        if self.version != 1
            || !(0.0..=1.0).contains(&max_validation_error)
            || !max_validation_error.is_finite()
            || self.validation_points.is_empty()
            || self.validation_points.len() > 4096
        {
            return Err(AimingError::Calibration(
                "invalid samples or residual limit",
            ));
        }
        let calibration = fit_projective(self.camera_size, &self.points, created_utc)?;
        let fit_residuals = calibration.residuals(&self.points)?;
        let validation_residuals = calibration.residuals(&self.validation_points)?;
        if validation_residuals.maximum > max_validation_error {
            return Err(AimingError::Calibration(
                "held-out error exceeds configured normalized limit",
            ));
        }
        Ok(FitReport {
            calibration,
            fit_residuals,
            validation_residuals,
        })
    }
}
impl Calibration {
    pub fn save_new(&self, path: &Path) -> Result<(), AimingError> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self, AimingError> {
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > 65536 {
            return Err(AimingError::Calibration("calibration exceeds 64KiB"));
        }
        let calibration: Self = serde_json::from_reader(file)?;
        calibration.validate()?;
        Ok(calibration)
    }
}
/// Fit a homography on the controlled plane. This acquires no physical points.
pub fn fit_projective(
    size: FrameSize,
    points: &[CalibrationPoint],
    created_utc: String,
) -> Result<Calibration, AimingError> {
    if size.width() < 2 || size.height() < 2 || !(4..=4096).contains(&points.len()) {
        return Err(AimingError::Calibration(
            "require >=2 dimensions and 4..=4096 fit points",
        ));
    }
    let sx = 2.0 / f64::from(size.width() - 1);
    let sy = 2.0 / f64::from(size.height() - 1);
    let mut normal = [[0.0; 8]; 8];
    let mut b = [0.0; 8];
    for point in points {
        if !size.contains(point.pixel) {
            return Err(AimingError::Calibration("fit point outside frame"));
        }
        let u = point.pixel.x() * sx - 1.0;
        let v = point.pixel.y() * sy - 1.0;
        let x = point.aim.x();
        let y = point.aim.y();
        for (row, value) in [
            ([u, v, 1.0, 0.0, 0.0, 0.0, -x * u, -x * v], x),
            ([0.0, 0.0, 0.0, u, v, 1.0, -y * u, -y * v], y),
        ] {
            for (i, a) in row.iter().enumerate() {
                b[i] += a * value;
                for (j, c) in row.iter().enumerate() {
                    normal[i][j] += a * c;
                }
            }
        }
    }
    let h = solve(normal, b)?;
    let calibration = Calibration {
        version: 1,
        camera_size: size,
        created_utc,
        matrix: [
            [h[0] * sx, h[1] * sy, h[2] - h[0] - h[1]],
            [h[3] * sx, h[4] * sy, h[5] - h[3] - h[4]],
            [h[6] * sx, h[7] * sy, 1.0 - h[6] - h[7]],
        ],
    };
    calibration.validate()?;
    Ok(calibration)
}
fn solve(mut matrix: [[f64; 8]; 8], mut b: [f64; 8]) -> Result<[f64; 8], AimingError> {
    for column in 0..8 {
        let mut pivot = column;
        for row in column + 1..8 {
            if matrix[row][column].abs() > matrix[pivot][column].abs() {
                pivot = row;
            }
        }
        if !matrix[pivot][column].is_finite() || matrix[pivot][column].abs() < 1e-12 {
            return Err(AimingError::Calibration("degenerate projective fit"));
        }
        matrix.swap(column, pivot);
        b.swap(column, pivot);
        let divisor = matrix[column][column];
        for value in &mut matrix[column] {
            *value /= divisor;
        }
        b[column] /= divisor;
        let pivot_row = matrix[column];
        let pivot_b = b[column];
        for row in 0..8 {
            if row != column {
                let factor = matrix[row][column];
                for (value, pivot_value) in matrix[row].iter_mut().zip(pivot_row) {
                    *value -= factor * pivot_value;
                }
                b[row] -= factor * pivot_b;
            }
        }
    }
    if b.iter().any(|value| !value.is_finite()) {
        return Err(AimingError::Calibration("nonfinite projective fit"));
    }
    Ok(b)
}
#[cfg(test)]
mod tests {
    use super::*;
    use fly_core::PixelPosition;
    #[test]
    fn projective_fit_matches_unseen_points_and_rejects_degeneracy() -> Result<(), AimingError> {
        let size = FrameSize::new(101, 81)?;
        let expected = Calibration {
            version: 1,
            camera_size: size,
            created_utc: "test".into(),
            matrix: [
                [0.008, 0.001, -0.8],
                [0.0002, 0.015, -0.8],
                [0.0005, -0.0004, 1.0],
            ],
        };
        let mut points = Vec::new();
        for (x, y) in [
            (0.0, 0.0),
            (100.0, 0.0),
            (0.0, 80.0),
            (100.0, 80.0),
            (20.0, 20.0),
            (75.0, 50.0),
        ] {
            let pixel = PixelPosition::new(x, y)?;
            points.push(CalibrationPoint {
                pixel,
                aim: expected.map(pixel)?,
            });
        }
        let fit = fit_projective(size, &points, "test".into())?;
        for x in (0..=100).step_by(5) {
            for y in (0..=80).step_by(5) {
                let pixel = PixelPosition::new(f64::from(x), f64::from(y))?;
                let a = fit.map(pixel)?;
                let b = expected.map(pixel)?;
                assert!((a.x() - b.x()).hypot(a.y() - b.y()) < 1e-9);
            }
        }
        assert!(fit_projective(size, &[points[0]; 4], "test".into()).is_err());
        Ok(())
    }
}
